// router/debug_cli.rs - router_debug CLI command and snapshot support (Tasks 147.4 + 147.5)

use crate::router::debug::{DecisionTree, generate_explain, VizJSON, generate_viz_json};
use crate::router::context::{RoutingContext, LLMBackend};
use crate::router::config::RouterConfig;
use std::fs;
use std::path::Path;
use chrono::Utc;

/// Result of a debug routing decision
#[derive(Debug, Clone)]
pub struct DebugResult {
    pub tree: DecisionTree,
    pub viz: VizJSON,
    pub narrative: String,
}

/// Run a debug routing decision and return full explain output
pub fn router_debug(
    ctx: &RoutingContext,
    backend: LLMBackend,
    latency_ms: u64,
) -> DebugResult {
    let tree = generate_explain(ctx, backend, latency_ms);
    let narrative = crate::router::debug_narrative::generate_narrative(&tree);
    let viz = generate_viz_json(tree.clone());

    DebugResult { tree, viz, narrative }
}

/// Save a debug snapshot to disk for later replay
pub fn save_snapshot<P: AsRef<Path>>(tree: &DecisionTree, path: P) -> Result<(), String> {
    let json = serde_json::to_string_pretty(tree).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

/// Load a debug snapshot from disk
pub fn load_snapshot<P: AsRef<Path>>(path: P) -> Result<DecisionTree, String> {
    let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let tree: DecisionTree = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    Ok(tree)
}

/// Simple CLI-style debug entry point (can be called from main CLI)
pub fn run_debug_cli(prompt: &str, forced_backend: Option<LLMBackend>) -> String {
    let ctx = RoutingContext {
        prompt: prompt.to_string(),
        token_estimate: (prompt.len() / 4) as u32,
        has_code: prompt.contains("```"),
        complexity_score: 0.5,
        timestamp: Utc::now(),
        user_override: forced_backend,
        telemetry: None,
        health: None,
    };

    let backend = forced_backend.unwrap_or(LLMBackend::Ollama);
    let result = router_debug(&ctx, backend, 15);

    format!(
        "{}\n\n--- JSON ---\n{}",
        result.narrative,
        serde_json::to_string_pretty(&result.viz).unwrap_or_default()
    )
}