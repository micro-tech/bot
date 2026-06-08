//! Plugin system for CLI (Task 118)
//! Discovers executables in ~/.bot/plugins/

use crate::utils::log_to_file;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

pub fn plugin_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(format!("{}/.bot/plugins", home))
    } else if let Ok(userprofile) = std::env::var("USERPROFILE") {
        PathBuf::from(format!("{}/.bot/plugins", userprofile))
    } else {
        PathBuf::from("./plugins")
    }
}

pub fn discover_plugins() -> Vec<String> {
    let dir = plugin_dir();
    let mut plugins = Vec::new();

    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Ok(name) = entry.file_name().into_string() {
                plugins.push(name);
            }
        }
    } else {
        // Directory may not exist yet - not an error
    }
    plugins
}

pub fn execute_plugin(name: &str, args: &[String]) -> Result<String, String> {
    let mut path = plugin_dir();
    path.push(name);

    if !path.exists() {
        let msg = format!("Plugin not found: {}", name);
        log_to_file(&msg);
        return Err(msg);
    }

    match Command::new(&path).args(args).output() {
        Ok(output) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                log_to_file(&format!("Plugin {} failed: {}", name, stderr));
            }
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        }
        Err(e) => {
            let msg = format!("Failed to execute plugin {}: {}", name, e);
            log_to_file(&msg);
            Err(msg)
        }
    }
}
