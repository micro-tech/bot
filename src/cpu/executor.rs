// cpu/executor.rs

use crate::cpu::instructions::Instruction;
use crate::cpu::state::AgentState;
use log::debug;
use std::sync::Arc;
use crate::utils::log_to_file;

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

    pub async fn execute(&self, state: &mut AgentState, instr: Instruction) -> anyhow::Result<()> {
        debug!("Executing instruction: {:?}", instr);
        log_to_file(&format!("Executing instruction: {:?}", instr));
        match instr {
            Instruction::ReadMemory { key } => {
                let value = self.memory.read(&key).await?;
                state.working_memory_key = Some(key);
                // you can stash value somewhere else if needed
            }
            Instruction::WriteMemory { key, value } => {
                self.memory.write(&key, value).await?;
            }
            Instruction::RunSkill { name, args } => {
                self.skills.run(&name, args).await?;
            }
            Instruction::ExecuteHooks { phase } => {
                self.hooks.run_phase(&phase, state).await?;
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
                self.memory.update_belief(&key, value).await?;
            }
            Instruction::WaitForEvent => {
                // no-op here; scheduler/loop will block on event
            }
        }

        Ok(())
    }
}