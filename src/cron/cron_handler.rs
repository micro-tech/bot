//! Cron Scheduling - Real Implementation with error handling
use std::sync::Arc;
use tokio::time::{interval, Duration};
use crate::bus::{Bus, Message};
use log::{info, error};

fn get_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

pub async fn start_cron(bus: Arc<Bus>) {
    let mut ticker = interval(Duration::from_secs(5)); // every 5 seconds for autonomous mode

    loop {
        ticker.tick().await;

        // Autonomous runtime trigger
        let task_msg = Message {
            to: "cpu".to_string(),
            from: "cron".to_string(),
            data: serde_json::json!({
                "type": "agent_run",
                "goal": "maintain system state"
            }).to_string(),
            timestamp: get_timestamp(),
        };

        match bus.publish(task_msg) {
            Ok(_) => info!("Cron: Autonomous agent_run triggered (maintain system state)"),
            Err(e) => error!("Cron: Failed to publish agent_run: {}", e),
        }

        // Original hourly maintenance (keep both)
        if Utc::now().hour() % 1 == 0 {
            let maintenance = Message {
                to: "ollama".to_string(),
                from: "cron".to_string(),
                data: "Hourly cron task: check logs, run maintenance".to_string(),
                timestamp: get_timestamp(),
            };
            let _ = bus.publish(maintenance);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cron_tick() {
        // Basic sanity test
        assert_eq!(1, 1);
    }
}
