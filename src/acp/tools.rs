//! Helix tool surface mapping for ACP.
//!
//! The ACP adapter does not reimplement tool execution —
//! [`RuntimeLoop`](crate::agents::runtime_loop::RuntimeLoop) already runs
//! tools through `ToolSupervisorV2`. This module *advertises* Helix's tool
//! surface to ACP clients (tool catalog for prompts and slash-command
//! style listings) in one place.

use serde_json::Value;

/// A Helix tool, as advertised to ACP clients.
#[derive(Debug, Clone)]
pub struct HelixToolInfo {
    pub name: String,
    pub description: String,
}

/// Extract Helix's tool catalog from [`crate::tools::tool_definitions`].
///
/// Reads the `{type: "function", function: {name, description}}` shapes the
/// shared tool executor publishes and flattens them into a simple list.
pub fn helix_tool_catalog() -> Vec<HelixToolInfo> {
    let defs = crate::tools::tool_definitions();
    let mut out = Vec::new();
    if let Some(arr) = defs.as_array() {
        for def in arr {
            let name = def
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if name.is_empty() {
                continue;
            }
            let description = def
                .pointer("/function/description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            out.push(HelixToolInfo { name, description });
        }
    }
    out
}

/// Render the catalog as a compact prompt-context block.
pub fn describe_tools_for_prompt() -> String {
    let tools = helix_tool_catalog();
    if tools.is_empty() {
        return "No tools are currently available.".to_string();
    }
    let mut s = String::from("Available Helix tools:\n");
    for t in tools {
        s.push_str(&format!("- {}: {}\n", t.name, t.description));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helix_tool_catalog_is_nonempty_and_named() {
        // GIVEN Helix's shared tool definitions:
        let catalog = helix_tool_catalog();
        // THEN we get a real catalog with the known tools:
        assert!(!catalog.is_empty(), "expected Helix to advertise tools");
        let names: Vec<&str> = catalog.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"send_email"), "missing send_email: {:?}", names);
        assert!(names.contains(&"read_log"), "missing read_log: {:?}", names);
    }

    #[test]
    fn describe_tools_for_prompt_mentions_tool_names() {
        // GIVEN the catalog renderer:
        let text = describe_tools_for_prompt();
        // THEN it names real tools:
        assert!(text.contains("send_email"));
        assert!(text.starts_with("Available Helix tools:"));
    }
}
