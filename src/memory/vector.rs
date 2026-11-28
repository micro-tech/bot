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
//!
//! Storage (task 401, M11): the file is JSON Lines — one record per line,
//! appended on every insert, so `add_fact` never rewrites the whole file.
//! Records are `{"kind":"fact", ...}` or `{"kind":"tombstone","id":N}`.
//! The store is capped at [`MAX_FACTS`] facts (oldest evicted first, with a
//! tombstone line so the eviction survives a reload) and deduped by text
//! (adding the same text twice returns the existing id). Legacy files
//! holding a single pretty JSON array are migrated to JSONL on first load.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::memory::manager::durable_write_json;

pub const VECTOR_FACTS_FILE: &str = "vector_facts.json";

/// Maximum facts retained. Bounds memory and the facts file; the oldest
/// fact is evicted (with a tombstone record) when the cap is reached.
/// 768-float embeddings are ~3 KB each in memory, so 1000 facts is a
/// few MB — generous for a memory aid, small enough to stay fast.
pub const MAX_FACTS: usize = 1000;

/// Compact the file when tombstones reach this count — keeps the log from
/// growing forever once the store sits at the cap (every insert evicts one).
const COMPACT_TOMBSTONE_THRESHOLD: usize = 1000;

/// One line of the JSONL facts file.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum FactRecord {
    Fact(Fact),
    Tombstone { id: u32 },
}

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
    /// Text -> id, for O(1) dedupe (task 401). Kept in sync with `facts`
    /// on insert, evict, and load; a text is stored at most once.
    text_index: HashMap<String, u32>,
    embedder: Box<dyn Embedder + Send + Sync>,
    /// Persistence target; `None` = memory-only (tests).
    file: Option<PathBuf>,
}

impl VectorMemory {
    /// In-memory only, production embedder. Prefer `load_or_new` in the bot.
    pub fn new() -> Self {
        Self::with_embedder(Box::new(OllamaEmbedder::from_env()), None)
    }
}

impl Default for VectorMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl VectorMemory {
    /// In-memory with an injected embedder (tests, custom providers).
    pub fn with_embedder(embedder: Box<dyn Embedder + Send + Sync>, file: Option<PathBuf>) -> Self {
        Self {
            facts: HashMap::new(),
            next_id: 0,
            text_index: HashMap::new(),
            embedder,
            file,
        }
    }

    /// Insert one fact loaded from disk: dedupes by text (first copy wins)
    /// and advances `next_id` past it.
    fn insert_loaded(&mut self, fact: Fact) {
        if self.text_index.contains_key(&fact.text) {
            return; // pre-dedupe file held the same text twice; keep the first
        }
        self.next_id = self.next_id.max(fact.id + 1);
        self.text_index.insert(fact.text.clone(), fact.id);
        self.facts.insert(fact.id, fact);
    }

    /// Drop one fact loaded from disk (tombstone record).
    fn remove_loaded(&mut self, id: u32) {
        if let Some(fact) = self.facts.remove(&id) {
            self.text_index.remove(&fact.text);
        }
    }

    /// Load persisted facts from `path` (missing/corrupt -> empty, warn).
    /// Reads the JSONL record format (task 401); a legacy file holding a
    /// single pretty JSON array is migrated to JSONL on first load.
    pub fn load_from(path: &Path) -> Self {
        let mut mem = Self::with_embedder(
            Box::new(OllamaEmbedder::from_env()),
            Some(path.to_path_buf()),
        );
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return mem, // missing file -> empty
        };
        if content.trim().is_empty() {
            return mem;
        }
        // Legacy format: a single pretty JSON array of facts. Parse it,
        // then rewrite the file as JSONL once so later appends are clean.
        if content.trim_start().starts_with('[') {
            match serde_json::from_str::<Vec<Fact>>(&content) {
                Ok(facts) => {
                    for fact in facts {
                        mem.insert_loaded(fact);
                    }
                    mem.enforce_cap_and_compact();
                    mem.compact_file();
                }
                Err(e) => log::warn!(
                    "vector facts file {} is corrupt ({}); starting empty",
                    path.display(),
                    e
                ),
            }
            return mem;
        }
        // JSONL: one record per line.
        let mut tombstones = 0usize;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<FactRecord>(line) {
                Ok(FactRecord::Fact(f)) => mem.insert_loaded(f),
                Ok(FactRecord::Tombstone { id }) => {
                    tombstones += 1;
                    mem.remove_loaded(id);
                }
                Err(e) => log::warn!(
                    "vector facts file {} has an unreadable line ({}); skipping it",
                    path.display(),
                    e
                ),
            }
        }
        // Enforce the cap (a pre-cap file may hold more) and compact when
        // tombstones dominate, so the log does not grow forever.
        let over_cap = mem.enforce_cap_and_compact();
        if over_cap || tombstones >= COMPACT_TOMBSTONE_THRESHOLD {
            mem.compact_file();
        }
        mem
    }

    /// Evict the oldest facts until the store is at the cap. Returns true
    /// when anything was evicted (caller rewrites the file compacted).
    /// Eviction never touches the newest facts.
    fn enforce_cap_and_compact(&mut self) -> bool {
        let mut evicted = false;
        while self.facts.len() > MAX_FACTS {
            if let Some(&oldest) = self.facts.keys().min() {
                self.remove_loaded(oldest);
                evicted = true;
            } else {
                break;
            }
        }
        evicted
    }

    /// Production constructor: disk-backed at the configured path.
    pub fn load_or_new() -> Self {
        Self::load_from(&facts_file())
    }

    pub fn add_fact(&mut self, text: String) -> u32 {
        // Dedupe: the same text is already stored — return its id, no write.
        if let Some(&id) = self.text_index.get(&text) {
            return id;
        }
        // Empty embedding = "no embedding available"; search() treats it
        // honestly via the keyword path instead of faking a vector.
        let embedding = self.embedder.embed(&text).unwrap_or_default();
        // Cap: evict the oldest fact first. The eviction is a tombstone
        // line (append-only), so the newest facts are never lost and the
        // store stays bounded without a rewrite.
        if self.facts.len() >= MAX_FACTS
            && let Some(&oldest) = self.facts.keys().min()
        {
            self.remove_loaded(oldest);
            self.append_record(&FactRecord::Tombstone { id: oldest });
        }
        let id = self.next_id;
        self.next_id += 1;
        let fact = Fact {
            id,
            text: text.clone(),
            embedding,
        };
        self.facts.insert(id, fact.clone());
        self.text_index.insert(text, id);
        self.append_record(&FactRecord::Fact(fact));
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

    /// Append one JSONL record to the facts file: one line + file sync,
    /// O(1) bytes per insert. The old path rewrote the whole pretty JSON
    /// and fsynced it on every insert — O(n) per fact.
    fn append_record(&self, record: &FactRecord) {
        let Some(path) = &self.file else {
            return;
        };
        let line = match serde_json::to_string(record) {
            Ok(l) => l,
            Err(e) => {
                log::error!("failed to serialize vector fact record: {}", e);
                return;
            }
        };
        use std::io::Write;
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            Ok(mut f) => {
                if let Err(e) = writeln!(f, "{}", line).and_then(|_| f.sync_all()) {
                    log::error!("failed to append vector fact to {}: {}", path.display(), e);
                }
            }
            Err(e) => log::error!("failed to open vector facts file {}: {}", path.display(), e),
        }
    }

    /// Rewrite the facts file from scratch as JSONL (live facts only, in
    /// id order). Used for the one-time legacy migration and for compaction
    /// when tombstones dominate — NOT on the insert path.
    fn compact_file(&self) {
        let Some(path) = &self.file else {
            return;
        };
        let mut facts: Vec<&Fact> = self.facts.values().collect();
        facts.sort_by_key(|f| f.id);
        let mut bytes = Vec::new();
        for fact in facts {
            if let Ok(line) = serde_json::to_string(&FactRecord::Fact((*fact).clone())) {
                bytes.extend_from_slice(line.as_bytes());
                bytes.push(b'\n');
            }
        }
        if let Err(e) = durable_write_json(path, &bytes) {
            log::error!(
                "failed to compact vector facts to {}: {}",
                path.display(),
                e
            );
        }
    }

    #[cfg(test)]
    fn fact_count(&self) -> usize {
        self.facts.len()
    }

    #[cfg(test)]
    fn contains_text(&self, text: &str) -> bool {
        self.text_index.contains_key(text)
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

    /// Scripted embedder response: map text -> vector, or None to
    /// simulate an unreachable backend.
    type EmbedFn = Box<dyn FnMut(&str) -> Option<Vec<f32>> + Send + Sync>;

    /// Test embedder with scripted responses: map text -> vector, or None to
    /// simulate an unreachable backend.
    struct StubEmbedder {
        f: EmbedFn,
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
        let mut mem = VectorMemory::with_embedder(Box::new(two_topic_embedder()), None);
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
            let mut mem =
                VectorMemory::with_embedder(Box::new(two_topic_embedder()), Some(path.clone()));
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
    fn duplicate_fact_is_deduped() {
        // GIVEN a fact already stored:
        let mut mem = VectorMemory::with_embedder(Box::new(two_topic_embedder()), None);
        let first = mem.add_fact("the dog barked".to_string());

        // WHEN the same text is added again,
        // THEN the existing id comes back and nothing is stored twice:
        let second = mem.add_fact("the dog barked".to_string());
        assert_eq!(first, second);
        assert_eq!(mem.fact_count(), 1);

        // AND a disk-backed repeat does not append a second line:
        let path = std::env::temp_dir().join(format!(
            "helix-vector-test-{}-dedupe.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut disk =
            VectorMemory::with_embedder(Box::new(two_topic_embedder()), Some(path.clone()));
        disk.add_fact("the dog barked".to_string());
        disk.add_fact("the dog barked".to_string());
        let lines = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            lines.lines().count(),
            1,
            "duplicate must not append a second record: {:?}",
            lines
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn store_evicts_oldest_at_cap_and_stays_evicted_after_reload() {
        // GIVEN a disk-backed memory filled past the cap:
        let path =
            std::env::temp_dir().join(format!("helix-vector-test-{}-cap.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let mut mem =
                VectorMemory::with_embedder(Box::new(two_topic_embedder()), Some(path.clone()));
            for i in 0..(MAX_FACTS + 2) {
                mem.add_fact(format!("fact number {}", i));
            }
            // THEN the store holds exactly the cap, newest facts kept:
            assert_eq!(mem.fact_count(), MAX_FACTS);
            assert!(
                !mem.contains_text("fact number 0"),
                "oldest fact must be evicted"
            );
            assert!(
                !mem.contains_text("fact number 1"),
                "second-oldest fact must be evicted too"
            );
            assert!(
                mem.contains_text(&format!("fact number {}", MAX_FACTS + 1)),
                "newest fact must survive"
            );
        }
        // AND a reload keeps the cap (evicted facts stay evicted):
        let reloaded = VectorMemory::load_from(&path);
        assert_eq!(reloaded.fact_count(), MAX_FACTS);
        assert!(!reloaded.contains_text("fact number 0"));
        assert!(reloaded.contains_text(&format!("fact number {}", MAX_FACTS + 1)));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn add_fact_appends_instead_of_rewriting() {
        // GIVEN a disk-backed memory with one fact:
        let path = std::env::temp_dir().join(format!(
            "helix-vector-test-{}-append.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut mem =
            VectorMemory::with_embedder(Box::new(two_topic_embedder()), Some(path.clone()));
        mem.add_fact("the dog barked".to_string());
        let after_first = std::fs::read_to_string(&path).unwrap();

        // WHEN a second fact is added,
        // THEN the file grows by appending — the first record is untouched:
        mem.add_fact("rust ownership".to_string());
        let after_second = std::fs::read_to_string(&path).unwrap();
        assert!(
            after_second.starts_with(&after_first),
            "second insert must append, not rewrite:\nbefore: {:?}\nafter: {:?}",
            after_first,
            after_second
        );
        assert_eq!(after_second.lines().count(), 2);
        // AND every line is a valid JSONL record:
        for line in after_second.lines() {
            let rec: FactRecord = serde_json::from_str(line).expect("line must parse");
            assert!(matches!(rec, FactRecord::Fact(_)));
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn legacy_array_file_migrates_to_jsonl() {
        // GIVEN a pre-401 file (a single pretty JSON array):
        let path = std::env::temp_dir().join(format!(
            "helix-vector-test-{}-legacy.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            r#"[{"id":0,"text":"the dog barked","embedding":[1.0,0.0]},{"id":1,"text":"rust ownership","embedding":[0.0,1.0]}]"#,
        )
        .unwrap();

        // WHEN loaded,
        // THEN the facts survive and the file is now JSONL:
        let mem = VectorMemory::load_from(&path);
        assert_eq!(mem.fact_count(), 2);
        let migrated = std::fs::read_to_string(&path).unwrap();
        assert!(
            !migrated.trim_start().starts_with('['),
            "legacy array must be migrated, got: {:?}",
            migrated
        );
        assert_eq!(migrated.lines().count(), 2);

        // AND a later insert appends cleanly onto the migrated file:
        let mut mem = mem;
        mem.embedder = Box::new(two_topic_embedder());
        mem.add_fact("new fact here".to_string());
        assert_eq!(mem.fact_count(), 3);
        let reloaded = VectorMemory::load_from(&path);
        assert_eq!(reloaded.fact_count(), 3);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tombstone_lines_are_applied_on_load() {
        // GIVEN a JSONL file with two facts and a tombstone for one:
        let path = std::env::temp_dir().join(format!(
            "helix-vector-test-{}-tombstone.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            "{\"kind\":\"fact\",\"id\":0,\"text\":\"the dog barked\",\"embedding\":[1.0,0.0]}\n\
             {\"kind\":\"fact\",\"id\":1,\"text\":\"rust ownership\",\"embedding\":[0.0,1.0]}\n\
             {\"kind\":\"tombstone\",\"id\":0}\n",
        )
        .unwrap();

        // WHEN loaded, THEN only the live fact remains:
        let mut mem = VectorMemory::load_from(&path);
        assert_eq!(mem.fact_count(), 1);
        assert_eq!(mem.search("rust", 5), vec!["rust ownership".to_string()]);
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
