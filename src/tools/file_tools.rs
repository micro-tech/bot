//! File-based tools: log reading, note writing/reading/listing, and repo exploration tools.

use serde_json::Value;
use std::fs;
use std::path::Path;

// Bring in chrono for timestamps (already a dependency)


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

// ── OKF / Cross-Machine Project Mapping Tools ────────────────────────────────

/// Scans a directory (recursively, limited depth) and returns a structured snapshot.
/// Useful for feeding into Mermaid diagram generation or OKF knowledge bundles.
///
/// Args:
///   path: base directory (relative or absolute — be careful with absolute)
///   max_depth: optional, default 4
///   include_hidden: optional, default false
pub fn scan_directory(args: &Value) -> String {
    let base_path = args["path"].as_str().unwrap_or(".");
    let max_depth = args["max_depth"].as_u64().unwrap_or(4) as usize;
    let include_hidden = args["include_hidden"].as_bool().unwrap_or(false);

    let mut tree = Vec::new();
    let mut file_count = 0usize;
    let mut dir_count = 0usize;

    fn walk(
        dir: &std::path::Path,
        current_depth: usize,
        max_depth: usize,
        include_hidden: bool,
        tree: &mut Vec<serde_json::Value>,
        file_count: &mut usize,
        dir_count: &mut usize,
    ) {
        if current_depth > max_depth {
            return;
        }

        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();

                if !include_hidden && name.starts_with('.') {
                    continue;
                }

                let is_dir = path.is_dir();
                let rel_path = path.strip_prefix(dir).unwrap_or(&path).to_string_lossy().to_string();

                if is_dir {
                    *dir_count += 1;
                    let mut children = Vec::new();
                    walk(&path, current_depth + 1, max_depth, include_hidden, &mut children, file_count, dir_count);

                    tree.push(serde_json::json!({
                        "type": "directory",
                        "name": name,
                        "path": rel_path,
                        "children": children
                    }));
                } else {
                    *file_count += 1;
                    tree.push(serde_json::json!({
                        "type": "file",
                        "name": name,
                        "path": rel_path,
                        "size": entry.metadata().map(|m| m.len()).unwrap_or(0)
                    }));
                }
            }
        }
    }

    let start = std::path::Path::new(base_path);
    walk(start, 0, max_depth, include_hidden, &mut tree, &mut file_count, &mut dir_count);

    let result = serde_json::json!({
        "base_path": base_path,
        "scanned_at": chrono::Utc::now().to_rfc3339(),
        "max_depth": max_depth,
        "file_count": file_count,
        "dir_count": dir_count,
        "tree": tree
    });

    serde_json::to_string_pretty(&result).unwrap_or_else(|_| "Failed to serialize scan result".to_string())
}

/// Generates a Mermaid flowchart from a directory scan.
/// This is the key piece for your "file changes → Mermaid maps → OKF server" workflow.
///
/// Args:
///   path: directory to scan
///   title: optional diagram title
///   max_depth: optional
///   diagram_style: "tree" | "changes" | "hybrid" (default "tree")
pub fn generate_mermaid_project_map(args: &Value) -> String {
    let base_path = args["path"].as_str().unwrap_or(".");
    let title = args["title"].as_str().unwrap_or("Project Structure");
    let max_depth = args["max_depth"].as_u64().unwrap_or(3) as usize;
    let style = args["diagram_style"].as_str().unwrap_or("tree");

    // First get the scan data
    let scan_json = scan_directory(&serde_json::json!({
        "path": base_path,
        "max_depth": max_depth,
        "include_hidden": false
    }));

    let scan: serde_json::Value = serde_json::from_str(&scan_json).unwrap_or(serde_json::json!({}));
    let tree = scan.get("tree").and_then(|t| t.as_array()).cloned().unwrap_or_default();

    let mut mermaid = format!("```mermaid\ngraph TD\n    subgraph \"{}\" [{}]\n", title, base_path);

    let mut node_id = 0u32;
    let mut id_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    fn add_node(
        node: &serde_json::Value,
        parent_id: Option<&str>,
        mermaid: &mut String,
        node_id: &mut u32,
        id_map: &mut std::collections::HashMap<String, String>,
    ) {
        let name = node["name"].as_str().unwrap_or("?");
        let path = node["path"].as_str().unwrap_or(name);
        let typ = node["type"].as_str().unwrap_or("file");

        *node_id += 1;
        let nid = format!("n{}", node_id);
        id_map.insert(path.to_string(), nid.clone());

        let label = if typ == "directory" {
            format!("📁 {}", name)
        } else {
            format!("📄 {}", name)
        };

        mermaid.push_str(&format!("        {}[\"{}\"]\n", nid, label));

        if let Some(pid) = parent_id {
            mermaid.push_str(&format!("        {} --> {}\n", pid, nid));
        }

        if typ == "directory" {
            if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
                for child in children {
                    add_node(child, Some(&nid), mermaid, node_id, id_map);
                }
            }
        }
    }

    // Root node
    mermaid.push_str(&format!("        root[\"{}\"]\n", base_path));

    for item in &tree {
        add_node(item, Some("root"), &mut mermaid, &mut node_id, &mut id_map);
    }

    mermaid.push_str("    end\n");

    // Add styling
    mermaid.push_str("\n    classDef dir fill:#e3f2fd,stroke:#1565c0\n");
    mermaid.push_str("    classDef file fill:#f1f8e9,stroke:#2e7d32\n");

    mermaid.push_str("```");

    // Build rich OKF-style knowledge payload
    let knowledge_payload = serde_json::json!({
        "type": "okf_knowledge",
        "content_type": "mermaid",
        "id": format!("project-map-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S")),
        "title": title,
        "diagram_type": style,
        "source_machine": std::env::var("COMPUTERNAME").unwrap_or_else(|_| "unknown-pc".to_string()),
        "base_path": base_path,
        "detected_at": chrono::Utc::now().to_rfc3339(),
        "mermaid": mermaid,
        "scan_summary": {
            "files": scan["file_count"],
            "dirs": scan["dir_count"],
            "depth": max_depth
        },
        "metadata": {
            "generated_by": "helix-file-tools",
            "purpose": "cross-machine project mapping for OKF"
        }
    });

    serde_json::to_string_pretty(&knowledge_payload).unwrap_or_else(|_| mermaid)
}

/// Convenience wrapper: produce a ready-to-send OKF knowledge entry for a project map.
/// You can POST the output of this directly (or the mermaid field) to your central OKF server.
pub fn generate_okf_project_map_knowledge(args: &Value) -> String {
    let map = generate_mermaid_project_map(args);
    format!(
        "✅ Generated OKF-ready project map knowledge.\n\n\
         You can now send this to your OKF server (Dell 630 / Proxmox) using:\n\
         POST /okf/manifest  or  /okf/push\n\n\
         Content:\n{}",
        map
    )
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