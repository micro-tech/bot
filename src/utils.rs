// Utility functions for Agent OS

use chrono::Local;
use log::error;
use std::fs::OpenOptions;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

/// Logs a message to the error log file with a timestamp.
pub fn log_to_file(message: &str) {
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let log_entry = format!(
        "[{}] {}
",
        timestamp, message
    );

    match OpenOptions::new()
        .append(true)
        .create(true)
        .open("logs/error_log.md")
    {
        Ok(mut file) => {
            if let Err(e) = file.write_all(log_entry.as_bytes()) {
                error!("Failed to write to error log file: {}", e);
            }
        }
        Err(e) => {
            error!("Failed to open error log file: {}", e);
        }
    }
}
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
