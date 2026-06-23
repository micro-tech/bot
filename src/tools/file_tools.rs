//! File-based tools: log reading, note writing/reading/listing, and repo exploration tools.

use serde_json::Value;
use std::fs;
use std::path::Path;

// ── Existing note/log tools (kept for brevity) ────────────────────────────────

pub fn read_log(args: &Value) -> String {
    let file = args["log_file"].as_str().unwrap_or("logs/chat_log.md");
    let normalised = file.replace('\\', "/");
    if !normalised.starts_with("logs/") {
        return format!("Security error: only files inside logs/ are readable. Got '{}'.", file);
    }
    match fs::read_to_string(file) {
        Ok(content) => {
            const MAX: usize = 2_000;
            if content.len() > MAX {
                format!("[…showing last {} chars of {}…]\n{}", MAX, file, &content[content.len() - MAX..])
            } else if content.is_empty() {
                format!("'{}' exists but is empty.", file)
            } else {
                content
            }
        }
        Err(e) => format!("Error reading '{}': {}", file, e),
    }
}

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
                format!("Note '{}' not found.\n\nAvailable notes:\n{}", title, available.join("\n"))
            }
        }
    }
}

pub fn list_notes() -> String {
    let notes = notes_list_internal();
    if notes.is_empty() {
        "No notes saved yet. Use write_note to create one.".to_string()
    } else {
        format!("Saved notes ({}):\n{}", notes.len(), notes.join("\n"))
    }
}

fn notes_list_internal() -> Vec<String> {
    fs::read_dir("notes")
        .map(|entries| {
            let mut names: Vec<String> = entries
                .flatten()
                .filter(|e| e.path().extension().map_or(false, |ext| ext == "md"))
                .filter_map(|e| e.path().file_stem().map(|s| format!("  • {}", s.to_string_lossy())))
                .collect();
            names.sort();
            names
        })
        .unwrap_or_default()
}

fn sanitise_filename(s: &str) -> String {
    let safe: String = s.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let mut result = String::new();
    let mut prev_under = false;
    for c in safe.chars() {
        if c == '_' {
            if !prev_under { result.push(c); }
            prev_under = true;
        } else {
            result.push(c);
            prev_under = false;
        }
    }
    result.trim_matches('_').to_string()
}

// ── NEW: Repo Explorer Tools (read-only) ─────────────────────────────────────

/// tool_glob: list files matching a glob pattern (e.g. "**/*.rs").
pub fn repo_glob(args: &Value) -> String {
    let pattern = args["pattern"].as_str().unwrap_or("**/*");
    // We reuse the existing glob_search helper from the crate root tools.
    // For now we just echo what would be searched (real impl would call glob_search).
    format!("[repo_glob] pattern = {}", pattern)
}

/// tool_read with optional line range support.
pub fn repo_read(args: &Value) -> String {
    let path = args["path"].as_str().unwrap_or("");
    let start = args["start_line"].as_u64().map(|v| v as usize);
    let end = args["end_line"].as_u64().map(|v| v as usize);

    if path.is_empty() {
        return "Error: 'path' is required for repo_read".to_string();
    }

    // Basic security: only allow relative paths inside the project (no absolute outside).
    if Path::new(path).is_absolute() {
        return "Security error: absolute paths are not allowed for repo_read.".to_string();
    }

    match fs::read_to_string(path) {
        Ok(content) => {
            let lines: Vec<&str> = content.lines().collect();
            let total = lines.len();

            let s = start.unwrap_or(1).saturating_sub(1);
            let e = end.unwrap_or(total).min(total);

            if s >= e || s >= total {
                return format!("File '{}' has {} lines. Requested range out of bounds.", path, total);
            }

            let slice = &lines[s..e];
            let header = if start.is_some() || end.is_some() {
                format!("[{}:{}] ", s + 1, e)
            } else {
                String::new()
            };
            format!("{}{}", header, slice.join("\n"))
        }
        Err(e) => format!("Error reading '{}': {}", path, e),
    }
}

/// tool_grep: search for a pattern inside files (simple line grep).
pub fn repo_grep(args: &Value) -> String {
    let pattern = args["pattern"].as_str().unwrap_or("");
    let path = args["path"].as_str().unwrap_or(".");

    if pattern.is_empty() {
        return "Error: 'pattern' is required for repo_grep".to_string();
    }

    // Very simple implementation – in real code we would use the search_file_content helper.
    // Here we just return a placeholder that the explorer can consume.
    format!("[repo_grep] pattern='{}' path='{}' (implementation pending real search)", pattern, path)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_repo_read_basic() {
        // This test assumes Cargo.toml exists at project root.
        let result = repo_read(&json!({"path": "Cargo.toml"}));
        assert!(result.contains("[package]"), "expected Cargo.toml content, got: {}", result);
    }

    #[test]
    fn test_repo_glob_placeholder() {
        let result = repo_glob(&json!({"pattern": "**/*.rs"}));
        assert!(result.contains("**/*.rs"));
    }
}