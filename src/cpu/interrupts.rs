use crate::bus::Message;
use crate::cpu::instructions::{CpuEvent, CpuEventKind, CpuEventSource};
use crate::utils::log_to_file;
use log::debug;
use std::time::Instant;

pub struct Interrupts;

impl Interrupts {
    pub async fn poll_next_event(bus: &crate::bus::BusHandle) -> Option<CpuEvent> {
        debug!("Starting to poll next event from bus...");  // Added logging
        log_to_file("Starting to poll next event from bus...");
        
        if let Ok(msg) = bus.try_recv() {
            debug!("Received bus message: {:?}", msg);  // Existing logging
            log_to_file(&format!("Received bus message: {:?}", msg));
            Some(Self::from_bus_message(msg))  // Return Some on success
        } else {
            debug!("No event polled after checking or error occurred");  // Added logging for error case
            log_to_file("No event polled after checking or error occurred");
            None  // Return None on error or no event
        }
    }

    fn from_bus_message(msg: Message) -> CpuEvent {
        CpuEvent {
            id: uuid::Uuid::new_v4().to_string(),
            source: CpuEventSource::Internal,
            kind: CpuEventKind::ToolResult {
                tool: msg.from,
                payload: serde_json::json!({ "to": msg.to, "data": msg.data }),
            },
            received_at: Instant::now(),
        }
    }
}
