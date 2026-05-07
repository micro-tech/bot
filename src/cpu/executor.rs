// cpu/executor.rs

use crate::cpu::instructions::Instruction;
use crate::cpu::interfaces::{BusInterface, LlmInterface, MemoryInterface, SkillInterface};
use crate::cpu::state::AgentState;
// use crate::error;
use crate::hy_evo::NodeResult;
use crate::utils::log_to_file;
use async_trait::async_trait;
use log::{debug, error};
use serde_json::Value;
use std::sync::Arc;

pub struct CpuExecutor {
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

    pub async fn execute(&mut self, state: &mut AgentState, instr: Instruction) -> NodeResult {
        debug!("Starting execution of instruction: {:?}", instr);
        log_to_file(&format!("Starting execution of instruction: {:?}", instr));

        match instr {
            Instruction::ReadMemory { key } => {
                debug!("Attempting to read memory for key: {}", key);
                let result = self.memory.read(&key);
                match result {
                    NodeResult::Value(value) => {
                        state.working_memory_key = Some(key.clone());
                        debug!("Successfully read memory for key: {}", key);
                        NodeResult::Value(value)
                    }
                    NodeResult::Error(e) => {
                        let error_msg = format!("Error reading memory for key {}: {:?}", key, e);
                        eprintln!("{}", error_msg);
                        log_to_file(&error_msg);
                        NodeResult::Error(error_msg)
                    }
                    other => other,
                }
            }
            Instruction::WriteMemory { key, value } => {
                debug!("Attempting to write memory for key: {}", key);
                let result = self.memory.write(&key, value);
                match result {
                    NodeResult::None | NodeResult::Value(_) | NodeResult::Text(_) => {
                        debug!("Successfully wrote memory for key: {}", key);
                        NodeResult::None
                    }
                    NodeResult::Error(e) => {
                        let error_msg = format!("Error writing memory for key {}: {:?}", key, e);
                        eprintln!("{}", error_msg);
                        log_to_file(&error_msg);
                        NodeResult::Error(error_msg)
                    }
                }
            }
            Instruction::RunSkill { name, args } => {
                debug!("Attempting to run skill: {}", name);
                let result = self.skills.call(&name, &args);
                match result {
                    NodeResult::Error(e) => {
                        let error_msg = format!("Error running skill {}: {:?}", name, e);
                        eprintln!("{}", error_msg);
                        log_to_file(&error_msg);
                        NodeResult::Error(error_msg)
                    }
                    node_res => {
                        debug!("Successfully ran skill: {}", name);
                        node_res
                    }
                }
            }
            Instruction::ExecuteHooks { phase } => {
                debug!("Executing hooks for phase: {}", phase);
                self.hooks.run_phase(&phase);
                debug!("Successfully executed hooks for phase: {}", phase);
                NodeResult::None
            }
            Instruction::EmitBusEvent { topic, payload } => {
                debug!("Attempting to emit bus event to topic: {}", topic);
                let msg = crate::bus::Message {
                    to: topic.clone(),
                    from: "cpu".into(),
                    data: payload.to_string(),
                    timestamp: chrono::Utc::now().timestamp_millis() as u64,
                };
                if let Err(e) = self.bus.publish(msg.clone()) {
                    let error_msg = format!("CPU failed to publish message: {}", e);
                    log_to_file(&error_msg);
                    error!("{}", error_msg);
                }
                debug!("Successfully emitted bus event to topic: {}", topic);
                NodeResult::None
            }
            Instruction::PlanNextSteps => {
                debug!("Planning next steps");
                NodeResult::None
            }
            Instruction::ReflectOnLastStep => {
                debug!("Reflecting on last step");
                NodeResult::None
            }
            Instruction::UpdateBelief { key, value } => {
                debug!("Attempting to update belief for key: {}", key);
                let result = self.memory.update_belief(&key, value);
                match result {
                    NodeResult::None | NodeResult::Value(_) | NodeResult::Text(_) => {
                        debug!("Successfully updated belief for key: {}", key);
                        NodeResult::None
                    }
                    NodeResult::Error(e) => {
                        let error_msg = format!("Error updating belief for key {}: {:?}", key, e);
                        eprintln!("{}", error_msg);
                        log_to_file(&error_msg);
                        NodeResult::Error(error_msg)
                    }
                }
            }
            Instruction::WaitForEvent => {
                debug!("Waiting for event");
                NodeResult::None
            }
            Instruction::CallLlm { .. } => {
                debug!("Executor received CallLlm — delegating to CPU");
                log_to_file("Executor received CallLlm — delegating to CPU");

                // Executor does nothing for LLM calls.
                // CPU handles this in Cpu::execute_instruction.
                NodeResult::None
            }
        }
    }
}
