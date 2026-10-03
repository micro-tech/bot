//! Local FastContext-style Repo Explorer subagent.
//!
//! Handles repository exploration via read-only tools (repo_glob, repo_read, repo_grep)
//! and returns compact evidence (file paths + line ranges + summaries)
//! instead of raw file contents. Designed to answer "Do I need repo context?"
//! queries efficiently.

use serde::{Deserialize, Serialize};

/// Query for the repo explorer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoQuery {
    /// Natural language description of what to find/explore.
    pub natural_language: String,
    /// Optional working directory (defaults to current project root).
    pub cwd: Option<String>,
    /// Optional hints to guide exploration (e.g. file extensions, keywords).
    #[serde(default)]
    pub hints: Vec<String>,
}

/// Single piece of evidence returned by the explorer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoEvidenceItem {
    /// Relative file path.
    pub path: String,
    /// Optional start line (1-based).
    pub line_start: Option<usize>,
    /// Optional end line (inclusive, 1-based).
    pub line_end: Option<usize>,
    /// Concise summary or reason this item is relevant.
    pub summary: String,
}

/// Collection of evidence items.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RepoEvidence {
    pub items: Vec<RepoEvidenceItem>,
}

impl RepoEvidence {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
}

/// Trait for repository explorers.
#[async_trait::async_trait]
pub trait RepoExplorer: Send + Sync {
    /// Explore the repository according to the query and return compact evidence.
    async fn explore(&self, query: RepoQuery) -> RepoEvidence;
}

/// Placeholder for a local LLM client (to be wired to Ollama or similar).
/// The explorer's system prompt must restrict it to read-only tools only.
pub struct LocalLLMClient {
    // TODO: wire to actual LLM (ollama::Llm or similar)
}

impl LocalLLMClient {
    pub async fn chat(&self, _system: &str, _user: &str) -> String {
        // Stub implementation – replace with real call.
        // For testing we return a "done" response so the explorer terminates quickly.
        r#"{"action":"done","evidence":[{"path":"Cargo.toml","line_start":1,"line_end":5,"summary":"Project manifest - likely relevant for build context"}]}"#.to_string()
    }
}

pub mod explorer_impl; // Concrete implementation with parallel execution
pub mod fallback;        // 159.6 fallback
pub mod integration;     // 159.5 planner hook

/// Convenience helper: returns true if the query looks like it would benefit from repo context.
pub fn needs_repo_context(natural_language: &str) -> bool {
    let lower = natural_language.to_lowercase();
    lower.contains("file") ||
    lower.contains("code") ||
    lower.contains("function") ||
    lower.contains("module") ||
    lower.contains("search") ||
    lower.contains("where is") ||
    lower.contains("find ") ||
    lower.contains("implementation")
}