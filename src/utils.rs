// Utility functions for Agent OS

use chrono::Local;
use log::error;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Returns a cross-platform path for the error log.
/// Windows: %APPDATA%\bot\logs\error_log.md
/// Linux/macOS: ~/.bot/logs/error_log.md
fn error_log_path() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
    } else {
        std::env::var("HOME")
            .map(|h| PathBuf::from(h).join(".bot"))
            .unwrap_or_else(|_| PathBuf::from(".bot"))
    };

    let log_dir = if cfg!(windows) {
        base.join("bot").join("logs")
    } else {
        base.join("logs")
    };

    // Ensure directory exists
    if let Err(e) = fs::create_dir_all(&log_dir) {
        // Can't log this yet, but we tried
        eprintln!("Failed to create log dir {:?}: {}", log_dir, e);
    }

    log_dir.join("error_log.md")
}

/// Logs a message to the error log file with a timestamp.
/// The log is human-readable markdown format.
pub fn log_to_file(message: &str) {
    let timestamp = Local::now().format("%Y-%m-%d %I:%M:%S %p").to_string();
    let log_entry = format!(
        "[{}] {}
",
        timestamp, message
    );

    let path = error_log_path();

    match OpenOptions::new().append(true).create(true).open(&path) {
        Ok(mut file) => {
            if let Err(e) = file.write_all(log_entry.as_bytes()) {
                error!("Failed to write to error log file {:?}: {}", path, e);
            }
        }
        Err(e) => {
            error!("Failed to open error log file {:?}: {}", path, e);
        }
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
