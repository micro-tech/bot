//! Shared tool executor — single source of truth for all tool implementations.
//!
//! Both the Ollama agentic tool-calling loop (`io/ollama/mod.rs`) and the
//! CPU `SkillRegistry` (`skills/mod.rs`) delegate here.  Keeping them in sync
//! is then automatic.

pub mod file_tools;
pub mod system_tools;

use serde_json::Value;

// ── Public dispatch entry point ───────────────────────────────────────────────

/// Execute a named tool and return its result as a plain string.
///
/// This is the single entry point for **all** tool invocations — Ollama
/// tool-calling, CPU skill requests, and direct slash-commands from the UI.
pub fn execute(name: &str, args: &Value) -> String {
    match name {
        "read_log" => file_tools::read_log(args),
        "write_note" => file_tools::write_note(args),
        "read_note" => file_tools::read_note(args),
        "list_notes" => file_tools::list_notes(),
        "send_email" => system_tools::send_email(args),
        "system_status" => system_tools::system_status(),
        "list_tools" => system_tools::list_tools(),
        "get_beliefs" => system_tools::get_beliefs(),
        "set_belief" => system_tools::set_belief(args),
        other => format!(
            "Unknown tool '{}' — not registered. Use list_tools to see available tools.",
            other
        ),
    }
}

// ── Ollama tool definitions ───────────────────────────────────────────────────

/// Returns the JSON array of tool schemas sent to Ollama in every `/api/chat`
/// request.  Keep in sync with the `execute` dispatch table above.
pub fn tool_definitions() -> Value {
    serde_json::json!([
        {
            "type": "function",
            "function": {
                "name": "read_log",
                "description": "Read the last portion of a log file from the logs/ directory. Use this to check errors, chat history, or system events.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "log_file": {
                            "type": "string",
                            "description": "Relative path to the log file, e.g. 'logs/chat_log.md' or 'logs/error_log.md'"
                        }
                    },
                    "required": ["log_file"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "write_note",
                "description": "Save a note or piece of information to the notes/ directory for future reference.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "title":   { "type": "string", "description": "Short title for the note (used as filename)" },
                        "content": { "type": "string", "description": "Full content of the note in markdown format" }
                    },
                    "required": ["title", "content"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "read_note",
                "description": "Read a previously saved note from the notes/ directory.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string", "description": "Title of the note to read" }
                    },
                    "required": ["title"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "list_notes",
                "description": "List all saved notes in the notes/ directory.",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "send_email",
                "description": "Queue an alert or notification email to be sent.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "to":      { "type": "string", "description": "Recipient email address" },
                        "subject": { "type": "string", "description": "Email subject line" },
                        "body":    { "type": "string", "description": "Plain-text body of the email" }
                    },
                    "required": ["to", "subject", "body"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "system_status",
                "description": "Get the current system status: log file sizes, note count, beliefs file, uptime timestamp.",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "list_tools",
                "description": "List every available tool or skill, with a one-line description of each.",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "get_beliefs",
                "description": "Read the current agent beliefs from beliefs.json. Beliefs are key/value facts the agent has learned or been told.",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "set_belief",
                "description": "Set or update an agent belief. Beliefs persist across restarts in beliefs.json.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "key":   { "type": "string", "description": "Belief key (short identifier)" },
                        "value": { "type": "string", "description": "Belief value" }
                    },
                    "required": ["key", "value"]
                }
            }
        }
    ])
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_unknown_tool() {
        let result = execute("totally_unknown", &json!({}));
        assert!(result.contains("Unknown tool"), "got: {}", result);
    }

    #[test]
    fn test_list_tools_returns_content() {
        let result = execute("list_tools", &json!({}));
        assert!(result.contains("read_log"), "got: {}", result);
    }

    #[test]
    fn test_system_status_returns_content() {
        let result = execute("system_status", &json!({}));
        assert!(!result.is_empty());
    }

    #[test]
    fn test_tool_definitions_is_array() {
        let defs = tool_definitions();
        assert!(defs.is_array());
        let arr = defs.as_array().unwrap();
        assert!(arr.len() >= 9, "expected >= 9 tools, got {}", arr.len());
    }
}
