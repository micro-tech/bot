//! Metrics export for CLI usage (Task 117)
use crate::utils::log_to_file;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};
use serde_json::json;

fn expand_path() -> String {
    if let Ok(home) = std::env::var("HOME") {
        format!("{}/.bot/metrics.json", home)
    } else if let Ok(userprofile) = std::env::var("USERPROFILE") {
        format!("{}/.bot/metrics.json", userprofile)
    } else {
        "metrics.json".to_string()
    }
}

pub fn record_command(cmd: &str, duration_ms: u64, success: bool) {
    let path = expand_path();
    if let Some(parent) = std::path::Path::new(&path).parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            log_to_file(&format!("Failed to create metrics dir: {}", e));
        }
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let entry = json!({
        "ts": timestamp,
        "cmd": cmd,
        "duration_ms": duration_ms,
        "success": success
    });

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        if let Err(e) = writeln!(file, "{}", entry) {
            log_to_file(&format!("Failed to write metrics: {}", e));
        }
    } else {
        log_to_file("Failed to open metrics file for writing");
    }
}

pub fn export_metrics() -> String {
    let path = expand_path();
    match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) => {
            log_to_file(&format!("Failed to read metrics file {}: {}", path, e));
            "[]".to_string()
        }
    }
}
