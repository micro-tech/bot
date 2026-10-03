use std::env;
use std::fs;
use std::path::Path;
use std::time::SystemTime;

fn main() {
    // Inject a build timestamp so the installer can print it at runtime.
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    println!("cargo:rustc-env=BUILD_TIMESTAMP={}", timestamp);

    // Re-run whenever the installer source or either embedded config changes.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/bin/installer.rs");
    println!("cargo:rerun-if-changed=config.toml");
    println!("cargo:rerun-if-changed=system_manifest.md");

    // === Project Intelligence / Diagram freshness check ===
    // This helps remind developers to update the hybrid mermaid diagrams
    // when the source structure changes significantly.
    check_diagram_freshness();
}

fn check_diagram_freshness() {
    let src_path = Path::new("src");
    let diagrams_path = Path::new(".grok/diagrams");

    if !src_path.exists() || !diagrams_path.exists() {
        return;
    }

    // Get the newest modification time in src/
    let src_newest = find_newest_mtime(src_path);

    // Get the modification time of the main hybrid diagram
    let main_diagram = diagrams_path.join("hybrid_project_map.mmd");
    let diagram_mtime = fs::metadata(&main_diagram)
        .and_then(|m| m.modified())
        .ok();

    if let (Some(src_time), Some(diag_time)) = (src_newest, diagram_mtime) {
        if src_time > diag_time {
            println!(
                "cargo:warning=src/ has been modified more recently than .grok/diagrams/hybrid_project_map.mmd"
            );
            println!(
                "cargo:warning=Consider regenerating the hybrid mermaid diagrams if major modules were added/removed."
            );
            println!(
                "cargo:warning=See .grok/diagrams/ for current diagrams and regeneration guidance."
            );
        }
    }
}

fn find_newest_mtime(dir: &Path) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                if let Some(sub_newest) = find_newest_mtime(&path) {
                    newest = Some(newest.map_or(sub_newest, |t| t.max(sub_newest)));
                }
            } else if let Ok(metadata) = fs::metadata(&path) {
                if let Ok(mtime) = metadata.modified() {
                    newest = Some(newest.map_or(mtime, |t| t.max(mtime)));
                }
            }
        }
    }

    newest
}