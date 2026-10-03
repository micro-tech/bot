//! Project Scanner Module (Task 171)
//! Scans projects, with support for remote access (SSH preferred).
//! Runs on Dell 630, targets Main PC over LAN.
//!
//! Also provides Mermaid generation from scan results (repurposed Task 172).
//!
//! SSH support is behind the optional "ssh" Cargo feature:
//!   cargo build --features ssh
//!
//! This avoids pulling in ssh2 -> libssh2-sys -> openssl-sys (which on Windows
//! tries to build OpenSSL from source and needs Perl).
//!
//! See Cargo.toml [features] and the comments there for setup on Windows
//! (set OPENSSL_DIR after installing OpenSSL).

use serde_json::Value;
use std::path::Path;
use std::fs;
use chrono::Utc;

use crate::config::project_scanner::{ProjectConfig, ProjectScannerConfig};

/// Structured result of a project scan with cross-machine metadata.
/// This is the canonical output that feeds Mermaid enhancers and OKF bundles.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProjectScanResult {
    pub project_name: String,
    pub target_machine: String,     // e.g. "main-pc"
    pub scanned_from: String,       // e.g. "dell630"
    pub protocol: String,           // "ssh", "smb", "local", etc.
    pub base_path: String,
    pub scanned_at: String,
    pub file_count: usize,
    pub dir_count: usize,
    pub max_depth: usize,
    pub tree: Vec<Value>,
    pub metadata: Value,
}

/// Main entry: scan a single project using its config + global scanner config.
/// For "ssh" protocol we use real SSH when possible (via ssh2 + system or vendored OpenSSL).
pub fn scan_project(project: &ProjectConfig, scanner_cfg: &ProjectScannerConfig) -> ProjectScanResult {
    let protocol = project.effective_protocol(&scanner_cfg.default_protocol());
    let max_depth = project.effective_max_depth(scanner_cfg.default_max_depth());
    let scanned_from = scanner_cfg.scanned_from();

    let effective_path = project.remote_path.clone();

    let (tree, file_count, dir_count) = if protocol == "local" || Path::new(&effective_path).exists() {
        walk_local_directory(&effective_path, max_depth, &project.include_globs, &project.exclude_globs)
    } else if protocol == "ssh" {
        // Real SSH remote walker (preferred for Dell 630 → Main PC over LAN).
        // This code path is only active when the "ssh" Cargo feature is enabled.
        // See Cargo.toml [features] + .cargo/config.toml for Windows setup (avoids Perl + openssl-src).
        #[cfg(feature = "ssh")]
        {
            match walk_ssh_directory(
                &effective_path,
                &project.target_machine,
                "youruser", // TODO: load from scanner_cfg.machines[target_machine]
                None,       // TODO: load ssh_key_path
                max_depth,
                &project.include_globs,
                &project.exclude_globs,
            ) {
                Ok(res) => res,
                Err(e) => {
                    eprintln!("[project_scanner] SSH scan failed: {}. Using local fallback if possible.", e);
                    if Path::new(&effective_path).exists() {
                        walk_local_directory(&effective_path, max_depth, &project.include_globs, &project.exclude_globs)
                    } else {
                        (
                            vec![serde_json::json!({"type": "error", "message": format!("SSH error: {}", e)})],
                            0,
                            0,
                        )
                    }
                }
            }
        }
        #[cfg(not(feature = "ssh"))]
        {
            // SSH feature disabled — fall back to local or give clear error
            eprintln!(
                "[project_scanner] Protocol 'ssh' requested for '{}' but 'ssh' feature is not enabled. \
                 Build with: cargo build --features ssh",
                project.name
            );
            if Path::new(&effective_path).exists() {
                walk_local_directory(&effective_path, max_depth, &project.include_globs, &project.exclude_globs)
            } else {
                (
                    vec![serde_json::json!({
                        "type": "error",
                        "message": "SSH not available (feature 'ssh' disabled). Rebuild with --features ssh and set OPENSSL_DIR if on Windows."
                    })],
                    0,
                    0,
                )
            }
        }
    } else {
        // smb / http stubs
        eprintln!(
            "[project_scanner] Protocol '{}' for project '{}' not fully implemented yet (stub).",
            protocol, project.name
        );
        if Path::new(&effective_path).exists() {
            walk_local_directory(&effective_path, max_depth, &project.include_globs, &project.exclude_globs)
        } else {
            (vec![], 0, 0)
        }
    };

    ProjectScanResult {
        project_name: project.name.clone(),
        target_machine: project.target_machine.clone(),
        scanned_from,
        protocol,
        base_path: effective_path.clone(),
        scanned_at: Utc::now().to_rfc3339(),
        file_count,
        dir_count,
        max_depth,
        tree,
        metadata: serde_json::json!({
            "description": project.description,
            "include_globs": project.include_globs,
            "exclude_globs": project.exclude_globs,
            "source": "helix-project-scanner",
            "version": "0.1"
        }),
    }
}

/// Walks a local (or mounted) directory. Respects include/exclude globs at a basic level.
fn walk_local_directory(
    base_path: &str,
    max_depth: usize,
    include_globs: &Option<Vec<String>>,
    exclude_globs: &Option<Vec<String>>,
) -> (Vec<Value>, usize, usize) {
    let mut tree = Vec::new();
    let mut file_count = 0usize;
    let mut dir_count = 0usize;

    fn should_include(name: &str, rel_path: &str, include: &Option<Vec<String>>, exclude: &Option<Vec<String>>) -> bool {
        if let Some(ex) = exclude {
            for pat in ex {
                let pat_clean = pat.trim_matches('*');
                if rel_path.contains(pat_clean) || name.contains(pat_clean) {
                    return false;
                }
            }
        }
        if let Some(inc) = include {
            if inc.is_empty() { return true; }
            for pat in inc {
                let pat_clean = pat.trim_matches('*');
                if rel_path.contains(pat_clean) || name.contains(pat_clean) {
                    return true;
                }
            }
            return false;
        }
        true
    }

    fn walk(
        dir: &Path,
        current_depth: usize,
        max_depth: usize,
        include: &Option<Vec<String>>,
        exclude: &Option<Vec<String>>,
        tree: &mut Vec<Value>,
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
                let rel_path = path.strip_prefix(dir).unwrap_or(&path).to_string_lossy().to_string();

                if !should_include(&name, &rel_path, include, exclude) {
                    continue;
                }

                let is_dir = path.is_dir();

                if is_dir {
                    *dir_count += 1;
                    let mut children = Vec::new();
                    walk(&path, current_depth + 1, max_depth, include, exclude, &mut children, file_count, dir_count);

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

    let start = Path::new(base_path);
    walk(start, 0, max_depth, include_globs, exclude_globs, &mut tree, &mut file_count, &mut dir_count);

    (tree, file_count, dir_count)
}

/// Real remote walk over SSH using ssh2 + SFTP.
/// Only compiled when the "ssh" feature is enabled (see Cargo.toml).
#[cfg(feature = "ssh")]
fn walk_ssh_directory(
    remote_path: &str,
    host: &str,
    user: &str,
    key_path: Option<&str>,
    max_depth: usize,
    include_globs: &Option<Vec<String>>,
    exclude_globs: &Option<Vec<String>>,
) -> Result<(Vec<serde_json::Value>, usize, usize), String> {
    use ssh2::Session;
    use std::net::TcpStream;

    let mut tree = Vec::new();
    let mut file_count = 0;
    let mut dir_count = 0;

    // Connect
    let tcp = TcpStream::connect(format!("{}:22", host))
        .map_err(|e| format!("TCP connect to {}: {}", host, e))?;
    let mut sess = Session::new().map_err(|e| e.to_string())?;
    sess.set_tcp_stream(tcp);
    sess.handshake().map_err(|e| e.to_string())?;

    // Auth
    if let Some(key) = key_path {
        sess.userauth_pubkey_file(user, None, std::path::Path::new(key), None)
            .map_err(|e| format!("SSH key auth failed: {}", e))?;
    } else {
        // Try agent first (works well on Linux/macOS and Windows with Pageant)
        if sess.userauth_agent(user).is_err() {
            return Err("SSH authentication failed (tried agent). Set a key_path or ensure ssh-agent/Pageant is running with your key.".into());
        }
    }

    if !sess.authenticated() {
        return Err("SSH authentication failed".into());
    }

    let sftp = sess.sftp().map_err(|e| e.to_string())?;

    fn walk_remote(
        sftp: &ssh2::Sftp,
        path: &str,
        depth: usize,
        max_depth: usize,
        include: &Option<Vec<String>>,
        exclude: &Option<Vec<String>>,
        tree: &mut Vec<serde_json::Value>,
        fc: &mut usize,
        dc: &mut usize,
    ) -> Result<(), String> {
        if depth > max_depth { return Ok(()); }

        let entries = sftp.readdir(std::path::Path::new(path))
            .map_err(|e| format!("readdir {}: {}", path, e))?;

        for (p, stat) in entries {
            let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
            let rel = p.strip_prefix(path).unwrap_or(&p).to_string_lossy().to_string();

            if let Some(ex) = exclude {
                if ex.iter().any(|pat| rel.contains(pat.trim_matches('*'))) { continue; }
            }
            if let Some(inc) = include {
                if !inc.is_empty() && !inc.iter().any(|pat| rel.contains(pat.trim_matches('*'))) {
                    continue;
                }
            }

            if stat.is_dir() {
                *dc += 1;
                let mut children = vec![];
                let _ = walk_remote(sftp, &p.to_string_lossy(), depth + 1, max_depth, include, exclude, &mut children, fc, dc);
                tree.push(serde_json::json!({
                    "type": "directory",
                    "name": name,
                    "path": rel,
                    "children": children
                }));
            } else {
                *fc += 1;
                tree.push(serde_json::json!({
                    "type": "file",
                    "name": name,
                    "path": rel,
                    "size": stat.size.unwrap_or(0)
                }));
            }
        }
        Ok(())
    }

    walk_remote(&sftp, remote_path, 0, max_depth, include_globs, exclude_globs, &mut tree, &mut file_count, &mut dir_count)?;
    Ok((tree, file_count, dir_count))
}

/// Fallback when SSH feature is disabled
#[cfg(not(feature = "ssh"))]
fn walk_ssh_directory(
    _remote_path: &str,
    _host: &str,
    _user: &str,
    _key_path: Option<&str>,
    _max_depth: usize,
    _include_globs: &Option<Vec<String>>,
    _exclude_globs: &Option<Vec<String>>,
) -> Result<(Vec<serde_json::Value>, usize, usize), String> {
    Err("SSH support not compiled in. Rebuild with: cargo build --features ssh".to_string())
}

/// Tool wrapper: scan_projects
pub fn scan_projects(args: &Value) -> String {
    let config_str = args.get("config_toml")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let project_name = args.get("project").and_then(|v| v.as_str());

    let scanner_cfg = ProjectScannerConfig::load_from_toml(config_str);
    let projects = ProjectScannerConfig::load_projects(config_str);

    if !scanner_cfg.is_enabled() && projects.is_empty() {
        return serde_json::json!({
            "error": "Project scanner not enabled in config. Add [helix.project_scanner] enabled = true"
        }).to_string();
    }

    let results: Vec<_> = if let Some(name) = project_name {
        if name == "all" || name == "*" {
            projects.iter().map(|p| scan_project(p, &scanner_cfg)).collect()
        } else {
            projects.iter()
                .filter(|p| p.name == name)
                .map(|p| scan_project(p, &scanner_cfg))
                .collect()
        }
    } else if !projects.is_empty() {
        vec![scan_project(&projects[0], &scanner_cfg)]
    } else {
        vec![]
    };

    let output = if results.len() == 1 {
        serde_json::to_value(&results[0]).unwrap_or_default()
    } else {
        serde_json::to_value(&results).unwrap_or_default()
    };

    serde_json::to_string_pretty(&output).unwrap_or_else(|_| "Failed to serialize scan results".to_string())
}

/// === Task 172: Mermaid Project Map Enhancer ===
pub fn generate_mermaid_from_scan_result(scan_result: &ProjectScanResult, title: Option<&str>) -> String {
    let title = title.unwrap_or(&scan_result.project_name);
    let mut mermaid = format!(
        "```mermaid\ngraph TD\n    subgraph \"{}\" [{} on {} (scanned from {})]\n",
        title, scan_result.project_name, scan_result.target_machine, scan_result.scanned_from
    );

    let mut node_id = 0u32;

    fn add_node(
        node: &Value,
        parent_id: Option<&str>,
        mermaid: &mut String,
        node_id: &mut u32,
    ) {
        let name = node["name"].as_str().unwrap_or("?");
        let typ = node["type"].as_str().unwrap_or("file");

        *node_id += 1;
        let nid = format!("n{}", node_id);

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
                    add_node(child, Some(&nid), mermaid, node_id);
                }
            }
        }
    }

    mermaid.push_str(&format!(
        "        root[\"{}<br/>target: {}<br/>from: {}<br/>protocol: {}<br/>at: {}\"]\n",
        scan_result.base_path,
        scan_result.target_machine,
        scan_result.scanned_from,
        scan_result.protocol,
        scan_result.scanned_at
    ));

    for item in &scan_result.tree {
        add_node(item, Some("root"), &mut mermaid, &mut node_id);
    }

    mermaid.push_str("    end\n");
    mermaid.push_str("\n    classDef dir fill:#e3f2fd,stroke:#1565c0\n");
    mermaid.push_str("    classDef file fill:#f1f8e9,stroke:#2e7d32\n");
    mermaid.push_str("```");

    let knowledge = serde_json::json!({
        "type": "okf_knowledge",
        "content_type": "project-map",
        "id": format!("project-map-{}-{}", scan_result.project_name, Utc::now().format("%Y%m%d-%H%M%S")),
        "title": title,
        "diagram_type": "mermaid",
        "source_machine": scan_result.target_machine,
        "scanned_from": scan_result.scanned_from,
        "base_path": scan_result.base_path,
        "detected_at": scan_result.scanned_at,
        "protocol": scan_result.protocol,
        "mermaid": mermaid,
        "scan_summary": {
            "files": scan_result.file_count,
            "dirs": scan_result.dir_count,
            "depth": scan_result.max_depth
        },
        "metadata": scan_result.metadata
    });

    serde_json::to_string_pretty(&knowledge).unwrap_or(mermaid)
}

pub fn generate_project_map_from_scan(args: &Value) -> String {
    let scan_json = if let Some(s) = args.get("scan_json").and_then(|v| v.as_str()) {
        s.to_string()
    } else {
        args.to_string()
    };

    let scan: ProjectScanResult = match serde_json::from_str(&scan_json) {
        Ok(s) => s,
        Err(_) => {
            if let Ok(val) = serde_json::from_str::<Value>(&scan_json) {
                if let Ok(s) = serde_json::from_value(val.clone()) {
                    s
                } else {
                    return "Error: could not parse scan result. Run scan_projects first.".to_string();
                }
            } else {
                return "Error parsing scan input".to_string();
            }
        }
    };

    let title = args.get("title").and_then(|v| v.as_str());
    generate_mermaid_from_scan_result(&scan, title)
}

// ── Tests for today's work (Task 171/172 + optional SSH feature) ─────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_load_scanner_config_defaults() {
        let cfg = ProjectScannerConfig::load_from_toml("");
        assert!(!cfg.is_enabled());
        assert_eq!(cfg.default_max_depth(), 5);
        assert_eq!(cfg.default_protocol(), "ssh");
        assert_eq!(cfg.scanned_from(), "dell630");
    }

    #[test]
    fn test_load_scanner_config_from_toml() {
        let toml = r#"
[helix.project_scanner]
enabled = true
default_max_depth = 3
default_protocol = "local"
scanned_from = "test-machine"
"#;
        let cfg = ProjectScannerConfig::load_from_toml(toml);
        assert!(cfg.is_enabled());
        assert_eq!(cfg.default_max_depth(), 3);
        assert_eq!(cfg.default_protocol(), "local");
        assert_eq!(cfg.scanned_from(), "test-machine");
    }

    #[test]
    fn test_load_projects_from_toml() {
        let toml = r#"
[[helix.projects]]
name = "demo"
target_machine = "main-pc"
remote_path = "/home/user/demo"
protocol = "local"
max_depth = 2
description = "Test project"
include_globs = ["*.rs", "*.toml"]
exclude_globs = ["target/**"]
"#;
        let projects = ProjectScannerConfig::load_projects(toml);
        assert_eq!(projects.len(), 1);
        let p = &projects[0];
        assert_eq!(p.name, "demo");
        assert_eq!(p.target_machine, "main-pc");
        assert_eq!(p.effective_protocol("ssh"), "local");
        assert_eq!(p.effective_max_depth(10), 2);
        assert!(p.include_globs.as_ref().unwrap().contains(&"*.rs".to_string()));
    }

    #[test]
    fn test_load_projects_empty_when_no_section() {
        let projects = ProjectScannerConfig::load_projects("");
        assert!(projects.is_empty());
    }

    #[test]
    fn test_scan_projects_returns_error_when_not_enabled() {
        let result = scan_projects(&json!({}));
        assert!(result.contains("Project scanner not enabled"));
    }

    #[test]
    fn test_scan_projects_local_directory() {
        // Use a minimal local scan (current dir, depth 1) — always safe
        let toml = r#"
[[helix.projects]]
name = "current"
target_machine = "localhost"
remote_path = "."
protocol = "local"
max_depth = 1
"#;

        let result_str = scan_projects(&json!({
            "config_toml": toml,
            "project": "current"
        }));

        // Should be valid JSON and contain the project name
        let parsed: serde_json::Value = serde_json::from_str(&result_str)
            .expect("scan_projects should return valid JSON");
        assert_eq!(parsed["project_name"], "current");
        assert!(parsed["file_count"].as_u64().is_some());
        assert!(parsed["tree"].is_array());
    }

    #[test]
    fn test_scan_projects_all_projects() {
        let toml = r#"
[[helix.projects]]
name = "proj1"
target_machine = "localhost"
remote_path = "."
protocol = "local"
max_depth = 0

[[helix.projects]]
name = "proj2"
target_machine = "localhost"
remote_path = "src"
protocol = "local"
max_depth = 0
"#;

        let result_str = scan_projects(&json!({
            "config_toml": toml,
            "project": "all"
        }));

        let parsed: serde_json::Value = serde_json::from_str(&result_str)
            .expect("should return array of results");
        assert!(parsed.is_array());
        let arr = parsed.as_array().unwrap();
        assert!(arr.len() >= 2);
    }

    #[test]
    fn test_generate_mermaid_from_scan_result() {
        let scan = ProjectScanResult {
            project_name: "unit-test-proj".to_string(),
            target_machine: "ci-runner".to_string(),
            scanned_from: "github".to_string(),
            protocol: "local".to_string(),
            base_path: ".".to_string(),
            scanned_at: "2025-01-01T00:00:00Z".to_string(),
            file_count: 42,
            dir_count: 7,
            max_depth: 2,
            tree: vec![
                json!({"type": "directory", "name": "src", "path": "src", "children": []}),
                json!({"type": "file", "name": "Cargo.toml", "path": "Cargo.toml", "size": 1234})
            ],
            metadata: json!({"source": "test"}),
        };

        let mermaid = generate_mermaid_from_scan_result(&scan, Some("Test Diagram"));
        assert!(mermaid.contains("graph TD"));
        assert!(mermaid.contains("unit-test-proj"));
        assert!(mermaid.contains("ci-runner"));
        assert!(mermaid.contains("src"));
        assert!(mermaid.contains("Cargo.toml"));
    }

    #[test]
    fn test_generate_project_map_from_scan_tool_wrapper() {
        let scan = ProjectScanResult {
            project_name: "wrapper-test".to_string(),
            target_machine: "test".to_string(),
            scanned_from: "test".to_string(),
            protocol: "local".to_string(),
            base_path: ".".to_string(),
            scanned_at: "now".to_string(),
            file_count: 0,
            dir_count: 0,
            max_depth: 1,
            tree: vec![],
            metadata: json!({}),
        };

        let args = json!({
            "scan_json": serde_json::to_string(&scan).unwrap(),
            "title": "Wrapper Test"
        });

        let output = generate_project_map_from_scan(&args);
        assert!(output.contains("mermaid") || output.contains("graph TD"));
        assert!(output.contains("Wrapper Test"));
    }

    // This test only runs when the ssh feature is NOT enabled (the normal default case).
    // It verifies the graceful fallback we added today.
    #[test]
    #[cfg(not(feature = "ssh"))]
    fn test_ssh_walk_fallback_when_feature_disabled() {
        let result = walk_ssh_directory(
            "/remote/path",
            "example-host",
            "user",
            None,
            3,
            &None,
            &None,
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("not compiled in") || err.contains("features ssh"),
            "Unexpected error message: {}",
            err
        );
    }

    #[test]
    fn test_project_scan_result_is_serializable() {
        let scan = ProjectScanResult {
            project_name: "serde-test".into(),
            target_machine: "machine".into(),
            scanned_from: "here".into(),
            protocol: "local".into(),
            base_path: ".".into(),
            scanned_at: "ts".into(),
            file_count: 1,
            dir_count: 1,
            max_depth: 1,
            tree: vec![],
            metadata: json!({"ok": true}),
        };

        let json_str = serde_json::to_string(&scan).expect("ProjectScanResult should serialize");
        assert!(json_str.contains("serde-test"));
        let _ : ProjectScanResult = serde_json::from_str(&json_str).expect("should roundtrip");
    }
}
