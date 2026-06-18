//! RuntimeTrace — structured tracing for the Unified Agent Runtime Loop.
//!
//! Captures planner decisions, LLM calls, tool executions, and state changes
//! for optional visualization and debugging.

use crate::agents::planner_output::PlannerOutput;
use serde::{Deserialize, Serialize};

/// A single step recorded during agent execution.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuntimeStep {
    pub step_number: u32,
    pub planner_output: Option<PlannerOutput>,
    pub llm_prompt: Option<String>,
    pub llm_response: Option<String>,
    pub tool_name: Option<String>,
    pub tool_args: Option<serde_json::Value>,
    pub tool_result: Option<serde_json::Value>,
    pub state_note: Option<String>,
    pub error: Option<String>,
    pub halted: bool,
}

/// Container for all trace steps collected during a run.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuntimeTrace {
    pub steps: Vec<RuntimeStep>,
}

impl RuntimeTrace {
    pub fn new() -> Self {
        Self { steps: Vec::new() }
    }

    pub fn push(&mut self, step: RuntimeStep) {
        self.steps.push(step);
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// 157.4 – Human-readable pretty printer
    pub fn pretty_print(&self) -> String {
        let mut out = String::new();
        out.push_str("=== Runtime Trace ===\n");

        for (i, step) in self.steps.iter().enumerate() {
            out.push_str(&format!("\n--- Step {} ---\n", step.step_number));

            if let Some(po) = &step.planner_output {
                out.push_str(&format!("Planner Decision: {:?}\n", po));
            }
            if let Some(p) = &step.llm_prompt {
                out.push_str(&format!("LLM Prompt: {}\n", p));
            }
            if let Some(r) = &step.llm_response {
                out.push_str(&format!("LLM Response: {}\n", r));
            }
            if let Some(name) = &step.tool_name {
                out.push_str(&format!("Tool Call: {} (args={:?})\n", name, step.tool_args));
            }
            if let Some(res) = &step.tool_result {
                out.push_str(&format!("Tool Result: {:?}\n", res));
            }
            if let Some(note) = &step.state_note {
                out.push_str(&format!("State: {}\n", note));
            }
            if let Some(err) = &step.error {
                out.push_str(&format!("Error: {}\n", err));
            }
            if step.halted {
                out.push_str("→ Terminal step reached\n");
            }
        }

        out.push_str("\n=== End of Trace ===\n");
        out
    }
}
