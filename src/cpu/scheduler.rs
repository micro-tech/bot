/ cpu/scheduler.rs

use crate::cpu::instructions::{Instruction, CpuEvent};
use crate::cpu::state::{AgentState, AgentMode};
use log::debug;
use crate::utils::log_to_file;

pub struct Scheduler;

impl Scheduler {
    pub fn schedule(
        state: &AgentState,
        event: Option<CpuEvent>,
    ) -> Vec<Instruction> {
        debug!("Scheduling for mode {:?}, event {:?}", state.mode, event.as_ref().map(|e| &e.kind));
        log_to_file(&format!("Scheduling for mode {:?}, event {:?}", state.mode, event.as_ref().map(|e| &e.kind)));
        let mut out = Vec::new();

        match (&state.mode, event) {
            (AgentMode::Idle, Some(ev)) => {
                // new input → go conversational
                // minimal example:
                out.push(Instruction::ExecuteHooks { phase: "pre_input".into() });
                out.push(Instruction::RunSkill {
                    name: "router.conversation".into(),
                    args: serde_json::json!({ "event": ev }),
                });
                out.push(Instruction::ExecuteHooks { phase: "post_input".into() });
            }

            (AgentMode::Planning, Some(ev)) => {
                out.push(Instruction::PlanNextSteps);
                out.push(Instruction::ExecuteHooks { phase: "post_plan".into() });
            }

            (AgentMode::Reflecting, _) => {
                out.push(Instruction::ReflectOnLastStep);
            }

            (_, None) => {
                // no event → maybe periodic reflection or just wait
                out.push(Instruction::WaitForEvent);
            }

            _ => {
                out.push(Instruction::WaitForEvent);
            }
        }

        debug!("Scheduled instructions: {:?}", out);
        log_to_file(&format!("Scheduled instructions: {:?}", out));
        out
    }
}