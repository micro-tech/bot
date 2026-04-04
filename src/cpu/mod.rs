// src/cpu/mod.rs
pub mod executor;
pub mod instructions;
pub mod interfaces;
pub mod state;
pub mod time_scheduler;

use std::sync::Arc;
use std::time::Instant;

use crate::bus::{Bus, Message};
use crate::cpu::executor::CpuExecutor;
use crate::cpu::instructions::{CpuEvent, CpuEventKind, Instruction};
use crate::cpu::interfaces::{BusInterface, LlmInterface, MemoryInterface, SkillInterface};
use crate::cpu::state::AgentState;

use crate::hy_evo::integration::{CpuExecutor as HyEvoCpuExecutor, HyEvoIntegration};
use crate::hy_evo::reflection::ReflectionLlm;
use crate::hy_evo::scoring::ExecutionMetrics;
use crate::hy_evo::workflow::{Workflow, WorkflowContext};

use crate::config::manifest::SystemManifest;
use crate::io::ollama::LlmTarget;
use crate::memory::MemoryManager;
use crate::utils::{log_to_file, now_ms};

use log::{debug, error};
use serde_json::Value;

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
    pub manifest: SystemManifest,
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
        manifest_path: &str,
    ) -> std::io::Result<Self> {
        // Load the system manifest from disk
        let manifest = SystemManifest::load(manifest_path)?;

        Ok(Self {
            state: AgentState::new(),
            memory,
            skills,
            llm,
            bus,
            hyevo,
            manifest,
        })
    }

    /// Runs routines described in system_manifest.md
    pub async fn run_manifest_routines(&mut self) {
        let manifest = &self.manifest.raw;

        // === Memory maintenance ===
        if manifest.contains("summarize working memory") {
            if let Some(chunk) = self.memory.working.drain_oldest_chunk(5) {
                match self.llm.summarize(&chunk).await {
                    crate::hy_evo::node::NodeResult::Text(summary) => {
                        self.memory.working.push_summary(summary);
                    }
                    _ => {}
                }
            }
        }

        // === Error log scanning ===
        if manifest.contains("check error logs") {
            let _ = std::fs::read_to_string("logs/error_log.md");
        }

        // === End-of-day routines ===
        if manifest.contains("End of Day") {
            // You can add time-window logic later
        }

        // === HyEvo tuning ===
        if manifest.contains("Run HyEvo cycle") {
            // Already handled in heartbeat, but manifest can override later
        }
    }

    // -------------------------------------------------------------------------
    // LLM Routing
    // -------------------------------------------------------------------------

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

        if let Err(e) = self.bus.publish(msg.clone()) {
            let error_msg = format!("CPU failed to route LLM request to {}: {}", to, e);
            log_to_file(&error_msg);
            error!("{}", error_msg);
        } else {
            debug!("CPU routed LLM request to {}", to);
            log_to_file(&format!("CPU routed LLM request to {}", to));
        }
    }

    pub fn handle_bus_message(&mut self, msg: Message) {
        log_to_file(&format!("CPU received message: {:?}", msg));
        // TODO: handle specific messages
    }

    // -------------------------------------------------------------------------
    // Instruction Execution
    // -------------------------------------------------------------------------

    pub async fn execute_instruction(&mut self, instr: Instruction) {
        println!("Executing instruction: {:?}", instr);

        match instr {
            Instruction::ReadMemory { key } => {
                let _ = self.memory.read(&key);
            }

            Instruction::WriteMemory { key, value } => {
                let _ = self.memory.write(&key, value);
            }

            Instruction::EmitBusEvent { topic, payload } => {
                let msg = Message {
                    to: topic,
                    from: "cpu".into(),
                    data: payload.to_string(),
                    timestamp: now_ms(),
                };
                let _ = self.bus.publish(msg);
            }

            Instruction::UpdateBelief { key, value } => {
                let _ = self.memory.write(&key, value);
            }

            Instruction::RunSkill { name, args } => {
                let result = self.skills.call(&name, &args).await;
                if let crate::hy_evo::node::NodeResult::Error(e) = result {
                    eprintln!("Error running skill: {}", e);
                }
            }

            Instruction::ExecuteHooks { phase } => {
                // If you have hooks, wire them here
            }

            Instruction::PlanNextSteps => {
                // TODO
            }

            Instruction::ReflectOnLastStep => {
                // TODO
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

    // -------------------------------------------------------------------------
    // Heartbeat + HyEvo
    // -------------------------------------------------------------------------

    pub async fn handle_heartbeat(&mut self) {
        println!("Handling heartbeat: Tick {}", self.state.tick_count);
        self.state.bump_tick();
        self.state.last_heartbeat = Instant::now();
        self.state.uptime = self.state.start_time.elapsed();

        self.run_manifest_routines().await;

        if self.state.tick_count % 10 == 0 {
            if let Err(e) = self.run_hyevo_cycle().await {
                eprintln!("Error in HyEvo cycle: {}", e);
            }
        }
    }

    pub async fn run_hyevo_cycle(&mut self) -> anyhow::Result<()> {
        let mut executor = CpuExecutorImpl {
            memory: &mut self.memory as &mut dyn MemoryInterface,
            skills: self.skills.as_ref(),
            llm: &self.llm as &dyn LlmInterface,
            bus: self.bus.as_ref() as &dyn BusInterface,
        };

        self.hyevo.run_and_evolve(&mut executor).await
    }
}

struct CpuExecutorImpl<'a> {
    memory: &'a mut dyn MemoryInterface,
    skills: &'a dyn SkillInterface,
    llm: &'a dyn LlmInterface,
    bus: &'a dyn BusInterface,
}

#[async_trait::async_trait]
impl<'a> HyEvoCpuExecutor for CpuExecutorImpl<'a> {
    async fn execute_workflow(&mut self, workflow: &Workflow) -> anyhow::Result<ExecutionMetrics> {
        let mut ctx = WorkflowContext {
            memory: self.memory,
            skills: self.skills,
            llm: self.llm,
            bus: self.bus,
        };
        let start = std::time::Instant::now();
        let mut metrics = ExecutionMetrics::default();
        for (i, _) in workflow.ordered_nodes.iter().enumerate() {
            let result = workflow.execute_node(i, &mut ctx).await;
            if let crate::hy_evo::node::NodeResult::Error(_) = result {
                metrics.errors += 1;
            }
        }
        metrics.latency_ms = start.elapsed().as_millis() as u64;
        metrics.success = metrics.errors == 0;
        Ok(metrics)
    }
}
