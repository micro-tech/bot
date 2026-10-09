//! Vector Memory for RAG — disk-backed, with real embeddings (task 204).
//!
//! Facts persist to `vector_facts.json` (next to `beliefs.json`); the path is
//! configurable via `HELIX_VECTOR_FACTS_FILE`. Embeddings come from a local
//! Ollama model (`HELIX_EMBED_URL`, default the desktop Ollama;
//! `HELIX_EMBED_MODEL`, default `nomic-embed-text`).
//!
//! If no embedding model is reachable, search degrades HONESTLY to
//! keyword/substring scoring and logs a warning — never silent dummy vectors.
//! (The old `dummy_embed` ranked by string length. It is gone.)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::memory::manager::durable_write_json;

pub const VECTOR_FACTS_FILE: &str = "vector_facts.json";

fn facts_file() -> PathBuf {
    PathBuf::from(
        std::env::var("HELIX_VECTOR_FACTS_FILE").unwrap_or_else(|_| VECTOR_FACTS_FILE.to_string()),
    )
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Fact {
    pub id: u32,
    pub text: String,
    pub embedding: Vec<f32>,
}

/// Embedding backend. A trait (not a concrete client) so tests can inject
/// fakes and production can swap providers without touching VectorMemory.
pub trait Embedder: Send + Sync {
    /// Returns `Some(embedding)` on success, `None` when the backend is
    /// unreachable — the caller must degrade honestly, never fake it.
    fn embed(&mut self, text: &str) -> Option<Vec<f32>>;
}

/// Ollama `/api/embed` backend with honest degradation.
///
/// On any failure the embedder backs off for 5 minutes (transient outages
/// shouldn't stall every call) and logs ONE warning per outage — then retries
/// automatically once the backoff expires.
pub struct OllamaEmbedder {
    base_url: String,
    model: String,
    degraded_until: Option<Instant>,
    warned_this_outage: bool,
}

impl OllamaEmbedder {
    pub fn new(base_url: String, model: String) -> Self {
        // reqwest is built with rustls-tls-manual-roots-no-provider: the
        // process must install a CryptoProvider. The binary does it in main(),
        // but the embedder must also work in tests and any other context, so
        // install idempotently here too (Err = already installed, ignored).
        let _ = rustls::crypto::ring::default_provider().install_default();
        // NOTE: no reqwest client is stored here on purpose. reqwest::blocking
        // owns a tokio Runtime internally, and dropping it on a thread that is
        // running an async runtime panics. MemoryManager lives inside Cpu
        // inside the bot's async runtime (and the #[tokio::test]s), so the HTTP
        // round-trip happens on a plain worker thread instead: the blocking
        // client is built, used, and dropped entirely off the caller's thread.
        Self {
            base_url,
            model,
            degraded_until: None,
            warned_this_outage: false,
        }
    }

    /// `HELIX_EMBED_URL` (default: desktop Ollama) and `HELIX_EMBED_MODEL`
    /// (default: `nomic-embed-text`).
    pub fn from_env() -> Self {
        let base_url = std::env::var("HELIX_EMBED_URL")
            .unwrap_or_else(|_| "http://192.168.1.149:11434".to_string());
        let model =
            std::env::var("HELIX_EMBED_MODEL").unwrap_or_else(|_| "nomic-embed-text".to_string());
        Self::new(base_url, model)
    }

    fn note_degraded(&mut self, reason: &str) {
        if !self.warned_this_outage {
            log::warn!(
                "embedding backend unreachable ({}); vector search degrading to keyword scoring for ~5 minutes",
                reason
            );
            self.warned_this_outage = true;
        }
        self.degraded_until = Some(Instant::now() + Duration::from_secs(300));
    }

    /// One embedding round-trip, executed on a plain spawned thread so the
    /// blocking client never touches the caller's async runtime (see the NOTE
    /// on `new`). Returns `None` on any failure: unreachable server, bad
    /// response, or a panicking worker.
    fn embed_on_worker(base_url: &str, model: &str, text: &str) -> Option<Vec<f32>> {
        let url = format!("{}/api/embed", base_url.trim_end_matches('/'));
        let body = serde_json::json!({ "model": model, "input": text });
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| -> Option<Vec<f32>> {
                let client = reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(10))
                    .connect_timeout(Duration::from_secs(3))
                    .build()
                    .ok()?;
                let resp = client.post(&url).json(&body).send().ok()?;
                if !resp.status().is_success() {
                    return None;
                }
                let v: serde_json::Value = resp.json().ok()?;
                v.get("embeddings")
                    .and_then(|e| e.get(0))
                    .and_then(|first| serde_json::from_value(first.clone()).ok())
            })();
            let _ = tx.send(result);
        });
        // Backstop well above the client's own 10s+3s timeouts.
        rx.recv_timeout(Duration::from_secs(30)).ok()?
    }
}

impl Embedder for OllamaEmbedder {
    fn embed(&mut self, text: &str) -> Option<Vec<f32>> {
        if let Some(until) = self.degraded_until {
            if Instant::now() < until {
                return None;
            }
            // Backoff expired: try again, and allow a fresh warning if still down.
            self.warned_this_outage = false;
        }
        let embedding = Self::embed_on_worker(&self.base_url, &self.model, text);
        if embedding.is_none() {
            self.note_degraded("request failed");
        } else {
            self.degraded_until = None;
            self.warned_this_outage = false;
        }
        embedding
    }
}

pub struct VectorMemory {
    facts: HashMap<u32, Fact>,
    next_id: u32,
    embedder: Box<dyn Embedder + Send + Sync>,
    /// Persistence target; `None` = memory-only (tests).
    file: Option<PathBuf>,
}

impl VectorMemory {
    /// In-memory only, production embedder. Prefer `load_or_new` in the bot.
    pub fn new() -> Self {
        Self::with_embedder(Box::new(OllamaEmbedder::from_env()), None)
    }

    /// In-memory with an injected embedder (tests, custom providers).
    pub fn with_embedder(embedder: Box<dyn Embedder + Send + Sync>, file: Option<PathBuf>) -> Self {
        Self {
            facts: HashMap::new(),
            next_id: 0,
            embedder,
            file,
        }
    }

    /// Load persisted facts from `path` (missing/corrupt -> empty, warn).
    pub fn load_from(path: &Path) -> Self {
        let mut mem = Self::with_embedder(
            Box::new(OllamaEmbedder::from_env()),
            Some(path.to_path_buf()),
        );
        match fs::read_to_string(path) {
            Ok(content) if !content.trim().is_empty() => {
                match serde_json::from_str::<Vec<Fact>>(&content) {
                    Ok(facts) => {
                        for fact in facts {
                            mem.next_id = mem.next_id.max(fact.id + 1);
                            mem.facts.insert(fact.id, fact);
                        }
                    }
                    Err(e) => log::warn!(
                        "vector facts file {} is corrupt ({}); starting empty",
                        path.display(),
                        e
                    ),
                }
            }
            _ => {}
        }
        mem
    }

    /// Production constructor: disk-backed at the configured path.
    pub fn load_or_new() -> Self {
        Self::load_from(&facts_file())
    }

    pub fn add_fact(&mut self, text: String) -> u32 {
        // Empty embedding = "no embedding available"; search() treats it
        // honestly via the keyword path instead of faking a vector.
        let embedding = self.embedder.embed(&text).unwrap_or_default();
        let id = self.next_id;
        self.facts.insert(
            id,
            Fact {
                id,
                text,
                embedding,
            },
        );
        self.next_id += 1;
        self.persist();
        id
    }

    /// Semantic search when the embedder is healthy; honest keyword scoring
    /// when it is not. Never ranks by string length (the old lie is gone).
    pub fn search(&mut self, query: &str, top_k: usize) -> Vec<String> {
        if self.facts.is_empty() {
            return Vec::new();
        }
        match self.embedder.embed(query) {
            Some(q) if q.iter().any(|&x| x != 0.0) => self.semantic_search(&q, top_k),
            _ => self.keyword_search(query, top_k),
        }
    }

    fn semantic_search(&self, query_emb: &[f32], top_k: usize) -> Vec<String> {
        let mut scores: Vec<(f32, u32, &String)> = self
            .facts
            .values()
            .map(|f| {
                let s = if f.embedding.is_empty() {
                    0.0
                } else {
                    cosine_sim(query_emb, &f.embedding)
                };
                (s, f.id, &f.text)
            })
            .collect();
        // Descending score, ascending id for stability.
        scores.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.1.cmp(&b.1))
        });
        scores
            .into_iter()
            .take(top_k)
            .map(|(_, _, text)| text.clone())
            .collect()
    }

    fn keyword_search(&self, query: &str, top_k: usize) -> Vec<String> {
        let terms: Vec<String> = query
            .split_whitespace()
            .map(|w| w.to_lowercase())
            .filter(|w| w.len() > 1)
            .collect();
        if terms.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(usize, u32, &String)> = self
            .facts
            .values()
            .map(|f| {
                let lower = f.text.to_lowercase();
                let score = terms.iter().filter(|t| lower.contains(t.as_str())).count();
                (score, f.id, &f.text)
            })
            .filter(|(score, _, _)| *score > 0)
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        scored
            .into_iter()
            .take(top_k)
            .map(|(_, _, text)| text.clone())
            .collect()
    }

    /// Rewrite the whole facts file durably (write temp + rename + fsync).
    /// Facts are low-frequency writes; durability beats incremental appends.
    fn persist(&self) {
        if let Some(path) = &self.file {
            let facts: Vec<&Fact> = self.facts.values().collect();
            match serde_json::to_vec_pretty(&facts) {
                Ok(bytes) => {
                    if let Err(e) = durable_write_json(path, &bytes) {
                        log::error!("failed to persist vector facts to {}: {}", path.display(), e);
                    }
                }
                Err(e) => log::error!("failed to serialize vector facts: {}", e),
            }
        }
    }

    #[cfg(test)]
    fn fact_count(&self) -> usize {
        self.facts.len()
    }
}

fn cosine_sim(a: &[f32], b: &[f32]) -> f32 {
    let dot = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum::<f32>();
    let norm_a = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test embedder with scripted responses: map text -> vector, or None to
    /// simulate an unreachable backend.
    struct StubEmbedder {
        f: Box<dyn FnMut(&str) -> Option<Vec<f32>> + Send + Sync>,
        pub calls: usize,
    }

    impl StubEmbedder {
        fn new(f: impl FnMut(&str) -> Option<Vec<f32>> + Send + Sync + 'static) -> Self {
            Self {
                f: Box::new(f),
                calls: 0,
            }
        }
    }

    impl Embedder for StubEmbedder {
        fn embed(&mut self, text: &str) -> Option<Vec<f32>> {
            self.calls += 1;
            (self.f)(text)
        }
    }

    /// Two well-separated directions: "animal" facts vs "tech" facts.
    fn two_topic_embedder() -> StubEmbedder {
        StubEmbedder::new(|text| {
            let lower = text.to_lowercase();
            if lower.contains("dog") || lower.contains("cat") || lower.contains("pet") {
                Some(vec![1.0, 0.0])
            } else if lower.contains("rust") || lower.contains("tokio") || lower.contains("code") {
                Some(vec![0.0, 1.0])
            } else {
                Some(vec![0.5, 0.5])
            }
        })
    }

    fn failing_embedder() -> StubEmbedder {
        StubEmbedder::new(|_| None)
    }

    #[test]
    fn semantically_similar_facts_rank_above_unrelated_ones() {
        // GIVEN facts from two topics with a working embedder:
        let mut mem =
            VectorMemory::with_embedder(Box::new(two_topic_embedder()), None);
        mem.add_fact("the dog barked loudly".to_string());
        mem.add_fact("rust ownership model".to_string());

        // WHEN we search for a pet-related query,
        // THEN the pet fact outranks the tech fact (impossible with dummy vectors):
        let results = mem.search("my pet cat", 2);
        assert_eq!(results.len(), 2);
        assert!(
            results[0].contains("dog"),
            "pet fact must rank first, got: {:?}",
            results
        );
    }

    #[test]
    fn unreachable_embedder_degrades_to_keyword_scoring() {
        // GIVEN a dead embedding backend:
        let mut mem = VectorMemory::with_embedder(Box::new(failing_embedder()), None);
        mem.add_fact("Rust is great".to_string());
        mem.add_fact("Tokio async runtime".to_string());

        // WHEN we search,
        // THEN keyword matches surface honestly instead of fake vectors:
        let results = mem.search("rust", 2);
        assert_eq!(results, vec!["Rust is great".to_string()]);

        // AND a query matching nothing returns nothing (no filler):
        let empty = mem.search("zebra", 2);
        assert!(empty.is_empty());
    }

    #[test]
    fn add_fact_persists_to_disk_and_reloads() {
        // GIVEN a temp file path:
        let path = std::env::temp_dir().join(format!(
            "helix-vector-test-{}-facts.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        // WHEN facts are added through a disk-backed memory,
        // THEN dropping and reloading restores identical content and search works:
        {
            let mut mem = VectorMemory::with_embedder(
                Box::new(two_topic_embedder()),
                Some(path.clone()),
            );
            mem.add_fact("the dog barked".to_string());
            mem.add_fact("rust ownership".to_string());
            assert!(path.exists(), "facts file must exist after add_fact");
        }
        let mut reloaded = VectorMemory::load_from(&path);
        // (fresh stub embedder for the reloaded instance)
        reloaded.embedder = Box::new(two_topic_embedder());
        assert_eq!(reloaded.fact_count(), 2);
        let results = reloaded.search("my pet", 2);
        assert!(results[0].contains("dog"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn corrupt_facts_file_loads_empty_without_panic() {
        // GIVEN a corrupt facts file:
        let path = std::env::temp_dir().join(format!(
            "helix-vector-test-{}-corrupt.json",
            std::process::id()
        ));
        std::fs::write(&path, "[[[ not json").unwrap();

        // WHEN we load it, THEN we get an empty memory, not a panic:
        let mem = VectorMemory::load_from(&path);
        assert_eq!(mem.fact_count(), 0);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_cosine_sim() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = cosine_sim(&a, &b);
        assert!((sim - 1.0).abs() < 1e-6);
        // Zero vectors are 0.0, not NaN:
        assert_eq!(cosine_sim(&[], &[]), 0.0);
        assert_eq!(cosine_sim(&[1.0], &[]), 0.0);
    }

    #[test]
    fn test_search_empty() {
        let mut mem = VectorMemory::with_embedder(Box::new(two_topic_embedder()), None);
        let results = mem.search("query", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn unreachable_endpoint_degrades_with_backoff() {
        // GIVEN a real OllamaEmbedder pointed at a dead address (nothing
        // listens on port 1; connection refused is fast and deterministic):
        let mut emb = OllamaEmbedder::new("http://127.0.0.1:1".to_string(), "m".to_string());

        // WHEN embedding against the unreachable endpoint,
        // THEN it returns None (and warns once per outage via log::warn!):
        assert!(emb.embed("hello world").is_none());
        assert!(
            emb.degraded_until.is_some(),
            "a failed attempt must arm the degraded backoff"
        );

        // AND a second call inside the backoff window short-circuits without
        // touching the network again:
        let start = Instant::now();
        assert!(emb.embed("hello world").is_none());
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "backoff must skip the network round-trip"
        );
    }
}
