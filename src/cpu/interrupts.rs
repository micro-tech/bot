use crate::bus::Message;
use crate::cpu::instructions::{CpuEvent, CpuEventKind, CpuEventSource};
use std::time::Instant;
use log::debug;

pub struct Interrupts;

impl Interrupts {
    pub async fn poll_next_event(bus: &crate::bus::BusHandle) -> Option<CpuEvent> {
        debug!("Polling next event from bus...");
        // 1. Check bus first (highest priority)
        if let Some(msg) = bus.try_recv() {
            debug!("Received bus message: {:?}", msg);
            return Some(Self::from_bus_message(msg));
        }

        // 2. TODO: check terminal, web, cron, a2a, mcp, etc.

        debug!("No event polled");
        None
    }

    fn from_bus_message(msg: Message) -> CpuEvent {
        CpuEvent {
            id: uuid::Uuid::new_v4().to_string(),
            source: CpuEventSource::Internal,
            kind: CpuEventKind::ToolResult {
                tool: msg.from,
                payload: serde_json::json!({
                    "to": msg.to,
                    "data": msg.data,
                }),
            },
            received_at: Instant::now(),
        }
    }
}