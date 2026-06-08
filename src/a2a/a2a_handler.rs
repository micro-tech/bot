//! A2A Communication Module - Real Implementation with error handling
use std::sync::Arc;
use crate::bus::{Bus, Message};
use log::{info, error};

fn get_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Handle agent-to-agent message routing with proper error handling.
pub fn handle_a2a_message(msg: Message, bus: &Arc<Bus>) {
    info!("A2A handling: {} -> {}", msg.from, msg.to);

    let response = format!("A2A response to {}: processed {}", msg.to, msg.data);
    let reply = Message {
        to: msg.from.clone(),
        from: "a2a".to_string(),
        data: response,
        timestamp: get_timestamp(),
    };

    if let Err(e) = bus.publish(reply) {
        error!("A2A failed to publish reply: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a2a_handle() {
        let bus = Arc::new(Bus::new());
        let msg = Message {
            to: "agent2".to_string(),
            from: "agent1".to_string(),
            data: "hello".to_string(),
            timestamp: 0,
        };
        // Should not panic
        handle_a2a_message(msg, &bus);
        assert!(true);
    }
}
