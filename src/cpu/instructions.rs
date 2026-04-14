// cpu/instructions.rs

use crate::Message;
use crate::io::ollama::LlmTarget;

/// Enum representing the source of a CPU event.
/// Suggestions for usage: Log events based on source when processing to track origins.
#[derive(Debug, Clone)]
pub enum CpuEventSource {
    IoTerminal,
    IoWeb,
    Cron,
    A2A,
    Mcp,
    Internal,
}

/// Enum representing the kind of CPU event.
/// Suggestions for usage: Add logging when matching on variants to record event details and handle potential errors in associated data.
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

/// Struct representing a CPU event.
/// Suggestions for usage: Log creation and processing of events, and add error checking when deserializing or handling payloads.
#[derive(Debug, Clone)]
pub struct CpuEvent {
    pub id: String,
    pub source: CpuEventSource,
    pub kind: CpuEventKind,
    pub received_at: std::time::Instant,
}

impl CpuEvent {
    /// Creates a CpuEvent from a Message.
    /// Suggestions: Implement logging for incoming messages and handle potential errors, such as invalid data in Message.
    pub fn from_message(msg: &Message) -> Self {
        log::debug!("Creating CpuEvent from message: {:?}", msg); // Added logging
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

/// Enum representing instructions for the CPU.
/// Suggestions for usage: Log instructions when executed and add error checking for invalid states or data in variants.
#[derive(Debug, Clone)]
pub enum Instruction {
    // Memory operations
    ReadMemory {
        key: String,
    },
    WriteMemory {
        key: String,
        value: serde_json::Value,
    },

    // Bus operations
    EmitBusEvent {
        topic: String,
        payload: serde_json::Value,
    },

    // Belief graph updates
    UpdateBelief {
        key: String,
        value: serde_json::Value,
    },

    // Skill system
    RunSkill {
        name: String,
        args: serde_json::Value,
    },

    // Hook system
    ExecuteHooks {
        phase: String,
    },

    // Planning / workflow
    PlanNextSteps,
    ReflectOnLastStep,
    WaitForEvent,

    // LLM routing
    CallLlm {
        target: LlmTarget,
        prompt: String,
        correlation_id: u64,
    },
}
