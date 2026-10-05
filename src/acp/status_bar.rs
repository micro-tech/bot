//! Status bar updates for the Helix ACP server.
//!
//! Builds the `session/update` payloads Zed renders as the bottom status
//! bar. Ported from grok-cli's `src/acp/status_bar.rs` with Helix defaults
//! (model name, context window).

use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

/// Compact status line shown at the bottom of the ACP session.
#[derive(Debug, Clone)]
pub struct StatusBarState {
    pub model: String,
    pub thinking_mode: String, // "Off" | "Low" | "High"
    pub current_tokens: usize,
    pub max_tokens: usize,
    pub context_percent: f32,
    pub is_generating: bool,

    /// Visual context graph, e.g. "[.....|      ]"
    pub context_graph: String,

    /// Small icons for currently active sub-agents.
    pub agent_icons: Vec<String>,
}

impl Default for StatusBarState {
    fn default() -> Self {
        Self {
            model: "helix".to_string(),
            thinking_mode: "Off".to_string(),
            current_tokens: 0,
            max_tokens: 128_000,
            context_percent: 0.0,
            is_generating: false,
            context_graph: "[............]".to_string(),
            agent_icons: vec![],
        }
    }
}

/// Maps a sub-agent role name to a small icon.
pub fn icon_for_agent_role(role: &str) -> &'static str {
    match role.to_lowercase().as_str() {
        "planner" | "plan" | "architect" => "🗺️",
        "coder" | "code" | "dev" | "programmer" => "💻",
        "researcher" | "research" | "explorer" | "search" => "🔎",
        "verifier" | "tester" => "✅",
        "reviewer" => "👀",
        "writer" | "docs" => "📝",
        "debugger" | "fixer" => "🐛",
        _ => "🧑",
    }
}

/// Creates a compact visual context meter: `[.....|..]`.
pub fn format_context_graph(current: usize, max: usize, compress_at: usize) -> String {
    const WIDTH: usize = 12;

    if max == 0 {
        return "[............]".to_string();
    }

    let used_ratio = (current as f64 / max as f64).clamp(0.0, 1.0);
    let comp_ratio = (compress_at as f64 / max as f64).clamp(0.0, 1.0);

    let used_chars = (used_ratio * WIDTH as f64).round() as usize;
    let comp_pos = (comp_ratio * WIDTH as f64).round() as usize;

    let mut bar = String::with_capacity(WIDTH + 3);
    bar.push('[');

    for i in 0..WIDTH {
        if i < used_chars {
            bar.push('█');
        } else if i == comp_pos {
            bar.push('|');
        } else {
            bar.push('░');
        }
    }

    bar.push(']');
    bar
}

/// Build the compact status line payload.
pub fn build_status_line(state: &StatusBarState) -> Value {
    let status_icon = if state.is_generating { "⏳" } else { "●" };

    let agents_part = if state.agent_icons.is_empty() {
        String::new()
    } else {
        format!(" {}", state.agent_icons.join(""))
    };

    json!({
        "sessionUpdate": "status_update",
        "status": {
            "kind": "compact",
            "text": format!(
                "{} {}  {}  🧠 {}{}",
                status_icon,
                state.model,
                state.context_graph,
                state.thinking_mode,
                agents_part
            ),
            "timestamp": current_timestamp(),
        }
    })
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_bar_default_uses_helix_model() {
        // GIVEN a default status bar:
        let s = StatusBarState::default();
        // THEN it brands Helix, not grok:
        assert_eq!(s.model, "helix");
    }

    #[test]
    fn format_context_graph_half_used() {
        // GIVEN half the context used:
        let g = format_context_graph(64_000, 128_000, 128_000);
        // THEN the bar shows the shape:
        assert!(g.starts_with('['));
        assert!(g.ends_with(']'));
        assert!(g.contains('█'));
    }

    #[test]
    fn format_context_graph_empty_max() {
        // GIVEN a zero max (degenerate):
        let g = format_context_graph(0, 0, 0);
        // THEN we get the placeholder, not a panic:
        assert_eq!(g, "[............]");
    }

    #[test]
    fn build_status_line_has_compact_shape() {
        // GIVEN a default state:
        let s = StatusBarState::default();
        // WHEN we build the payload:
        let v = build_status_line(&s);
        // THEN it has the expected envelope:
        assert_eq!(v["sessionUpdate"], "status_update");
        assert_eq!(v["status"]["kind"], "compact");
    }
}
