//! Fallback logic when the local LLM client is unavailable or returns poor results.
//! (Part of 159.6)

use super::{RepoEvidence, RepoEvidenceItem, RepoQuery};

/// Simple static fallback that returns a few high-value files when the explorer cannot run.
pub fn static_fallback(query: &RepoQuery) -> RepoEvidence {
    let mut items = vec![
        RepoEvidenceItem {
            path: "Cargo.toml".to_string(),
            line_start: Some(1),
            line_end: Some(30),
            summary: "Project manifest and dependencies".to_string(),
        },
        RepoEvidenceItem {
            path: "src/main.rs".to_string(),
            line_start: Some(1),
            line_end: Some(50),
            summary: "Application entry point".to_string(),
        },
    ];

    // Add a hint-based item if the query mentions a specific area
    if query.natural_language.to_lowercase().contains("tool") {
        items.push(RepoEvidenceItem {
            path: "src/tools/mod.rs".to_string(),
            line_start: Some(1),
            line_end: Some(40),
            summary: "Tool registry and dispatch logic".to_string(),
        });
    }

    if query.natural_language.to_lowercase().contains("agent") {
        items.push(RepoEvidenceItem {
            path: "src/agents/mod.rs".to_string(),
            line_start: Some(1),
            line_end: Some(30),
            summary: "Agent module declarations".to_string(),
        });
    }

    RepoEvidence { items }
}

/// Returns true if we should skip the expensive local LLM path.
pub fn should_use_fallback(llm_available: bool, query_len: usize) -> bool {
    !llm_available || query_len < 5
}