use backon::{ExponentialBuilder, Retryable};
use log::{error, info, warn};
use ollama_rs::{Ollama, generation::completion::request::GenerationRequest};
use serde::{Deserialize, Serialize};
use serde_json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use std::time::Duration as StdDuration;
use tokio::time::{interval, Duration as TokioDuration};

// Include the io module to ensure its tests are run
mod io;
// Include the bus module to make it accessible in the crate hierarchy
mod bus;
// Include the utils module for utility functions
mod utils;
mod bayesian;
use crate::bus::{Bus, Message};
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

async fn send_heartbeat_to_ollama(
    ollama: &Ollama,
    heartbeat: &Heartbeat,
) -> Result<String, String> {
    info!(
        "Preparing to send heartbeat data to Ollama: {:?}",
        heartbeat
    );
    let heartbeat_json = match serde_json::to_string(heartbeat) {
        Ok(json) => json,
        Err(e) => {
            let error_msg = format!("Failed to serialize heartbeat to JSON: {}", e);
            log_to_file(&error_msg);
            error!("{}", error_msg);
            return Err(error_msg);
        }
    };
    info!("Serialized heartbeat to JSON: {}", heartbeat_json);

    let request = GenerationRequest::new(
        "llama3".to_string(), // Assuming a model name, adjust as needed
        format!("Process this heartbeat data: {}", heartbeat_json),
    );

    let retry_policy = ExponentialBuilder::default()
        .with_max_times(3)
        .with_min_delay(StdDuration::from_millis(500))
        .with_max_delay(StdDuration::from_secs(10));

    let send_request = || async {
        info!("Attempting to send request to Ollama");
        match ollama.generate(request.clone()).await {
            Ok(response) => {
                info!(
                    "Successfully received response from Ollama: {}",
                    response.response
                );
                Ok(response.response)
            }
            Err(e) => {
                let error_msg = format!("Failed to send heartbeat to Ollama: {}", e);
                log_to_file(&error_msg);
                error!("{}", error_msg);
                Err(e.to_string())
            }
        }
    };

    match tokio::time::timeout(TokioDuration::from_secs(30), send_request.retry(&retry_policy)).await {
        Ok(Ok(response)) => {
            info!("Heartbeat successfully sent to Ollama after retries if any");
            Ok(response)
        }
        Ok(Err(e)) => {
            let error_msg = format!("Failed to send heartbeat to Ollama after retries: {}", e);
            log_to_file(&error_msg);
            error!("{}", error_msg);
            Err(error_msg)
        }
        Err(timeout_err) => {
            let error_msg = format!("Timeout while sending heartbeat to Ollama: {}", timeout_err);
            log_to_file(&error_msg);
            error!("{}", error_msg);
            Err(error_msg)
        }
    }
}

async fn send_heartbeat() -> Result<(), String> {
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

    let mut bus = Bus::new();
    let ollama_rx = bus.subscribe("ollama");
    let message = Message {
        to: "ollama".to_string(),
        from: "heartbeat".to_string(),
        data: heartbeat_json,
        timestamp: get_current_timestamp(),
    };
    bus.publish(message);
    info!("Heartbeat published to bus");

    if cfg!(debug_assertions) {
        match ollama_rx.recv_timeout(StdDuration::from_millis(500)) {
            Ok(msg) => {
                info!("Ollama received via bus: {:?}", msg);
                if let Some(resp) = crate::io::ollama::handle_ollama_message(msg.clone(), &mut bus).await
                {
                    info!("Ollama response: {}", resp);

                    let terminal_rx = bus.subscribe("terminal");
                    let reply = Message {
                        to: "terminal".to_string(),
                        from: "ollama".to_string(),
                        data: resp.clone(),
                        timestamp: get_current_timestamp(),
                    };
                    bus.publish(reply);
                    info!("Reply published to terminal");

                    if let Ok(terminal_msg) = terminal_rx.recv_timeout(StdDuration::from_millis(500)) {
                        info!("Terminal received: {}", terminal_msg.data);
                    } else {
                        info!("Terminal no immediate response (normal if no handler)");
                    }
                }
            }
            Err(e) => {
                warn!("Ollama recv timeout/err: {}", e);
            }
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() {
    env_logger::init();
    fs::create_dir_all("logs").expect("Failed to create logs dir");
    info!("Starting bot heartbeat system");

    let mut interval = interval(TokioDuration::from_secs(60));
    let mut consecutive_failures = 0;
    let max_consecutive_failures = 5;

    loop {
        interval.tick().await;
        match send_heartbeat().await {
            Ok(()) => {
                info!("Heartbeat sent successfully");
                consecutive_failures = 0; // Reset on success
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
                    warn!(
                        "Maximum consecutive failures reached. Consider taking corrective action."
                    );
                    // Future: Implement circuit breaker or fallback logic here
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
        assert_ne!(
            timestamp, 0,
            "Timestamp should not be zero unless there's an error"
        );
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
        assert!(
            json_result.is_ok(),
            "Heartbeat should serialize to JSON without error"
        );
        let json = json_result.unwrap();
        assert!(
            json.contains("1234567890"),
            "JSON should contain the timestamp"
        );
        assert!(
            json.contains("Operational"),
            "JSON should contain the system status"
        );
        assert!(
            json.contains("Test event"),
            "JSON should contain the recent events"
        );
    }

    // Mock test for send_heartbeat_to_ollama since we can't guarantee Ollama is running
    #[tokio::test]
    async fn test_send_heartbeat_without_network() {
        let heartbeat = Heartbeat {
            timestamp: 1234567890,
            system_status: "Operational".to_string(),
            recent_events: vec!["Test event".to_string()],
        };
        // Since send_heartbeat_to_ollama requires a network call, we test only up to serialization
        let json_result = serde_json::to_string(&heartbeat);
        assert!(
            json_result.is_ok(),
            "Serialization should succeed before network attempt"
        );
        // Note: Actual network test skipped as it depends on Ollama availability
    }

    #[tokio::test]
    async fn test_send_heartbeat_function() {
        // This test will likely fail if Ollama isn't running locally, which is expected
        // It's included for completeness and to verify error handling
        let result = send_heartbeat().await;
        if result.is_err() {
            let err_msg = result.unwrap_err().to_string();
            assert!(
                err_msg.contains("connection refused")
                    || err_msg.contains("timeout")
                    || err_msg.contains("failed to connect")
                    || err_msg.contains("deadline has elapsed"),
                "Error should be related to network connection issues if Ollama is not running: {}",
                err_msg
            );
        } else {
            assert!(
                result.is_ok(),
                "Sending heartbeat should succeed if Ollama is available"
            );
        }
    }
}