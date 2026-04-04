// src/cpu/mod.rs

use std::sync::Arc;
use std::time::Instant;

use crate::bus::{Bus, Message};
use crate::cpu::executor::{CpuExecutor, CpuExecutorImpl};
use crate::cpu::interfaces::{BusInterface, LlmInterface, MemoryInterface, SkillInterface};
use crate::cpu::state::AgentState;
use crate::hy_evo::integration::HyEvoIntegration;
use crate::hy_evo::reflection::ReflectionLlm;
use crate::hy_evo::scoring::ExecutionMetrics;
use crate::llm::LlmTarget;
use crate::memory::MemoryManager;
use crate::utils::now_ms;

use log::{debug, error};
use serde_json::Value;

// If you have an Instruction enum somewhere else, import it:
use crate::cpu::state::Instruction; // adjust path if needed
use crate::utils::log_to_file; // adjust path if needed

/// Main CPU struct, generic over the LLM type.
pub struct Cpu<L>
where
    L: ReflectionLlm + LlmInterface + Send + Sync + 'static,
{
    pub state: AgentState,
    pub memory: MemoryManager,
    pub skills: Box<dyn SkillInterface>,
    pub llm: L,
    pub bus: Arc<Bus>,
    pub hyevo: HyEvoIntegration<L>,
}

impl<L> Cpu<L>
where
    L: ReflectionLlm + LlmInterface + Send + Sync + 'static,
{
    pub fn new(
        memory: MemoryManager,
        skills: Box<dyn SkillInterface>,
        llm: L,
        bus: Arc<Bus>,
        hyevo: HyEvoIntegration<L>,
    ) -> Self {
        Self {
            state: AgentState::new(),
            memory,
            skills,
            llm,
            bus,
            hyevo,
        }
    }

    /// Route an LLM request to the appropriate backend.
    fn route_llm_request(&self, target: LlmTarget, prompt: String, correlation_id: u64) {
        let to = match target {
            LlmTarget::OllamaLan => "ollama_lan",
            LlmTarget::OllamaLocal => "ollama_local",
            LlmTarget::Gemini => "gemini",
            LlmTarget::Grok => "grok",
        };

        let msg = Message {
            to: to.to_string(),
            from: "cpu".into(),
            data: serde_json::json!({
                "type": "llm_request",
                "correlation_id": correlation_id,
                "prompt": prompt,
            })
            .to_string(),
            timestamp: now_ms(),
        };

        if let Err(e) = self.bus.sender.send(msg.clone()) {
            let error_msg = format!("CPU failed to route LLM request to {}: {}", to, e);
            log_to_file(&error_msg);
            error!("{}", error_msg);
        } else {
            debug!("CPU routed LLM request to {}", to);
            log_to_file(&format!("CPU routed LLM request to {}", to));
        }
    }

    /// Handle an LLM response coming back on the bus.
    pub fn handle_llm_response(&mut self, msg: Message) -> Result<(), String> {
        let payload: Value = serde_json::from_str(&msg.data).unwrap_or_else(|e| {
            let err = format!("Failed to parse LLM response payload: {}", e);
            log_to_file(&err);
            error!("{}", err);
            serde_json::json!({})
        });

        let correlation_id = payload["correlation_id"].as_u64().unwrap_or(0);
        let text = payload["msg"].as_str().unwrap_or("").to_string();

        // Build UI message
        let ui_msg = Message {
            to: "web_interface".to_string(),
            from: "cpu".to_string(),
            data: serde_json::json!({
                "type": "llm_output",
                "correlation_id": correlation_id,
                "msg": text,
            })
            .to_string(),
            timestamp: now_ms(),
        };

        if let Err(e) = self.bus.publish(ui_msg.clone()) {
            let err = format!("CPU failed to publish LLM output to UI: {}", e);
            log_to_file(&err);
            error!("{}", err);
            return Err(err);
        }

        debug!("CPU forwarded LLM response to UI");
        log_to_file("CPU forwarded LLM response to UI");

        Ok(())
    }

    /// Execute a single instruction from the CPU state machine / HyEvo.
    pub fn execute_instruction(&mut self, instr: Instruction) {
        println!("Executing instruction: {:?}", instr);

        match instr {
            Instruction::RunSkill { name, args } => {
                if let Err(e) = self
                    .skills
                    .call(&name, args, &mut self.memory, self.bus.as_ref())
                {
                    eprintln!("Error running skill: {}", e);
                }
            }

            Instruction::ExecuteHooks { phase } => {
                // If you have hooks, wire them here; placeholder:
                // let _ = self.hooks.execute(&phase, &mut self.memory, self.bus.as_ref());
            }

            Instruction::PlanNextSteps => {
                // TODO: planning logic
            }

            Instruction::ReflectOnLastStep => {
                // TODO: reflection logic
            }

            Instruction::WaitForEvent => {
                // idle
            }

            Instruction::CallLlm {
                target,
                prompt,
                correlation_id,
            } => {
                self.route_llm_request(target, prompt, correlation_id);
            }
        }
    }

    /// Main heartbeat handler — called every X ms by your scheduler.
    pub fn handle_heartbeat(&mut self) {
        self.state.bump_tick();
        self.state.last_heartbeat = Instant::now();
        self.state.uptime = self.state.start_time.elapsed().unwrap_or_default();

        // Manifest-driven routines
        self.run_manifest_routines();

        // HyEvo cycle
        if self.state.tick_count % 10 == 0 {
            let _ = self.run_hyevo_cycle();
        }
    }

    /// Run a HyEvo evolution cycle.
    pub fn run_hyevo_cycle(&mut self) -> anyhow::Result<()> {
        let executor = CpuExecutorImpl {
            memory: &mut self.memory as &mut dyn MemoryInterface,
            skills: self.skills.as_ref(),
            llm: &self.llm as &dyn LlmInterface,
            bus: self.bus.as_ref() as &dyn BusInterface,
        };

        self.hyevo.run_and_evolve(&executor)
    }

    /// Writes heartbeat.md
    fn write_heartbeat_file(&self) -> std::io::Result<()> {
        use std::fs;

        let md = format!(
            "# Heartbeat\n\n\
             Tick: {}\n\
             Uptime: {:?}\n\
             Mode: {:?}\n\
             Errors: {}\n",
            self.state.tick_count, self.state.uptime, self.state.mode, self.state.error_count,
        );

        fs::write("heartbeat.md", md)
    }
}
