//! File-based tools: log reading, note writing/reading/listing.

use serde_json::Value;
use std::fs;

// ── read_log ──────────────────────────────────────────────────────────────────

/// Read the tail of a log file (max 2 000 chars).
/// Only files inside `logs/` are allowed for security.
pub fn read_log(args: &Value) -> String {
    let file = args["log_file"].as_str().unwrap_or("logs/chat_log.md");

    // Security guard: only allow paths inside logs/
    let normalised = file.replace('\\', "/");
    if !normalised.starts_with("logs/") {
        return format!(
            "Security error: only files inside logs/ are readable. \
             Got '{}'. Example: 'logs/chat_log.md'",
            file
        );
    }

    match fs::read_to_string(file) {
        Ok(content) => {
            const MAX: usize = 2_000;
            if content.len() > MAX {
                format!(
                    "[…showing last {} chars of {}…]\n{}",
                    MAX,
                    file,
                    &content[content.len() - MAX..]
                )
            } else if content.is_empty() {
                format!("'{}' exists but is empty.", file)
            } else {
                content
            }
        }
        Err(e) => format!("Error reading '{}': {}", file, e),
    }
}

// ── write_note ────────────────────────────────────────────────────────────────

pub fn write_note(args: &Value) -> String {
    let title = args["title"].as_str().unwrap_or("untitled");
    let content = args["content"].as_str().unwrap_or("");

    let safe = sanitise_filename(title);
    if safe.is_empty() {
        return "Error: note title must contain at least one alphanumeric character.".to_string();
    }

    if let Err(e) = fs::create_dir_all("notes") {
        return format!("Error creating notes/ directory: {}", e);
    }

    let path = format!("notes/{}.md", safe);
    let body = format!("# {}\n\n{}", title, content);

    match fs::write(&path, &body) {
        Ok(_) => format!("✅ Note '{}' saved to {}", title, path),
        Err(e) => format!("Error saving note to '{}': {}", path, e),
    }
}

// ── read_note ─────────────────────────────────────────────────────────────────

pub fn read_note(args: &Value) -> String {
    let title = args["title"].as_str().unwrap_or("");
    let safe = sanitise_filename(title);

    let path = format!("notes/{}.md", safe);
    match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(_) => {
            let available = notes_list_internal();
            if available.is_empty() {
                format!("Note '{}' not found. No notes have been saved yet.", title)
            } else {
                format!(
                    "Note '{}' not found.\n\nAvailable notes:\n{}",
                    title,
                    available.join("\n")
                )
            }
        }
    }
}

// ── list_notes ────────────────────────────────────────────────────────────────

pub fn list_notes() -> String {
    let notes = notes_list_internal();
    if notes.is_empty() {
        "No notes saved yet. Use write_note to create one.".to_string()
    } else {
        format!("Saved notes ({}):\n{}", notes.len(), notes.join("\n"))
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn notes_list_internal() -> Vec<String> {
    fs::read_dir("notes")
        .map(|entries| {
            let mut names: Vec<String> = entries
                .flatten()
                .filter(|e| e.path().extension().map_or(false, |ext| ext == "md"))
                .filter_map(|e| {
                    e.path()
                        .file_stem()
                        .map(|s| format!("  • {}", s.to_string_lossy()))
                })
                .collect();
            names.sort();
            names
        })
        .unwrap_or_default()
}

/// Replace any character that's not alphanumeric, dash, or underscore with `_`.
/// Spaces become `_`.  Collapses consecutive underscores.
fn sanitise_filename(s: &str) -> String {
    let safe: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();

    // Collapse runs of underscores and trim them
    let mut result = String::new();
    let mut prev_under = false;
    for c in safe.chars() {
        if c == '_' {
            if !prev_under {
                result.push(c);
            }
            prev_under = true;
        } else {
            result.push(c);
            prev_under = false;
        }
    }
    result.trim_matches('_').to_string()
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_read_log_security_guard() {
        let result = read_log(&json!({"log_file": "../secrets.txt"}));
        assert!(result.contains("Security error"), "got: {}", result);
    }

    #[test]
    fn test_read_log_missing_file() {
        let result = read_log(&json!({"log_file": "logs/nonexistent_xyz_abc.md"}));
        assert!(result.contains("Error reading"), "got: {}", result);
    }

    #[test]
    fn test_sanitise_filename_spaces() {
        assert_eq!(sanitise_filename("hello world"), "hello_world");
    }

    #[test]
    fn test_sanitise_filename_special_chars() {
        assert_eq!(sanitise_filename("note: #1 test!"), "note_1_test");
    }

    #[test]
    fn test_list_notes_does_not_panic() {
        // notes/ dir may or may not exist; must not panic
        let _ = list_notes();
    }
}
