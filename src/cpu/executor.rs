// cpu/executor.rs

use crate::cpu::instructions::Instruction;
use crate::cpu::interfaces::{MemoryInterface, LlmInterface, SkillInterface, BusInterface};
use crate::cpu::state::AgentState;
use crate::hy_evo::NodeResult;
use crate::utils::log_to_file;
use log::debug;
use serde_json::Value;
use std::sync::Arc;
use async_trait::async_trait;

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

    pub async fn execute(
        &mut self,
        state: &mut AgentState,
        instr: Instruction,
    ) -> NodeResult {
        debug!("Starting execution of instruction: {:?}", instr);
        log_to_file(&format!("Starting execution of instruction: {:?}", instr));
        
        match instr {
            Instruction::ReadMemory { key } => {
                debug!("Attempting to read memory for key: {}", key);
                let result = self.memory.read(&key);
                match result {
                    Ok(value) => {
                        state.working_memory_key = Some(key);
                        debug!("Successfully read memory for key: {}", key);
                        result  // Return the result directly
                    },
                    Err(e) => {
                        let error_msg = format!("Error reading memory for key {}: {:?}", key, e);
                        eprintln!("{}", error_msg);
                        log_to_file(&error_msg);
                        result  // Return the error result
                    }
                }
            },
            Instruction::WriteMemory { key, value } => {
                debug!("Attempting to write memory for key: {}", key);
                let result = self.memory.write(&key, value);
                match result {
                    Ok(_) => {
                        debug!("Successfully wrote memory for key: {}", key);
                        result
                    },
                    Err(e) => {
                        let error_msg = format!("Error writing memory for key {}: {:?}", key, e);
                        eprintln!("{}", error_msg);
                        log_to_file(&error_msg);
                        result
                    }
                }
            },
            Instruction::RunSkill { name, args } => {
                debug!("Attempting to run skill: {}", name);
                let result = self.skills.call(&name, &args).await;
                match result {
                    Ok(_) => {
                        debug!("Successfully ran skill: {}", name);
                        result
                    },
                    Err(e) => {
                        let error_msg = format!("Error running skill {}: {:?}", name, e);
                        eprintln!("{}", error_msg);
                        log_to_file(&error_msg);
                        result
                    }
                }
            },
            Instruction::ExecuteHooks { phase } => {
                debug!("Executing hooks for phase: {}", phase);
                let result = self.hooks.run_phase(&phase);  // Assuming synchronous for now
                if let Err(e) = result {
                    let error_msg = format!("Error executing hooks for phase {}: {:?}", phase, e);
                    eprintln!("{}", error_msg);
                    log_to_file(&error_msg);
                }
                result  // Return the result
            },
            Instruction::EmitBusEvent { topic, payload } => {
                debug!("Attempting to emit bus event to topic: {}", topic);
                let msg = crate::bus::Message {
                    to: topic,
                    from: "cpu".into(),
                    data: payload.to_string(),
                    timestamp: chrono::Utc::now().timestamp_millis() as u64,
                };
                let result = self.bus.publish(&msg).await;  // Assuming async
                match result {
                    Ok(_) => {
                        debug!("Successfully emitted bus event to topic: {}", topic);
                        result
                    },
                    Err(e) => {
                        let error_msg = format!("Error emitting bus event to topic {}: {:?}", topic, e);
                        eprintln!("{}", error_msg);
                        log_to_file(&error_msg);
                        result
                    }
                }
            },
            Instruction::PlanNextSteps => {
                debug!("Planning next steps");
                NodeResult::Ok(())  // Placeholder, assuming it has Ok variant
            },
            Instruction::ReflectOnLastStep => {
                debug!("Reflecting on last step");
                NodeResult::Ok(())  // Placeholder
            },
            Instruction::UpdateBelief { key, value } => {
                debug!("Attempting to update belief for key: {}", key);
                let result = self.memory.update_belief(&key, value);
                match result {
                    Ok(_) => {
                        debug!("Successfully updated belief for key: {}", key);
                        result
                    },
                    Err(e) => {
                        let error_msg = format!("Error updating belief for key {}: {:?}", key, e);
                        eprintln!("{}", error_msg);
                        log_to_file(&error_msg);
                        result
                    }
                }
            },
            Instruction::WaitForEvent => {
                debug!("Waiting for event");
                NodeResult::Ok(())  // Placeholder
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct MockMemory;
    #[async_trait]
    impl MemoryInterface for MockMemory {
        fn read(&mut self, _key: &str) -> NodeResult { NodeResult::Ok(serde_json::Value::Null) }
        fn write(&mut self, _key: &str, _value: Value) -> NodeResult { NodeResult::Ok(()) }
    }

    #[async_trait]
    struct MockSkills;
    #[async_trait]
    impl SkillInterface for MockSkills {
        async fn call(&self, _name: &str, _params: &Value) -> NodeResult { NodeResult::Ok(()) }
    }

    struct MockHooks;  // Simplified, assuming it's not async

    #[async_trait]
    struct MockBus;
    #[async_trait]
    impl BusInterface for MockBus {
        async fn publish(&self, _to: &str, _data: Value) -> NodeResult { NodeResult::Ok(()) }
    }

    #[tokio::test]
    async fn test_execute_read_memory_success() {
        let memory = MockMemory;
        let skills = MockSkills;
        let hooks = MockHooks;
        let bus = Arc::new(MockBus);
        let mut executor = CpuExecutor::new(memory, skills, hooks, bus);
        let mut state = AgentState::new();
        let result = executor.execute(&mut state, Instruction::ReadMemory { key: "test".to_string() }).await;
        match result {
            NodeResult::Ok(_) => assert!(true),
            _ => assert!(false),
        }
    }
}