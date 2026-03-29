// cpu/instructions.rs

use crate::Message;
// No executable code here, just type definitions. No logging added.

#[derive(Debug, Clone)]
pub enum CpuEventSource {
    IoTerminal,
    IoWeb,
    Cron,
    A2A,
    Mcp,
    Internal,
}

#[derive(Debug, Clone)]
pub enum CpuEventKind {
    UserMessage {
        text: String,
    },
    ToolResult {
        tool: String,
        payload: serde_json::Value,
    },
    CronTick {
        job_id: String,
    },
    A2AMessage {
        from: String,
        payload: serde_json::Value,
    },
    McpRequest {
        capability: String,
        payload: serde_json::Value,
    },
    BusMessage {
        payload: serde_json::Value,
    },
    InternalPlanReady,
    InternalReflectionNeeded,
}

#[derive(Debug, Clone)]
pub struct CpuEvent {
    pub id: String,
    pub source: CpuEventSource,
    pub kind: CpuEventKind,
    pub received_at: std::time::Instant,
}

impl CpuEvent {
    pub fn from_message(msg: &Message) -> Self {
        CpuEvent {
            id: msg.timestamp.to_string(),
            source: CpuEventSource::Internal,
            kind: CpuEventKind::BusMessage {
                payload: serde_json::Value::String(msg.data.clone()),
            },
            received_at: std::time::Instant::now(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Instruction {
    ReadMemory {
        key: String,
    },
    WriteMemory {
        key: String,
        value: serde_json::Value,
    },
    RunSkill {
        name: String,
        args: serde_json::Value,
    },
    ExecuteHooks {
        phase: String,
    },
    EmitBusEvent {
        topic: String,
        payload: serde_json::Value,
    },
    PlanNextSteps,
    ReflectOnLastStep,
    UpdateBelief {
        key: String,
        value: serde_json::Value,
    },
    WaitForEvent,
}
