//! Integration helpers for the main planner / runtime (159.5).
//!
//! Provides a lightweight "Do I need repo context?" hook that the main
//! planner or RuntimeLoop can call before deciding on heavy file reads.

use super::{needs_repo_context, explorer_impl::LocalRepoExplorer, RepoEvidence, RepoExplorer, RepoQuery};
use crate::agents::agent_state::AgentState;

/// Returns true if the current state/query would benefit from repo exploration.
pub fn should_explore_repo(state: &AgentState, last_user_message: &str) -> bool {
    // Simple heuristic + state check
    if state.step_count > 3 {
        return false; // avoid repeated expensive exploration
    }
    needs_repo_context(last_user_message)
}

/// Convenience wrapper that runs the explorer (if a client is available) and
/// injects the resulting evidence into the agent state messages.
pub async fn maybe_inject_repo_evidence(
    explorer: Option<&LocalRepoExplorer>,
    state: &mut AgentState,
    query: RepoQuery,
) -> Option<RepoEvidence> {
    if let Some(ex) = explorer {
        let evidence = ex.explore(query).await;
        if !evidence.is_empty() {
            // Inject a compact summary into state so the planner sees it
            let summary = evidence
                .items
                .iter()
                .map(|e| format!("{}: {}", e.path, e.summary))
                .collect::<Vec<_>>()
                .join(" | ");

            state.messages.push(format!("[RepoExplorer] {}", summary));
            return Some(evidence);
        }
    }
    None
}