//! Observability and debug controls for reasoning engine (Task 74)

use crate::reasoning::state::ReasoningState;
use serde_json::json;
use tracing::debug;

/// Structured log entry for reasoning events
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ReasoningLogEntry {
    pub correlation_id: String,
    pub phase: String,
    pub event: String,
    pub data: serde_json::Value,
}

/// Observability helper for reasoning engine
#[allow(dead_code)]
pub struct ReasoningObserver;

#[allow(dead_code)]
impl ReasoningObserver {
    /// Emit a structured reasoning event to logs
    pub fn log_event(state: &ReasoningState, event: &str, extra: Option<serde_json::Value>) {
        let entry = json!({
            "correlation_id": state.correlation_id,
            "phase": format!("{:?}", state.phase),
            "goal": state.goal,
            "event": event,
            "uncertainty": state.uncertainty,
            "hypotheses_count": state.hypotheses.len(),
            "correction_cycles": state.correction_cycles,
            "extra": extra,
        });

        // In debug mode this would be more verbose
        debug!(target: "reasoning", "{}", entry);
    }

    /// Create a summary for normal operation (redacted)
    pub fn summary(state: &ReasoningState) -> serde_json::Value {
        json!({
            "session": state.session_id.to_string(),
            "phase": format!("{:?}", state.phase),
            "goal": state.goal,
            "uncertainty": format!("{:.2}", state.uncertainty),
        })
    }

    /// Create detailed trace for debug mode
    pub fn detailed_trace(state: &ReasoningState) -> serde_json::Value {
        json!({
            "session": state.session_id.to_string(),
            "correlation_id": state.correlation_id,
            "phase": format!("{:?}", state.phase),
            "goal": state.goal,
            "hypotheses": state.hypotheses.iter().map(|h| json!({
                "id": h.id.to_string(),
                "description": h.description,
                "belief": h.belief,
            })).collect::<Vec<_>>(),
            "memory_refs": state.memory_refs,
            "uncertainty": state.uncertainty,
            "correction_cycles": state.correction_cycles,
        })
    }
}
