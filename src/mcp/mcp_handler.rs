//! MCP - Master Control Program - Real Implementation
use crate::bus::{Bus, Message};
use std::sync::Arc;
use log::{info, warn};

fn get_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

pub struct MCP {
    bus: Arc<Bus>,
}

impl MCP {
    pub fn new(bus: Arc<Bus>) -> Self {
        Self { bus }
    }

    pub fn start(&self) {
        info!("MCP started - monitoring bus and prioritizing tasks");
    }

    /// Decide routing based on message content with proper error handling
    pub fn decide(&self, msg: &Message) -> Option<Message> {
        if msg.data.contains("high_prio") || msg.data.contains("urgent") {
            Some(Message {
                to: "ollama".to_string(),
                from: "mcp".to_string(),
                data: format!("MCP prioritized: {}", msg.data),
                timestamp: get_timestamp(),
            })
        } else if msg.data.contains("error") {
            warn!("MCP detected error message: {}", msg.data);
            None
        } else {
            None
        }
    }

    /// Publish a prioritized message
    pub fn publish_prioritized(&self, msg: Message) -> Result<(), String> {
        self.bus.publish(msg).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_decide_high_prio() {
        let bus = Arc::new(Bus::new());
        let mcp = MCP::new(bus);
        let msg = Message {
            to: "mcp".to_string(),
            from: "test".to_string(),
            data: "high_prio task".to_string(),
            timestamp: 0,
        };
        assert!(mcp.decide(&msg).is_some());
    }

    #[test]
    fn test_mcp_decide_normal() {
        let bus = Arc::new(Bus::new());
        let mcp = MCP::new(bus);
        let msg = Message {
            to: "mcp".to_string(),
            from: "test".to_string(),
            data: "normal task".to_string(),
            timestamp: 0,
        };
        assert!(mcp.decide(&msg).is_none());
    }
}
