// Utility functions for Agent OS

use std::fs::OpenOptions;
use std::io::Write;
use chrono::Local;
use log::error;

/// Logs a message to the error log file with a timestamp.
pub fn log_to_file(message: &str) {
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let log_entry = format!("[{}] {}
", timestamp, message);
    
    match OpenOptions::new()
        .append(true)
        .create(true)
        .open("logs/error_log.md") {
        Ok(mut file) => {
            if let Err(e) = file.write_all(log_entry.as_bytes()) {
                error!("Failed to write to error log file: {}", e);
            }
        },
        Err(e) => {
            error!("Failed to open error log file: {}", e);
        }
    }
}
