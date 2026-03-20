use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use serde_json;
use std::fs;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{Duration as TokioDuration, interval};

// Include modules
mod io;
mod bus;
mod utils;
mod bayesian;

use crate::bus::{Bus, Message};
use crate::io::web_server::start_web_server;
use utils::log_to_file;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Heartbeat {
    timestamp: u64,
    system_status: String,
    recent_events: Vec<String>,
}

fn get_current_timestamp() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis() as u64,
        Err(_) => {
            log_to_file(&"Failed to get system time".to_string());
            error!("Failed to get system time");
            0
        }
    }
}

async fn send_heartbeat(bus: Arc<Bus>) -> Result<(), String> {
    let heartbeat = Heartbeat {
        timestamp: get_current_timestamp(),
        system_status: "Operational".to_string(),
        recent_events: vec!["System check completed".to_string()],
    };

    info!("Sending heartbeat: {:?}", heartbeat);

    let heartbeat_json = match serde_json::to_string(&heartbeat) {
        Ok(json) => json,
        Err(e) => {
            let error_msg = format!("Failed to serialize heartbeat to JSON: {}", e);
            log_to_file(&error_msg);
            error!("{}", error_msg);
            return Err(error_msg);
        }
    };

    let message = Message {
        to: "ollama".to_string(),
        from: "heartbeat".to_string(),
        data: heartbeat_json,
        timestamp: get_current_timestamp(),
    };
    bus.publish(message);
    info!("Heartbeat published to bus");

    // Publish live log update to web
    let log_data = format!(r#"{{"type":"log","level":"info","msg":"Heartbeat sent to Ollama"}}"#);
    let log_msg = Message {
        to: "web_interface".to_string(),
        from: "logger".to_string(),
        data: log_data,
        timestamp: get_current_timestamp(),
    };
    bus.publish(log_msg);

    Ok(())
}

#[tokio::main]
async fn main() {
    env_logger::init();
    fs::create_dir_all("logs").expect("Failed to create logs dir");
    info!("Starting bot with HTTPS web interface and Ollama chat");

    let bus = Arc::new(Bus::new());

    // Spawn HTTPS Web Server
    let web_bus = bus.clone();
    tokio::spawn(async move {
        if let Err(e) = start_web_server(web_bus).await {
            error!("Web server failed: {}", e);
        }
    });
    info!("HTTPS Web Server spawned - visit https://localhost:8443 (accept self-signed cert warning)");

    // Spawn Ollama Handler (SIMPLIFIED SYNC VERSION)
    let ollama_bus = bus.clone();
    tokio::spawn(async move {
        let rx = ollama_bus.subscribe("ollama");
        tokio::task::spawn_blocking(move || {
            while let Ok(msg) = rx.recv() {
                println!("Ollama message received: {:?}", msg);

                // Simple echo response for now
                if let Ok(resp) = serde_json::to_string(&format!("Processed: {}", msg.data)) {
                    let reply = Message {
                        to: "web_interface".to_string(),
                        from: "ollama".to_string(),
                        data: resp,
                        timestamp: get_current_timestamp(),
                    };
                    ollama_bus.publish(reply);
                }
            }
        });
    });
    info!("Ollama handler spawned");

    // Heartbeat loop
    let mut interval = interval(TokioDuration::from_secs(60));
    let mut consecutive_failures = 0;
    let max_consecutive_failures = 5;

    loop {
        interval.tick().await;
        match send_heartbeat(bus.clone()).await {
            Ok(()) => {
                info!("Heartbeat sent successfully");
                consecutive_failures = 0;
            }
            Err(e) => {
                let error_msg = format!("Failed to send heartbeat: {}", e);
                log_to_file(&error_msg);
                error!("{}", error_msg);
                consecutive_failures += 1;
                warn!(
                    "Consecutive failures: {}/{}",
                    consecutive_failures, max_consecutive_failures
                );
                if consecutive_failures >= max_consecutive_failures {
                    warn!("Maximum consecutive failures reached. Consider taking corrective action.");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_current_timestamp() {
        let timestamp = get_current_timestamp();
        assert_ne!(timestamp, 0, "Timestamp should not be zero unless there's an error");
    }

    #[test]
    fn test_heartbeat_struct_creation() {
        let heartbeat = Heartbeat {
            timestamp: 1234567890,
            system_status: "Operational".to_string(),
            recent_events: vec!["Test event".to_string()],
        };
        assert_eq!(heartbeat.timestamp, 1234567890);
        assert_eq!(heartbeat.system_status, "Operational");
        assert_eq!(heartbeat.recent_events, vec!["Test event".to_string()]);
    }

    #[test]
    fn test_heartbeat_serialization() {
        let heartbeat = Heartbeat {
            timestamp: 1234567890,
            system_status: "Operational".to_string(),
            recent_events: vec!["Test event".to_string()],
        };
        let json_result = serde_json::to_string(&heartbeat);
        assert!(json_result.is_ok(), "Heartbeat should serialize to JSON without error");
        let json = json_result.unwrap();
        assert!(json.contains("1234567890"), "JSON should contain the timestamp");
        assert!(json.contains("Operational"), "JSON should contain the system status");
        assert!(json.contains("Test event"), "JSON should contain the recent events");
    }
}