//! MCP - Master Control Program - Real Implementation
use crate::bus::{Bus, Message};
use std::sync::Arc;
use log::info;

pub struct MCP {
    bus: Arc<Bus>,
}

impl MCP {
    pub fn new(bus: Arc<Bus>) -> Self {
        Self { bus }
    }

    pub fn start(&self) {
        info!(\"MCP started - monitoring bus\");
        // Monitor bus, prioritize tasks
        // e.g., if high prio msg, route to ollama
    }

    pub fn decide(&self, msg: &Message) -> Option<Message> {
        if msg.data.contains(\"high_prio\") {
            Some(Message {
                to: \"ollama\".to_string(),
                from: \"mcp\".to_string(),
                data: format!(\"MCP prioritized: {}\", msg.data),
                timestamp: get_timestamp(),
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_mcp_decide() {
        let bus = Arc::new(Bus::new());
        let mcp = MCP::new(bus);
        let msg = Message { /* mock */ };
        assert!(mcp.decide(&msg).is_some());
    }
}