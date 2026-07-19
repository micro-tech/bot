//! Shared tool executor — single source of truth for all tool implementations.
//!
//! Both the Ollama agentic tool-calling loop (`io/ollama/mod.rs`) and the
//! CPU `SkillRegistry` (`skills/mod.rs`) delegate here.  Keeping them in sync
//! is then automatic.
//!
//! OKF integration: When the global OKF librarian is active, its tools are
//! discoverable via list_okf_tools + merged into tool_definitions().

pub mod email_tools;
pub mod file_tools;
pub mod system_tools;

use serde_json::Value;

// ── Public dispatch entry point ───────────────────────────────────────────────

/// Execute a named tool and return its result as a plain string.
///
/// This is the single entry point for **all** tool invocations — Ollama
/// tool-calling, CPU skill requests, and direct slash-commands from the UI.
///
/// OKF tools (loaded from remote bundles) are tried after the built-in set.
pub fn execute(name: &str, args: &Value) -> String {
    // Built-in tools first
    let result = match name {
        "read_log" => file_tools::read_log(args),
        "write_note" => file_tools::write_note(args),
        "read_note" => file_tools::read_note(args),
        "list_notes" => file_tools::list_notes(),
        "send_email" => email_tools::send_email(args),
        "read_email" => email_tools::read_email(args),
        "check_inbox" => email_tools::check_inbox(args),
        "system_status" => system_tools::system_status(),
        "list_tools" => system_tools::list_tools(),
        "get_beliefs" => system_tools::get_beliefs(),
        "set_belief" => system_tools::set_belief(args),
        "bayes_show" => system_tools::bayes_show(),
        "bayes_update" => {
            let evidence = args["evidence"].as_str().unwrap_or("");
            system_tools::bayes_update(evidence)
        }
        "bayes_reset" => system_tools::bayes_reset(),
        "repo_glob" => file_tools::repo_glob(args),
        "repo_read" => file_tools::repo_read(args),
        "repo_grep" => file_tools::repo_grep(args),
        "list_okf_tools" => list_okf_tools(),
        other => String::new(), // signal: try OKF or unknown
    };

    if !result.is_empty() {
        return result;
    }

    // Try OKF-provided tools (from global librarian)
    if let Some(desc) = crate::okf::get_okf_tool_description(name) {
        return format!(
            "🧠 OKF Tool '{}'\nDescription: {}\n\n\
             (This tool is provided by the remote OKF bundle. \
              Full remote execution will be wired in a later step. \
              Args received: {})",
            name, desc, args
        );
    }

    // Unknown
    format!(
        "Unknown tool '{}' — not registered. Use list_tools or list_okf_tools to see available tools.",
        name
    )
}

/// Return a human-readable list of currently loaded OKF tools.
pub fn list_okf_tools() -> String {
    let names = crate::okf::list_okf_tool_names();
    if names.is_empty() {
        return "No OKF tools currently loaded.\n(Enable [helix.okf] in config and load a bundle via /okf/manifest or /okf/reload.)".to_string();
    }

    let mut lines = vec!["🧠 OKF Tools (from remote bundle):".to_string()];
    for name in &names {
        if let Some(desc) = crate::okf::get_okf_tool_description(name) {
            lines.push(format!("  • {} — {}", name, desc));
        } else {
            lines.push(format!("  • {}", name));
        }
    }
    lines.push("\nThese tools are discoverable by agents. Full remote dispatch coming soon.".to_string());
    lines.join("\n")
}

// ── Ollama tool definitions ───────────────────────────────────────────────────

/// Returns the JSON array of tool schemas sent to Ollama in every `/api/chat`
/// request.  Keep in sync with the `execute` dispatch table above.
///
/// Dynamically merges in any tools loaded from the OKF librarian.
pub fn tool_definitions() -> Value {
    let mut defs = vec![
        serde_json::json!({
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
        }),
        serde_json::json!({
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
        }),
        serde_json::json!({
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
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "list_notes",
                "description": "List all saved notes in the notes/ directory.",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "send_email",
                "description": "Send an email via SMTP. Falls back to logs/email_outbox.md when SMTP is not configured.",
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
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "read_email",
                "description": "Read recent emails from an IMAP folder. Returns subject, sender, and date for the last N messages.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "folder": { "type": "string", "description": "IMAP folder (default: INBOX)" },
                        "count":  { "type": "integer", "description": "Number of emails to return (default: 5, max: 20)" }
                    },
                    "required": []
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "check_inbox",
                "description": "Check how many messages are in an IMAP folder.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "folder": { "type": "string", "description": "IMAP folder name (default: INBOX)" }
                    },
                    "required": []
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "system_status",
                "description": "Get the current system status: log file sizes, note count, beliefs file, uptime timestamp.",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "list_tools",
                "description": "List every available tool or skill, with a one-line description of each.",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "list_okf_tools",
                "description": "List tools that were dynamically loaded from remote OKF (Open Knowledge Format) bundles. These are additional capabilities provided by the OKF librarian.",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "get_beliefs",
                "description": "Read the current agent beliefs from beliefs.json. Beliefs are key/value facts the agent has learned or been told.",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        }),
        serde_json::json!({
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
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "bayes_show",
                "description": "Show the current Bayesian belief state (probabilities for positive/negative/neutral hypotheses). Reads persisted state from beliefs_bayes.json.",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "bayes_update",
                "description": "Apply a Bayesian update for a piece of evidence. Evidence containing 'pos'/'good'/'yes' boosts positive; 'neg'/'bad'/'no' boosts negative; anything else boosts neutral. State persists in beliefs_bayes.json.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "evidence": { "type": "string", "description": "A short evidence string, e.g. 'positive_signal' or 'bad_outcome'" }
                    },
                    "required": ["evidence"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "bayes_reset",
                "description": "Reset the Bayesian belief state back to default priors (positive=50%, negative=30%, neutral=20%).",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "repo_glob",
                "description": "List files matching a glob pattern inside the repository (read-only). Example: '**/*.rs' or 'src/**/*.toml'.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string", "description": "Glob pattern, e.g. '**/*.rs'" }
                    },
                    "required": ["pattern"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "repo_read",
                "description": "Read a file or a specific line range from the repository (read-only). Supports optional start_line and end_line (1-based).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Relative file path" },
                        "start_line": { "type": "integer", "description": "Optional start line (1-based)" },
                        "end_line": { "type": "integer", "description": "Optional end line (inclusive)" }
                    },
                    "required": ["path"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "repo_grep",
                "description": "Search for a text pattern inside repository files (read-only). Returns matching lines with context.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string", "description": "Text or regex to search for" },
                        "path": { "type": "string", "description": "Optional directory or file to limit search (default: whole repo)" }
                    },
                    "required": ["pattern"]
                }
            }
        })
    ];

    // Merge OKF tools (if any are loaded via the global librarian)
    if let Some(librarian_arc) = crate::okf::get_global_librarian() {
        if let Ok(guard) = librarian_arc.try_lock() {
            for (name, tool) in &guard.registry.tools {
                let mut func = serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": tool.description.clone(),
                    }
                });

                if let Some(schema) = &tool.input_schema {
                    func["function"]["parameters"] = schema.clone();
                }

                defs.push(func);
            }
        }
    }

    Value::Array(defs)
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
        assert!(arr.len() >= 12, "expected >= 12 tools, got {}", arr.len());
    }
}
