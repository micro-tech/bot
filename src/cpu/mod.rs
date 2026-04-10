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

use chrono::{Timelike, Utc};

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
    pub personality: String,
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
            personality: "neutral".to_string(),
        })
    }

    /// Runs routines described in system_manifest.md
    pub async fn run_manifest_routines(&mut self) {
        // === Memory maintenance ===
        if self.manifest.raw.contains("summarize working memory") {
            if let Some(chunk) = self.memory.working.drain_oldest_chunk(5) {
                match self.llm.summarize(&chunk).await {
                    crate::hy_evo::node::NodeResult::Text(summary) => {
                        self.memory.working.push_summary(summary);
                    }
                    _ => {}
                }
            }
        }

        // === Manifest evolution ===
        if self.manifest.raw.contains("evolve constitution") {
            self.evolve_manifest().await;
        }

        // === Error log scanning ===
        if self.manifest.raw.contains("check error logs") {
            let _ = std::fs::read_to_string("logs/error_log.md");
        }

        // === End-of-day routines ===
        if self.manifest.raw.contains("End of Day") {
            // You can add time-window logic later
        }

        // === HyEvo tuning ===
        if self.manifest.raw.contains("Run HyEvo cycle") {
            // Already handled in heartbeat, but manifest can override later
        }
    }

    /// Evolve the system manifest using LLM suggestions
    pub async fn evolve_manifest(&mut self) {
        let current_manifest = &self.manifest.raw;

        let prompt = format!(
            "You are an AI agent constitution optimizer.\n\
             The current system manifest is:\n\n{}\n\n\
             Suggest improvements to make the agent more effective, safe, and aligned with its goals.\n\
             Provide the improved manifest in full, starting with the title and sections.\n\
             Ensure it remains in markdown format with ## headers.\n\
             Focus on adding useful routines, improving policies, or enhancing safety.\n\
             Avoid removing existing content to ensure safety.",
            current_manifest
        );

        match self.llm.call("ollama", &prompt, &serde_json::Value::Null).await {
            crate::hy_evo::node::NodeResult::Text(new_manifest) => {
                let proposed = SystemManifest::load_from_string(&new_manifest);
                let diff = self.manifest.diff(&proposed);

                // Check for safe updates only: no deletions
                if diff.contains("-\n") {
                    log_to_file(&format!("Manifest evolution rejected: contains deletions\nDiff:\n{}", diff));
                    return;
                }

                // Validate the new manifest
                if self.validate_manifest(&new_manifest) {
                    // Log the diff
                    log_to_file(&format!("Manifest diff:\n{}", diff));
                    // Backup current
                    let backup = self.manifest.raw.clone();
                    // Apply new
                    self.manifest = proposed;
                    // Log success
                    log_to_file(&format!("Manifest evolved successfully"));
                } else {
                    // Rollback or log error
                    log_to_file(&format!("Manifest evolution failed validation"));
                }
            }
            _ => {
                log_to_file(&format!("Manifest evolution failed: no response"));
            }
        }
    }

    pub async fn arbitrate_skill(&self, task: &str) -> String {
        let available_skills = "noop, send_email, read_log"; // hardcoded for now
        let prompt = format!(
            "Available skills: {}\n\
             Task description: {}\n\
             Choose the most appropriate skill name from the list above.\n\
             Respond with only the skill name.",
            available_skills, task
        );

        match self.llm.call("ollama", &prompt, &serde_json::Value::Null).await {
            crate::hy_evo::node::NodeResult::Text(name) => name.trim().to_string(),
            _ => "noop".to_string(),
        }
    }

    pub async fn nightly_maintenance(&mut self) {
        log_to_file("Starting nightly maintenance");

        // Cleanup old logs
        if let Ok(entries) = std::fs::read_dir("logs/") {
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    if let Ok(modified) = metadata.modified() {
                        if modified.elapsed().unwrap_or_default() > std::time::Duration::from_secs(7 * 24 * 3600) { // 7 days
                            let _ = std::fs::remove_file(entry.path());
                        }
                    }
                }
            }
        }

        // Backup manifest
        let backup_path = format!("backups/manifest_{}.md", chrono::Utc::now().timestamp());
        let _ = std::fs::create_dir_all("backups");
        let _ = std::fs::write(&backup_path, &self.manifest.raw);

        // Force evolution if not recent
        // Already in manifest routines

        log_to_file("Nightly maintenance completed");
    }

    pub async fn self_repair(&mut self) {
        log_to_file("Running self-repair routines");

        // Repair working memory overflow
        if self.memory.working.context.len() > self.memory.working.max_len * 2 {
            self.memory.working.context.truncate(self.memory.working.max_len);
            log_to_file("Repaired: truncated working memory");
        }

        // Repair episodic memory overflow
        // Assuming episodic has max, but not implemented

        // Check manifest integrity
        if !self.validate_manifest(&self.manifest.raw) {
            log_to_file("Manifest corrupted, reloading from disk");
            if let Ok(manifest) = SystemManifest::load("system_manifest.md") {
                self.manifest = manifest;
            }
        }

        // Reset stuck state if uptime too long without tick
        if self.state.uptime.as_secs() > 3600 && self.state.tick_count < 100 {
            log_to_file("Detected stuck state, resetting tick");
            self.state.tick_count = 0;
        }

        log_to_file("Self-repair completed");
    }
    fn enhance_prompt_with_memory(&self, prompt: &str) -> String {
        let mut context = String::new();

        // Working memory
        if let Some(recent) = self.memory.working.get_recent_entries(3) {
            context.push_str(&format!("Recent working memory:\n{}\n\n", recent.join("\n")));
        }

        // Beliefs
        if !self.memory.beliefs.is_empty() {
            let beliefs_str = self.memory.beliefs.iter()
                .map(|(k, v)| format!("{}: {}", k, v))
                .collect::<Vec<_>>()
                .join("\n");
            context.push_str(&format!("Stable beliefs:\n{}\n\n", beliefs_str));
        }

        // Vector memory (placeholder)
        // TODO: search vector DB for relevant facts

        // Episodic memory (placeholder)
        // TODO: retrieve relevant episodes

        format!("Context from memory:\n{}\n\nPersonality: {}\n\nOriginal prompt:\n{}", context, self.personality, prompt)
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
            error!("Failed to route LLM request to {}: {}", to, e);
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
                let skill_name = if name == "auto" {
                    self.arbitrate_skill(&args.to_string()).await
                } else {
                    name
                };
                let result = self.skills.call(&skill_name, &args).await;
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
                let enhanced_prompt = self.enhance_prompt_with_memory(&prompt);
                self.route_llm_request(target, enhanced_prompt, correlation_id);
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

        // Nightly maintenance at 2 AM
        if Utc::now().hour() == 2 && self.state.tick_count % 3600 == 0 {
            self.nightly_maintenance().await;
        }

        // Self-repair every 100 ticks
        if self.state.tick_count % 100 == 0 {
            self.self_repair().await;
        }

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
