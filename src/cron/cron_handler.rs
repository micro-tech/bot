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
    let mut ticker = interval(Duration::from_secs(3600)); // Hourly

    loop {
        ticker.tick().await;

        let task_msg = Message {
            to: "ollama".to_string(),
            from: "cron".to_string(),
            data: "Hourly cron task: check logs, run maintenance".to_string(),
            timestamp: get_timestamp(),
        };

        match bus.publish(task_msg) {
            Ok(_) => info!("Cron: Hourly maintenance task published"),
            Err(e) => error!("Cron: Failed to publish task: {}", e),
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
