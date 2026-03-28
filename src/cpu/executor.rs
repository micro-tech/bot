// cpu/executor.rs

use crate::cpu::instructions::Instruction;
use crate::cpu::interfaces::MemoryInterface;
use crate::cpu::state::AgentState;
use crate::hy_evo::NodeResult;
use crate::utils::log_to_file;
use log::debug;
use serde_json::Value;
use std::sync::Arc;

pub struct CpuExecutor {
    // Inject handles to subsystems
    pub memory: crate::memory::MemoryHandle,
    pub skills: crate::skills::SkillRegistry,
    pub hooks: crate::hooks::HookRegistry,
    pub bus: Arc<crate::bus::Bus>,
}

impl CpuExecutor {
    pub fn new(
        memory: crate::memory::MemoryHandle,
        skills: crate::skills::SkillRegistry,
        hooks: crate::hooks::HookRegistry,
        bus: Arc<crate::bus::Bus>,
    ) -> Self {
        Self {
            memory,
            skills,
            hooks,
            bus,
        }
    }

    pub async fn execute(
        &mut self,
        state: &mut AgentState,
        instr: Instruction,
    ) -> anyhow::Result<()> {
        debug!("Executing instruction: {:?}", instr);
        log_to_file(&format!("Executing instruction: {:?}", instr));
        match instr {
            Instruction::ReadMemory { key } => {
                let _value = self.memory.read(&key);
                state.working_memory_key = Some(key);
                // stash value into state or working memory as needed
            }
            Instruction::WriteMemory { key, value } => {
                self.memory.write(&key, value);
            }
            Instruction::RunSkill { name, args } => {
                self.skills.call(&name, &args);
            }
            Instruction::ExecuteHooks { phase } => {
                self.hooks.run_phase(&phase);
            }
            Instruction::EmitBusEvent { topic, payload } => {
                let msg = crate::bus::Message {
                    to: topic,
                    from: "cpu".into(),
                    data: payload.to_string(),
                    timestamp: chrono::Utc::now().timestamp_millis() as u64,
                };
                self.bus.publish(msg);
            }
            Instruction::PlanNextSteps => {
                // call planner / Bayesian router here
            }
            Instruction::ReflectOnLastStep => {
                // call reflection / analysis skill here
            }
            Instruction::UpdateBelief { key, value } => {
                self.memory.update_belief(&key, value);
            }
            Instruction::WaitForEvent => {
                // no-op here; scheduler/loop will block on event
            }
        }

        Ok(())
    }
}
