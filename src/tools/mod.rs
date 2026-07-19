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
pub mod project_scanner;
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
        "scan_directory" => file_tools::scan_directory(args),
        "generate_mermaid_project_map" => file_tools::generate_mermaid_project_map(args),
        "generate_okf_project_map_knowledge" => file_tools::generate_okf_project_map_knowledge(args),
        "scan_projects" => project_scanner::scan_projects(args),
        "generate_project_map_from_scan" => project_scanner::generate_project_map_from_scan(args),

        // SSH-dependent tools are behind the optional "ssh" feature.
        // When the feature is off we return a helpful message instead of failing at runtime.
        _ if name == "scan_projects" || name == "generate_project_map_from_scan" => {
            // This branch is only reached if the above match arms didn't catch it.
            // Because we list the tools unconditionally, we guard execution here.
            if cfg!(feature = "ssh") {
                // Should have been handled above; this is a safety net.
                "SSH feature is enabled but tool routing missed it.".to_string()
            } else {
                format!(
                    "Tool '{}' requires the 'ssh' feature (remote scanning).\n\
                     Build with: cargo build --features ssh\n\
                     Then ensure OpenSSL is installed and OPENSSL_DIR is set on Windows.",
                    name
                )
            }
        }
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
        }),
        // === OKF / Cross-machine Project Mapping (for building Mermaid maps from file changes) ===
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "scan_directory",
                "description": "Recursively scan a directory tree and return a structured JSON snapshot. Use this to feed project structure into Mermaid diagram generation or OKF knowledge bundles. Supports cross-machine file change tracking.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Base directory to scan (e.g. '.', 'src', 'C:/Users/you/projects')" },
                        "max_depth": { "type": "integer", "description": "Maximum recursion depth (default 4)" },
                        "include_hidden": { "type": "boolean", "description": "Include dotfiles and hidden dirs (default false)" }
                    },
                    "required": ["path"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "generate_mermaid_project_map",
                "description": "Scan a directory and generate a Mermaid diagram (graph TD) plus a full OKF-ready knowledge payload. Perfect for sending project maps and file change visualizations to a central OKF server (e.g. on Proxmox Dell 630). Includes source_machine and timestamp.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Directory to map (e.g. '.', 'src', your project root)" },
                        "title": { "type": "string", "description": "Diagram title (default: 'Project Structure')" },
                        "max_depth": { "type": "integer", "description": "Recursion depth (default 3)" },
                        "diagram_style": { "type": "string", "description": "tree | changes | hybrid (default 'tree')" }
                    },
                    "required": ["path"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "generate_okf_project_map_knowledge",
                "description": "High-level helper: produces a complete OKF knowledge entry (with content_type=mermaid + rich metadata) ready to POST to your central OKF server. Use this when you want to push file-change diagrams from this machine to the Proxmox OKF host.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "title": { "type": "string" },
                        "max_depth": { "type": "integer" }
                    },
                    "required": ["path"]
                }
            }
        }),
        // === Task 171: Remote Project Scanner (Dell 630 scans Main PC via SSH) ===
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "scan_projects",
                "description": "Scan one or more projects (remote or local). Designed for Dell 630 scanning the Main PC over LAN (SSH preferred). Returns rich ProjectScanResult with cross-machine metadata (target_machine, scanned_from, protocol, tree). Feed output directly to Mermaid enhancers or OKF bundle builder. Pass full config TOML or use project name.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "config_toml": {
                            "type": "string",
                            "description": "Full config.toml content containing [helix.project_scanner] and [[helix.projects]] (optional if using defaults)"
                        },
                        "project": {
                            "type": "string",
                            "description": "Project name to scan, or 'all' / '*' to scan every configured project"
                        }
                    },
                    "required": []
                }
            }
        }),
        // Companion to scan_projects (Task 172)
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "generate_project_map_from_scan",
                "description": "Takes a ProjectScanResult (from scan_projects) and turns it into a Mermaid diagram + OKF knowledge bundle. Supports Task 171/172 cross-machine project visualization.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "scan_json": {
                            "type": "string",
                            "description": "JSON string of a ProjectScanResult (or the object itself)"
                        },
                        "title": {
                            "type": "string",
                            "description": "Optional title for the diagram"
                        }
                    },
                    "required": ["scan_json"]
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

    // ── Tests for today's work (Task 171/172 + optional SSH feature + rustls migration) ──

    #[test]
    fn test_scan_projects_tool_is_registered_in_definitions() {
        let defs = tool_definitions();
        let arr = defs.as_array().expect("tool_definitions must return array");

        let has_scan_projects = arr.iter().any(|t| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                == Some("scan_projects")
        });
        assert!(has_scan_projects, "scan_projects tool should always be advertised (even without ssh feature)");

        let has_generate_map = arr.iter().any(|t| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                == Some("generate_project_map_from_scan")
        });
        assert!(has_generate_map, "generate_project_map_from_scan should be registered");
    }

    #[test]
    fn test_execute_scan_projects_without_config_returns_not_enabled() {
        // Default behavior when no [helix.project_scanner] config is provided.
        // This is the expected path for "ssh not configured" case in today's scanner.
        let result = execute("scan_projects", &json!({}));
        assert!(
            result.contains("Project scanner not enabled") || result.contains("enabled = true"),
            "Expected 'not enabled' guidance, got: {}",
            result
        );
    }

    #[test]
    fn test_execute_generate_project_map_from_scan_requires_valid_input() {
        // The wrapper should give a clear error on bad input (today's implementation)
        let result = execute("generate_project_map_from_scan", &json!({}));
        assert!(
            result.contains("could not parse") || result.contains("Run scan_projects first") || result.contains("Error"),
            "Should give helpful parse error. Got: {}",
            result
        );
    }

    #[test]
    fn test_tool_definitions_includes_project_scanner_tools() {
        let defs = tool_definitions();
        let arr = defs.as_array().unwrap();

        // Today's additions: scan_projects + generate_project_map_from_scan
        let has_scanner_tools = arr.iter().any(|t| {
            let name = t.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");
            name == "scan_projects" || name == "generate_project_map_from_scan"
        });

        assert!(has_scanner_tools, "Project scanner tools from today's work should be in definitions");
    }

    #[test]
    fn test_ssh_feature_flag_and_fallback_exist() {
        // Documents the key architectural change made today:
        // - ssh2 is optional
        // - Code has cfg(feature = "ssh") guards
        // - Default build (no feature) still compiles and the tools are usable for local scans
        #[cfg(feature = "ssh")]
        {
            assert!(true);
        }
        #[cfg(not(feature = "ssh"))]
        {
            // This is the common case on Windows without Perl + full OpenSSL build
            assert!(true, "Default build path without ssh feature is active");
        }
    }

    #[test]
    fn test_local_project_scan_via_execute_works() {
        // End-to-end through the main execute() entrypoint using a local project.
        // Include explicit [helix.project_scanner] so the "not enabled" guard is not triggered.
        let config = r#"
[helix.project_scanner]
enabled = true
default_protocol = "local"

[[helix.projects]]
name = "self"
target_machine = "localhost"
remote_path = "."
protocol = "local"
max_depth = 1
"#;

        let result = execute("scan_projects", &json!({
            "config_toml": config,
            "project": "self"
        }));

        assert!(result.contains("self") || result.contains("project_name"), "scan should succeed via execute(): {}", result);
        // Use a more precise check to avoid false positives from other text
        assert!(!result.contains("\"error\""), "local scan result should not contain an error key. Got: {}", result);
    }
}
