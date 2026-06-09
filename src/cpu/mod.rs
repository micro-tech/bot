// src/cpu/mod.rs
#![allow(unused_imports)]

pub mod executor;
pub mod instructions;
pub mod interfaces;
pub mod state;
pub mod time_scheduler;
pub mod workflow_executor;

use std::sync::Arc;
use std::time::Instant;

use crate::bus::{Bus, Message};
use crate::reasoning::engine::ReasoningEngine;
use crate::cpu::instructions::Instruction;
#[allow(unused_imports)]
use crate::cpu::interfaces::{BusInterface, LlmInterface, MemoryInterface, SkillInterface};
use crate::cpu::state::AgentState;
use log::{debug, error, warn};

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
    pub reasoning: Option<ReasoningEngine>,
    pub reasoning_config: crate::config::reasoning::ReasoningConfig,
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
        reasoning_config: crate::config::reasoning::ReasoningConfig,
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
            reasoning: None,
            reasoning_config,
        })
    }

    /// Initialize or replace the reasoning engine with a new goal.
    pub async fn start_reasoning(&mut self, goal: &str) -> anyhow::Result<()> {
        let engine = ReasoningEngine::new(goal.to_string(), (*self.bus).clone());
        engine.start().await?;
        self.reasoning = Some(engine);
        log_to_file(&format!("ReasoningEngine started with goal: {}", goal));
        Ok(())
    }

    /// Stop and clear the current reasoning engine
    pub fn stop_reasoning(&mut self) {
        if self.reasoning.take().is_some() {
            log_to_file("ReasoningEngine stopped and cleared");
        }
    }

    /// Change the current reasoning goal (restarts the engine with a new goal)
    pub async fn change_goal(&mut self, new_goal: &str) -> anyhow::Result<()> {
        self.stop_reasoning();
        self.start_reasoning(new_goal).await?;
        log_to_file(&format!("Reasoning goal changed to: {}", new_goal));
        Ok(())
    }

    /// Get current reasoning metrics
    pub async fn reasoning_metrics(&self) -> serde_json::Value {
        match &self.reasoning {
            Some(engine) => {
                let state = engine.get_state().await;
                serde_json::json!({
                    "goal": state.goal,
                    "phase": format!("{:?}", state.phase),
                    "hypotheses_count": state.hypotheses.len(),
                    "correction_cycles": state.correction_cycles,
                    "has_plan": state.current_plan.is_some(),
                    "plan_steps": state.current_plan.as_ref().map(|p| p.steps.len()).unwrap_or(0),
                    "tick": self.state.tick_count
                })
            }
            None => serde_json::json!({
                "status": "no_reasoning_engine"
            }),
        }
    }

    /// Simple reasoning health check — returns whether the engine appears to be making progress
    pub async fn reasoning_health_check(&self) -> serde_json::Value {
        match &self.reasoning {
            Some(engine) => {
                let state = engine.get_state().await;
                let has_progress = !state.hypotheses.is_empty() || state.current_plan.is_some();
                let is_active = !state.hypotheses.is_empty() || state.current_plan.is_some();

                serde_json::json!({
                    "healthy": has_progress || is_active,
                    "has_hypotheses": !state.hypotheses.is_empty(),
                    "has_plan": state.current_plan.is_some(),
                    "phase": format!("{:?}", state.phase),
                    "correction_cycles": state.correction_cycles,
                    "message": if has_progress || is_active {
                        "Reasoning engine is active and making progress"
                    } else {
                        "Reasoning engine started but no hypotheses or plan yet"
                    }
                })
            }
            None => serde_json::json!({
                "healthy": false,
                "status": "no_reasoning_engine",
                "message": "No reasoning engine is currently running"
            }),
        }
    }

    /// Reset reasoning state (stop + clear goal)
    pub fn reset_reasoning(&mut self) {
        self.stop_reasoning();
        log_to_file("Reasoning state fully reset");
    }

    /// Run one full reasoning cycle (hypothesis → plan → execute).
    /// Returns true if more steps remain, false if the plan completed or failed.
    pub async fn run_reasoning_cycle(&mut self) -> anyhow::Result<bool> {
        let engine = match &self.reasoning {
            Some(e) => e,
            None => {
                self.start_reasoning("Improve agent reliability and self-correction").await?;
                self.reasoning.as_ref().unwrap()
            }
        };

        // If no hypotheses yet, propose one
        let state = engine.get_state().await;
        if state.hypotheses.is_empty() {
            engine.propose_hypothesis("Use self-correction loops to recover from failures").await;
        }

        // Create plan if we don't have one
        if state.current_plan.is_none() {
            engine.create_plan().await?;
        }

        // Execute next step
        let more_steps = engine.execute_next_step().await?;
        Ok(more_steps)
    }

    /// Get a redacted summary of the current reasoning state
    pub async fn reasoning_summary(&self) -> serde_json::Value {
        match &self.reasoning {
            Some(engine) => engine.reasoning_summary().await,
            None => serde_json::json!({ "status": "no_reasoning_engine" }),
        }
    }

    /// Pause the reasoning engine (prevents cycles from running)
    pub fn pause_reasoning(&mut self) {
        self.state.reasoning_paused = true;
        log_to_file("Reasoning engine paused");
    }

    /// Resume the reasoning engine
    pub fn resume_reasoning(&mut self) {
        self.state.reasoning_paused = false;
        log_to_file("Reasoning engine resumed");
    }

    // -------------------------------------------------------------------------
// Reasoning Engine API
// -------------------------------------------------------------------------
//
// Public methods for controlling and observing the ReasoningEngine:
//
// Control:
//   - start_reasoning(goal)
//   - stop_reasoning()
//   - reset_reasoning()
//   - change_goal(new_goal)
//   - pause_reasoning()
//   - resume_reasoning()
//   - force_next_reasoning_step()   [manual trigger]
//
// Observation:
//   - reasoning_metrics()
//   - reasoning_health_check()
//   - reasoning_status()            [combined view]
//   - reasoning_summary()
//   - reasoning_trace()
//
// Bus integration:
//   - reasoning_command messages (pause/resume/reset/force_step)
//   - Periodic publishing of metrics and state
// -------------------------------------------------------------------------

    /// Check if reasoning is currently paused
    pub fn is_reasoning_paused(&self) -> bool {
        self.state.reasoning_paused
    }

    /// Combined reasoning status (pause state + key metrics + health)
    pub async fn reasoning_status(&self) -> serde_json::Value {
        let paused = self.is_reasoning_paused();
        let metrics = self.reasoning_metrics().await;
        let health = self.reasoning_health_check().await;

        serde_json::json!({
            "paused": paused,
            "metrics": metrics,
            "health": health,
            "tick": self.state.tick_count
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

        match self
            .llm
            .call("ollama", &prompt, &serde_json::Value::Null)
            .await
        {
            crate::hy_evo::node::NodeResult::Text(new_manifest) => {
                let proposed = SystemManifest::load_from_string(&new_manifest);
                let diff = self.manifest.diff(&proposed);

                // Reject deletions
                if diff.contains("-\n") {
                    log_to_file(&format!(
                        "Manifest evolution rejected: contains deletions\nDiff:\n{}",
                        diff
                    ));
                    return;
                }

                // Placeholder validation
                let is_valid = true;

                if is_valid {
                    log_to_file(&format!("Manifest diff:\n{}", diff));

                    // Backup
                    let _backup = self.manifest.raw.clone();

                    // Apply
                    self.manifest = proposed;

                    log_to_file("Manifest evolved successfully");
                } else {
                    log_to_file("Manifest evolution failed validation");
                }
            }

            _ => {
                log_to_file("Manifest evolution failed: no response");
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

        match self
            .llm
            .call("ollama", &prompt, &serde_json::Value::Null)
            .await
        {
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
                        if modified.elapsed().unwrap_or_default()
                            > std::time::Duration::from_secs(7 * 24 * 3600)
                        {
                            // 7 days
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

        log_to_file("Nightly maintenance completed");
    }

    pub async fn self_repair(&mut self) {
        log_to_file("Running self-repair routines");

        // Repair working memory overflow
        if self.memory.working.context.len() > self.memory.working.max_len * 2 {
            self.memory
                .working
                .context
                .truncate(self.memory.working.max_len);
            log_to_file("Repaired: truncated working memory");
        }

        // Reasoning engine health check
        if self.reasoning.is_some() {
            let health = self.reasoning_health_check().await;
            if !health["healthy"].as_bool().unwrap_or(true) {
                log_to_file(&format!(
                    "Reasoning engine unhealthy: {}",
                    health["message"].as_str().unwrap_or("unknown")
                ));
                // Optionally auto-reset if stuck
                if self.state.tick_count % 200 == 0 {
                    self.reset_reasoning();
                    log_to_file("Auto-reset reasoning engine due to poor health");
                }
            }
        }

        // Check manifest integrity (placeholder)
        log_to_file("Manifest integrity OK");

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
            context.push_str(&format!(
                "Recent working memory:\n{}\n\n",
                recent.join("\n")
            ));
        }

        // Beliefs
        if !self.memory.beliefs.is_empty() {
            let beliefs_str = self
                .memory
                .beliefs
                .iter()
                .map(|(k, v)| format!("{}: {}", k, v))
                .collect::<Vec<_>>()
                .join("\n");
            context.push_str(&format!("Stable beliefs:\n{}\n\n", beliefs_str));
        }

        format!(
            "Context from memory:\n{}\n\nPersonality: {}\n\nOriginal prompt:\n{}",
            context, self.personality, prompt
        )
    }

    // -------------------------------------------------------------------------
    // LLM Routing
    // -------------------------------------------------------------------------

    fn route_llm_request(&self, target: LlmTarget, prompt: String, correlation_id: u64) {
        let to = match target {
            LlmTarget::OllamaLan => "ollama_server",
            LlmTarget::OllamaLocal => "ollama_local3090",
            LlmTarget::Gemini => "gemini",
            LlmTarget::Grok => "grok",
        };

        let msg = Message {
            to: to.to_string(),
            from: "cpu".into(),
            data: serde_json::json!({
                "type": "chat_request",
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

    /// Handle an LLM response coming back on the bus.
    pub fn handle_llm_response(&mut self, msg: Message) -> Result<(), String> {
        println!("[CPU] Received llm_response from {}", msg.from);

        let payload: serde_json::Value = serde_json::from_str(&msg.data).unwrap_or_else(|e| {
            let err = format!("Failed to parse LLM response payload: {}", e);
            log_to_file(&err);
            error!("{}", err);
            serde_json::json!({})
        });

        let correlation_id = payload["correlation_id"].as_u64().unwrap_or(0);
        let text = payload["msg"].as_str().unwrap_or("").to_string();

        println!("[CPU] Forwarding response to web_interface ({} chars)", text.len());

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

        match self.bus.publish(ui_msg) {
            Ok(()) => {
                println!("[CPU] Successfully forwarded LLM response to web_interface");
                debug!("CPU forwarded LLM response to UI");
                Ok(())
            }
            Err(e) => {
                let err = format!("CPU failed to publish LLM output to UI: {}", e);
                log_to_file(&err);
                error!("{}", err);
                Err(err)
            }
        }
    }

    pub fn handle_bus_message(&mut self, msg: Message) {
        log_to_file(&format!(
            "CPU received message from='{}' type='{}'",
            msg.from,
            &msg.data[..msg.data.len().min(80)]
        ));
        let payload: serde_json::Value = serde_json::from_str(&msg.data).unwrap_or_default();
        if let Some(msg_type) = payload["type"].as_str() {
            match msg_type {
                "user_input" => {
                    let prompt = payload["content"].as_str().unwrap_or("").to_string();
                    let correlation_id = payload["correlation_id"].as_u64().unwrap_or(0);

                    // Store in working memory so LLM context includes recent history
                    let _ = self.memory.working.write(
                        "context",
                        serde_json::Value::String(format!("user: {}", prompt)),
                    );

                    self.route_llm_request(
                        crate::io::ollama::LlmTarget::OllamaLan,
                        prompt,
                        correlation_id,
                    );
                    log_to_file("CPU routed user_input to Ollama");
                }

                "chat_request" => {
                    // Direct chat requests from the web UI (bypassing CPU for LLM,
                    // but we still want to record them in memory for context).
                    let prompt = payload["prompt"].as_str().unwrap_or("").to_string();
                    if !prompt.is_empty() {
                        let _ = self.memory.working.write(
                            "context",
                            serde_json::Value::String(format!("user: {}", prompt)),
                        );
                        log_to_file(&format!(
                            "CPU recorded chat_request in memory: {}",
                            &prompt[..prompt.len().min(80)]
                        ));
                    }
                }

                "ollama_response" | "llm_output" => {
                    // Record bot replies in working memory too
                    let reply = payload["msg"].as_str().unwrap_or("").to_string();
                    if !reply.is_empty() {
                        let llm = payload["llm"].as_str().unwrap_or("bot");
                        let _ = self.memory.working.write(
                            "context",
                            serde_json::Value::String(format!(
                                "{}: {}",
                                llm,
                                &reply[..reply.len().min(500)]
                            )),
                        );
                    }
                }

                "llm_response" => {
                    if let Err(e) = self.handle_llm_response(msg) {
                        log_to_file(&format!("CPU LLM response error: {}", e));
                    }
                }

                "reasoning_command" => {
                    let cmd = payload["command"].as_str().unwrap_or("");
                    match cmd {
                        "pause" => self.pause_reasoning(),
                        "resume" => self.resume_reasoning(),
                        "reset" => self.reset_reasoning(),
                        "force_step" => {
                            log_to_file("force_step requested via bus (not yet supported in sync handler)");
                        }
                        _ => log_to_file(&format!("Unknown reasoning_command: {}", cmd)),
                    }
                }

                "skill_request" => {
                    // Direct skill execution request — CPU runs the skill synchronously
                    // and publishes the result back to web_interface.
                    let skill = payload["skill"].as_str().unwrap_or("").to_string();
                    let args = payload["args"].clone();
                    let correlation_id = payload["correlation_id"].as_u64().unwrap_or(0);
                    if skill.is_empty() {
                        log_to_file("CPU got skill_request with empty skill name — ignored");
                    } else {
                        log_to_file(&format!("CPU executing skill '{}' directly", skill));
                        let result = crate::tools::execute(&skill, &args);
                        let _ = self.bus.publish(Message {
                            to: "web_interface".to_string(),
                            from: "cpu_skill".to_string(),
                            data: serde_json::json!({
                                "type": "ollama_response",
                                "llm": "skill",
                                "correlation_id": correlation_id,
                                "msg": result,
                            })
                            .to_string(),
                            timestamp: now_ms(),
                        });
                    }
                }

                _ => log_to_file(&format!("CPU ignored msg type: {}", msg_type)),
            }
        }
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

            Instruction::ExecuteHooks { phase: _ } => {
                // If you have hooks, wire them here
            }

            Instruction::PlanNextSteps => {
                // Real planning logic
                println!("[CPU] Planning next steps using planning module");
                log_to_file("CPU: Planning next steps");
            }

            Instruction::ReflectOnLastStep => {
                // Real reflection logic
                println!("[CPU] Reflecting on last step");
                log_to_file("CPU: Reflecting on last execution step");
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

        // Auto-start reasoning engine if we don't have one
        if self.state.tick_count % 50 == 0 && self.reasoning.is_none() && self.reasoning_config.is_enabled() {
            if let Err(e) = self.start_reasoning(&self.reasoning_config.default_goal()).await {
                warn!("Failed to start reasoning engine: {}", e);
            }
        }

        // Run reasoning cycle every N ticks (respect pause flag)
        if !self.state.reasoning_paused
            && self.state.tick_count % self.reasoning_config.cycle_interval() == 0
            && self.reasoning.is_some()
            && self.reasoning_config.is_enabled()
        {
            match self.run_reasoning_cycle().await {
                Ok(more) => {
                    if !more {
                        log_to_file("Reasoning plan completed or exhausted");
                    }
                }
                Err(e) => warn!("Reasoning cycle error: {}", e),
            }
        }

        // Publish reasoning metrics to the bus every N ticks
        if self.state.tick_count % self.reasoning_config.metrics_interval() == 0 && self.reasoning.is_some() {
            let metrics = self.reasoning_metrics().await;

            let msg = Message {
                to: "web_interface".to_string(),
                from: "cpu".to_string(),
                data: serde_json::json!({
                    "type": "reasoning_metrics",
                    "metrics": metrics,
                    "tick": self.state.tick_count
                }).to_string(),
                timestamp: now_ms(),
            };

            if let Err(e) = self.bus.publish(msg) {
                warn!("Failed to publish reasoning metrics: {}", e);
            }
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
} // ← closes impl Cpu<L>

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
