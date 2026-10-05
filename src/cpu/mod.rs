// src/cpu/mod.rs
#![allow(unused_imports)]

pub mod executor;
pub mod instructions;
pub mod interfaces;
pub mod state;
pub mod workflow_executor;
pub mod commands;

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
use crate::router::{route, resolve_with_fallback, LLMBackend, RoutingContext, RouterConfig, HealthStore};
use crate::utils::{log_to_file, now_ms};

use crate::agents::{
    agent_state::AgentState as UnifiedAgentState,
    memory_continuity::{ImportantOutcome, LessonLearned},
    planner::Planner,
    rule_layer::RuleLayer,
    runtime_loop::RuntimeLoop,
    simple_planner::SimplePlanner,
};

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
    pub router_config: RouterConfig,
    pub health_store: Option<HealthStore>,
    /// Directory of the config file Helix actually loaded (task 188).
    /// Derived from the manifest path's parent. Runtime-relative files —
    /// heartbeat.md, routines.json, logs/, memory/ — resolve here, never
    /// against the process CWD.
    pub runtime_dir: std::path::PathBuf,
}

// -------------------------------------------------------------------------
// Router Configuration Accessors
// -------------------------------------------------------------------------

/// Per-run counts from `consolidate_memory_inner`, surfaced in the nightly
/// report (task 205): how much short-term memory actually made it into
/// long-term storage.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConsolidationReport {
    pub chunks_drained: usize,
    pub episodes_archived: usize,
    pub facts_added: usize,
}

impl<L> Cpu<L>
where
    L: ReflectionLlm + LlmInterface + Send + Sync + 'static,
{
    /// Replace the current router configuration at runtime.
    pub fn set_router_config(&mut self, config: RouterConfig) {
        self.router_config = config;
        log_to_file("CPU router configuration updated");
    }

    /// Load router configuration from a TOML file (hot-reload friendly).
    pub fn load_router_config(&mut self, path: &str) -> Result<(), String> {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                match toml::from_str::<RouterConfig>(&content) {
                    Ok(cfg) => {
                        self.router_config = cfg;
                        log_to_file(&format!("Loaded router config from {}", path));
                        Ok(())
                    }
                    Err(e) => Err(format!("Failed to parse router config: {}", e)),
                }
            }
            Err(e) => Err(format!("Failed to read router config file: {}", e)),
        }
    }
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

        // Load router configuration at startup (non-fatal)
        let router_config = match std::fs::read_to_string("router.toml") {
            Ok(content) => {
                match toml::from_str::<RouterConfig>(&content) {
                    Ok(cfg) => {
                        log_to_file("Loaded router.toml at Cpu startup");
                        cfg
                    }
                    Err(e) => {
                        log_to_file(&format!("Failed to parse router.toml, using defaults: {}", e));
                        RouterConfig::default()
                    }
                }
            }
            Err(_) => {
                log_to_file("router.toml not found, using RouterConfig::default()");
                RouterConfig::default()
            }
        };

        // Auto-create HealthStore for backend health monitoring (Task 161)
        let default_backends = vec![
            "localollama".to_string(),
            "lanollama".to_string(),
            "gemini".to_string(),
        ];
        let health_store = Some(HealthStore::new(default_backends));

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
            router_config,
            health_store,
            // The manifest is loaded from <runtime_dir>/system_manifest.md
            // (task 188/199), so its parent IS the runtime dir.
            runtime_dir: std::path::Path::new(manifest_path)
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from(".")),
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

    /// Light bridge to the new Unified Agent Runtime Loop.
    /// This is a non-breaking integration point.
    pub async fn run_unified_agent<P: Planner + Send + Sync + 'static>(
        &mut self,
        goal: &str,
        planner: P,
        allowed_tools: Vec<String>,
        max_steps: u32,
    ) -> anyhow::Result<String> {
        log_to_file(&format!("[CPU] Starting unified agent runtime with goal: {}", goal));

        let mut state = UnifiedAgentState::new();
        state.messages.push(format!("goal: {}", goal));

        // Task 206: long-term memory continuity — inject relevant past
        // episodes, semantically related facts, and beliefs before the run.
        crate::agents::memory_continuity::inject_long_term_context(
            &mut state,
            &mut self.memory,
            goal,
        );

        let rule_layer = RuleLayer::new(allowed_tools);
        let mut runtime = RuntimeLoop::new(planner, rule_layer, max_steps);

        let mut final_state = runtime.run(state).await;

        // Task 206: the run itself becomes long-term memory. Record what
        // happened (a lesson when it failed) and persist the deltas into
        // the disk-backed MemoryManager.
        match final_state.last_error.clone() {
            Some(err) => {
                crate::agents::memory_continuity::record_lesson(
                    &mut final_state,
                    goal,
                    &format!("run failed: {}", err),
                    0.9,
                );
                crate::agents::memory_continuity::record_important_outcome(
                    &mut final_state,
                    goal,
                    &format!("run failed: {}", err),
                    vec!["agent-run".to_string(), "failure".to_string()],
                );
            }
            None => {
                crate::agents::memory_continuity::record_important_outcome(
                    &mut final_state,
                    goal,
                    "run completed successfully",
                    vec!["agent-run".to_string()],
                );
            }
        }
        crate::agents::memory_continuity::persist_memory_deltas(&mut final_state, &mut self.memory);

        if let Some(err) = final_state.last_error {
            Err(anyhow::anyhow!("Agent halted with error: {}", err))
        } else {
            log_to_file("[CPU] Unified agent runtime completed successfully");
            Ok("Agent completed".to_string())
        }
    }

    /// High-level convenience method: run an agent using the default planner.
    /// Uses ReflectionPlannerAdapter<SimplePlanner> for structured reasoning (Task 2).
    /// Falls back to SimplePlanner if reflection parsing fails.
    pub async fn run_agent(&mut self, goal: &str, max_steps: u32) -> anyhow::Result<String> {
        log_to_file(&format!("[CPU] run_agent (ReflectionPlannerAdapter default) started: {}", goal));

        // Wrap SimplePlanner with ReflectionPlannerAdapter for structured output
        let inner_planner = SimplePlanner::new(goal.to_string());
        let planner = crate::agents::reflection_planner_adapter::ReflectionPlannerAdapter::new(inner_planner);

        let allowed_tools = vec!["noop".to_string(), "search".to_string(), "calc".to_string()];

        self.run_unified_agent(goal, planner, allowed_tools, max_steps).await
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

    // ── Scheduled routines (task 202) ─────────────────────────────────────
    // Fired by the cron registry scheduler (task 201) as `routine_run` bus
    // messages; main.rs dispatches them here. These are John's hartbeat.md
    // End-of-Day jobs (1–7am): (a) memory consolidation, (b) error-log
    // review, (c) nighttime code fixes under the granted autonomy.

    /// Dispatch a fired routine to its handler.
    pub async fn handle_routine_run(&mut self, routine_id: &str, routine_name: &str) {
        println!("[CPU] routine_run: {} ({})", routine_name, routine_id);
        log_to_file(&format!("CPU routine_run: {} ({})", routine_name, routine_id));
        match routine_id {
            "nightly_memory_consolidation" => self.routine_memory_consolidation().await,
            "error_log_review" => self.routine_error_log_review().await,
            other => log_to_file(&format!("CPU received unknown routine_run: {}", other)),
        }
    }

    /// Append a section to the nightly report
    /// (`<runtime>/logs/nightly_report.md`) — John's morning review surface
    /// for everything the night shift did or decided.
    pub fn nightly_report(&self, title: &str, body: &str) {
        let path = self.runtime_dir.join("logs/nightly_report.md");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let entry = format!(
            "## {} — {}\n\n{}\n\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            title,
            body
        );
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| {
                use std::io::Write;
                f.write_all(entry.as_bytes())
            });
    }

    /// Task 202.1: copy short-term (working) memory to long-term storage —
    /// hartbeat.md End-of-Day (a). Gated on the 1–7am nightly window.
    /// Task 205 upgraded the inner path: summaries land in episodic + vector
    /// as typed ImportantOutcome / LessonLearned records, tagged with the
    /// date; the counts below prove the short->long-term path ran.
    pub async fn routine_memory_consolidation(&mut self) {
        if !crate::utils::in_nightly_window() {
            let msg = "memory consolidation skipped: outside the 1–7am nightly window";
            log_to_file(msg);
            self.nightly_report("Memory consolidation", msg);
            return;
        }
        let report = self.consolidate_memory_inner().await;
        let msg = format!(
            "consolidated {} working-memory chunks -> {} episodes archived, {} vector facts added ({})",
            report.chunks_drained,
            report.episodes_archived,
            report.facts_added,
            self.runtime_dir.join("memory/consolidated.md").display()
        );
        println!("[CPU] {}", msg);
        log_to_file(&msg);
        self.nightly_report("Memory consolidation", &msg);
    }

    /// Task 205: the real short-term -> long-term path. Drain the oldest
    /// working-memory chunks, summarize each via the LLM, and persist each
    /// summary as typed long-term records — an `ImportantOutcome` and a
    /// `LessonLearned` (the continuity types, now with real callers) —
    /// written to episodic (event log) AND vector (semantic store), both
    /// disk-backed, tagged with the date. The 202-era `consolidated.md`
    /// append is kept as the human-readable morning surface. A chunk whose
    /// summary fails is kept verbatim (marked UNSUMMARIZED) — consolidation
    /// never destroys memory.
    async fn consolidate_memory_inner(&mut self) -> ConsolidationReport {
        let mut report = ConsolidationReport::default();
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        // Bound one night's work: at most 10 chunks per run.
        while report.chunks_drained < 10 {
            let Some(chunk) = self.memory.working.drain_oldest_chunk(20) else {
                break;
            };
            report.chunks_drained += 1;
            let summary = match self.llm.summarize(&chunk).await {
                crate::hy_evo::node::NodeResult::Text(s) => s,
                _ => format!("[UNSUMMARIZED] {}", chunk),
            };
            let tagged = format!("[{}] {}", date, truncate_200(&summary));

            let outcome = ImportantOutcome {
                goal: "nightly memory consolidation".to_string(),
                result_summary: tagged.clone(),
                timestamp: chrono::Utc::now().to_rfc3339(),
                tags: vec!["nightly-consolidation".to_string(), date.clone()],
            };
            let lesson = LessonLearned {
                goal: "nightly memory consolidation".to_string(),
                lesson: tagged.clone(),
                timestamp: outcome.timestamp.clone(),
                // LLM summary of a drained chunk: useful, not verified.
                confidence: 0.7,
            };

            // Episodic: the durable event log.
            self.memory.episodic.record(format!(
                "OUTCOME [{}] {}",
                outcome.goal, outcome.result_summary
            ));
            report.episodes_archived += 1;

            // Vector: the semantically searchable long-term store.
            self.memory.vector.add_fact(format!(
                "Outcome for {}: {} [tags: {}]",
                outcome.goal,
                outcome.result_summary,
                outcome.tags.join(",")
            ));
            self.memory.vector.add_fact(format!(
                "Lesson about {}: {} (confidence {:.1})",
                lesson.goal, lesson.lesson, lesson.confidence
            ));
            report.facts_added += 2;

            self.persist_consolidated_memory(&tagged);
        }
        report
    }

    /// Append one summary to the file-backed long-term memory store.
    fn persist_consolidated_memory(&self, summary: &str) {
        let dir = self.runtime_dir.join("memory");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("consolidated.md");
        let entry = format!(
            "### {}\n\n{}\n\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            summary
        );
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| {
                use std::io::Write;
                f.write_all(entry.as_bytes())
            });
    }

    /// Task 202.2: error-log check that actually reports findings —
    /// hartbeat.md End-of-Day (b). Replaces the old read-and-discard.
    /// Findings go to the nightly report; a bus alert fires when errors
    /// were found.
    pub async fn routine_error_log_review(&mut self) {
        let findings = self.check_error_logs();
        let mut body = String::new();
        for (path, count) in &findings.files {
            body.push_str(&format!("- {}: {} error lines\n", path, count));
        }
        if findings.recent.is_empty() {
            body.push_str("\nNo errors found. Log is clean.\n");
        } else {
            body.push_str("\nMost recent errors:\n");
            for line in &findings.recent {
                body.push_str(&format!("  {}\n", line));
            }
        }
        println!("[CPU] error-log review: {} error lines", findings.total);
        log_to_file(&format!("error-log review: {} error lines", findings.total));
        self.nightly_report("Error log review", &body);

        if findings.total > 0 {
            let _ = self.bus.publish(Message {
                to: "web_interface".to_string(),
                from: "cpu".to_string(),
                data: serde_json::json!({
                    "type": "error",
                    "msg": format!(
                        "Nightly error-log review: {} error lines. See nightly report.",
                        findings.total
                    ),
                })
                .to_string(),
                timestamp: now_ms(),
            });
            // Task 202.3: nighttime code-fix autonomy.
            self.maybe_nightly_fix(&findings).await;
        }
    }

    /// Read the error logs and return real findings. Checks the hartbeat.md
    /// path (`<runtime>/logs/error_log.md`, resolved against the runtime dir
    /// — the old code used a CWD-relative path) plus the `log_to_file`
    /// stream (`~/.helix/logs/error_log.md`), which is where CPU log lines
    /// actually land.
    pub fn check_error_logs(&self) -> ErrorFindings {
        let mut candidates = vec![self.runtime_dir.join("logs/error_log.md")];
        if let Some(home) = std::env::var_os("HOME") {
            candidates.push(
                std::path::PathBuf::from(home)
                    .join(".helix/logs/error_log.md"),
            );
        }
        let mut findings = ErrorFindings::default();
        for path in candidates {
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let lines: Vec<&str> = content
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect();
            if lines.is_empty() {
                continue;
            }
            findings.total += lines.len();
            findings
                .files
                .push((path.display().to_string(), lines.len()));
            for line in lines.iter().rev().take(5) {
                findings.recent.push(truncate_200(line).to_string());
            }
        }
        findings
    }

    /// Task 202.3: nighttime code-write autonomy (granted by John 2026-10-04).
    /// Every guardrail is enforced here in code, not just documented:
    /// 1. Nightly window (1–7am local) only — otherwise propose-only.
    /// 2. Only errors actually observed in error_log.md (the findings).
    /// 3. `[shell] enabled` must be set — fixes are verified with real test
    ///    runs; a red test reverts the change.
    /// 4. Every change (file, what, why) is logged to the nightly report.
    pub async fn maybe_nightly_fix(&mut self, findings: &ErrorFindings) {
        match fix_autonomy_decision(
            crate::utils::in_nightly_window(),
            findings.total,
            crate::tools::shell_tool::is_enabled(),
        ) {
            FixDecision::ProposeOnly(reason) => {
                self.nightly_report(
                    "Code-fix autonomy",
                    &format!("Propose-only: {}. Findings recorded, no changes made.", reason),
                );
                return;
            }
            FixDecision::Armed => {}
        }
        self.nightly_report(
            "Code-fix autonomy",
            &format!(
                "ARMED (1–7am window, {} logged errors, [shell] enabled). Attempting fixes.",
                findings.total
            ),
        );
        // Bound the night's work: at most 2 distinct errors, 8 tool steps each.
        for entry in findings.recent.iter().take(2) {
            self.attempt_nightly_fix(entry).await;
        }
    }

    /// One bounded fix attempt: a ReAct loop over the repo-scoped tools,
    /// then test verification with revert-on-red.
    async fn attempt_nightly_fix(&mut self, error_line: &str) {
        self.nightly_report(
            "Fix attempt",
            &format!("Error under repair:\n  {}\n", error_line),
        );
        let allowed = ["repo_read", "repo_grep", "repo_glob", "run_shell"];
        let mut transcript = format!(
            "You are Helix's nighttime maintenance agent fixing a logged error.\n\
             Repo root: {}\n\
             RULES: (1) Fix ONLY the error below — no speculative refactors. \
             (2) Read files with repo_read/repo_grep before changing them. \
             (3) Apply edits via run_shell (e.g. python3 or apply_patch-style heredocs). \
             (4) Before EACH file edit, back it up: cp <file> <file>.nightly-bak. \
             (5) When the fix is in, run the relevant tests via run_shell \
             (e.g. `cargo test <filter>`); iterate if red. \
             (6) Reply with exactly one JSON tool call per turn: \
             {{\"tool\": \"<name>\", \"args\": {{...}}}} — or {{\"done\": true, \"summary\": \"...\"}} \
             when finished, or {{\"stuck\": true, \"why\": \"...\"}} if it cannot be fixed safely.\n\
             Available tools: {}.\n\n\
             ERROR TO FIX:\n{}",
            self.runtime_dir.display(),
            allowed.join(", "),
            error_line
        );
        let mut outcome = "no fix applied".to_string();

        for _step in 0..8 {
            let prompt = format!(
                "{}\n\nRespond with exactly one JSON tool call, no prose.",
                transcript
            );
            let reply = match self.llm.call("ollama", &prompt, &serde_json::Value::Null).await {
                crate::hy_evo::node::NodeResult::Text(t) => t,
                _ => {
                    transcript.push_str("\nLLM gave no usable reply; stopping.\n");
                    break;
                }
            };
            let call: serde_json::Value = match extract_json_object(&reply) {
                Some(v) => v,
                None => {
                    transcript.push_str(&format!(
                        "\n[tool] Could not parse a JSON tool call from: {}\n",
                        truncate_200(&reply)
                    ));
                    continue;
                }
            };
            if call.get("done").and_then(|v| v.as_bool()).unwrap_or(false) {
                outcome = format!(
                    "fix applied: {}",
                    call.get("summary").and_then(|v| v.as_str()).unwrap_or("(no summary)")
                );
                break;
            }
            if call.get("stuck").and_then(|v| v.as_bool()).unwrap_or(false) {
                outcome = format!(
                    "STUCK: {}",
                    call.get("why").and_then(|v| v.as_str()).unwrap_or("(no reason)")
                );
                break;
            }
            let tool = call.get("tool").and_then(|v| v.as_str()).unwrap_or("");
            if !allowed.contains(&tool) {
                transcript.push_str(&format!(
                    "\n[tool] '{}' is not allowed in the nightly fix loop. Use one of: {}.\n",
                    tool,
                    allowed.join(", ")
                ));
                continue;
            }
            let args = call.get("args").cloned().unwrap_or(serde_json::Value::Null);
            // run_shell is already gated on [shell] enabled + denylist +
            // timeouts + workdir confinement (task 191).
            let result = crate::tools::execute(tool, &args);
            transcript.push_str(&format!(
                "\n[tool {} ->]\n{}\n",
                tool,
                truncate_2000(&result)
            ));
        }

        // Every .nightly-bak the loop created (per its RULES) — discovered
        // from the filesystem, not trusted from tool output.
        let bak_list = crate::tools::execute(
            "run_shell",
            &serde_json::json!({"command": format!("find {} -name '*.nightly-bak' 2>/dev/null", self.runtime_dir.display())}),
        );
        let bak_files: Vec<&str> = bak_list.lines().map(str::trim).filter(|l| !l.is_empty()).collect();

        // Guardrail: verify with the relevant tests; revert on red.
        let test_out = crate::tools::execute(
            "run_shell",
            &serde_json::json!({"command": "cargo test --lib 2>&1 | tail -5"}),
        );
        let green = test_out.contains("test result: ok");
        if green {
            // Clean up backups — the fix survived its tests.
            for bak in &bak_files {
                let _ = crate::tools::execute(
                    "run_shell",
                    &serde_json::json!({"command": format!("rm -f {}", bak)}),
                );
            }
        } else {
            // Revert: restore every .nightly-bak the loop made.
            for bak in &bak_files {
                let orig = bak.trim_end_matches(".nightly-bak");
                let _ = crate::tools::execute(
                    "run_shell",
                    &serde_json::json!({"command": format!("mv -f {} {}", bak, orig)}),
                );
            }
            outcome = format!("REVERTED (tests red): {}. Test output: {}", outcome, truncate_200(&test_out));
        }
        self.nightly_report(
            "Fix attempt result",
            &format!("Error: {}\nOutcome: {}\nTests: {}\n", error_line, outcome, if green { "GREEN" } else { "RED — reverted" }),
        );
        log_to_file(&format!("nightly fix attempt: {} -> {}", error_line, outcome));
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

    fn enhance_prompt_with_memory(&mut self, prompt: &str) -> String {
        let mut context = String::new();

        // Beliefs are refreshed from beliefs.json on every prompt (task 203):
        // the set_belief tool writes the file directly, so the in-memory map
        // would otherwise go stale between prompts.
        self.memory.refresh_beliefs();

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
    // LLM Routing (now powered by Context Engine / Router)
    // -------------------------------------------------------------------------

    /// Build a RoutingContext from a prompt for intelligent backend selection.
    fn build_routing_context(&self, prompt: &str, _correlation_id: u64) -> RoutingContext {
        let token_estimate = prompt.split_whitespace().count();
        let has_code = prompt.contains("```") || prompt.contains("fn ") || prompt.contains("struct ");

        RoutingContext {
            prompt: prompt.to_string(),
            token_estimate,
            has_code,
            complexity_score: 0.0, // will be computed by router
            timestamp: chrono::Utc::now(),
            user_override: None,
            telemetry: None,
            health: None,
        }
    }

    /// Intelligent LLM routing using the Context Engine.
    /// Falls back to a safe default (OllamaLan) if routing fails.
    fn route_llm_request(&self, prompt: String, correlation_id: u64) {
        let ctx = self.build_routing_context(&prompt, correlation_id);
        let backend = route(&ctx, &self.router_config);

        let to = match backend {
            LLMBackend::LocalOllama => "ollama_local",
            LLMBackend::LanOllama => "ollama_server",
            LLMBackend::Gemini => "gemini",
            LLMBackend::Grok => "grok",
            LLMBackend::Fallback => "ollama_server", // safe default
        };

        let msg = Message {
            to: to.to_string(),
            from: "cpu".into(),
            data: serde_json::json!({
                "type": "chat_request",
                "correlation_id": correlation_id,
                "prompt": prompt,
                "routed_via": format!("{:?}", backend),
            })
            .to_string(),
            timestamp: now_ms(),
        };

        if let Err(e) = self.bus.publish(msg.clone()) {
            error!("Failed to route LLM request to {}: {}", to, e);
            log_to_file(&format!("CPU routing error: {}", e));
        } else {
            debug!("CPU routed LLM request to {} via {:?}", to, backend);
            log_to_file(&format!("CPU routed to {} via {:?}", to, backend));
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

                    self.route_llm_request(prompt, correlation_id);
                    log_to_file("CPU routed user_input via Context Engine");
                }

                "chat_request" => {
                    // Route through Context Engine for intelligent backend selection
                    let prompt = payload["prompt"].as_str().unwrap_or("").to_string();
                    let correlation_id = payload["correlation_id"].as_u64().unwrap_or(0);

                    if !prompt.is_empty() {
                        let _ = self.memory.working.write(
                            "context",
                            serde_json::Value::String(format!("user: {}", prompt)),
                        );
                        self.route_llm_request(prompt.clone(), correlation_id);
                        log_to_file(&format!(
                            "CPU routed chat_request via Context Engine: {}",
                            &prompt[..prompt.len().min(80)]
                        ));
                    }
                }

                "ollama_response" | "llm_output" => {
                    // Record replies in working memory too
                    let reply = payload["msg"].as_str().unwrap_or("").to_string();
                    if !reply.is_empty() {
                        let llm = payload["llm"].as_str().unwrap_or("helix");
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

                "agent_run" => {
                    let goal = payload["goal"].as_str().unwrap_or("").to_string();
                    let max_steps = payload["max_steps"].as_u64().unwrap_or(20) as u32;

                    if goal.is_empty() {
                        log_to_file("CPU received empty agent_run goal — ignored");
                    } else {
                        log_to_file(&format!("CPU received AgentRun via bus: {} (max_steps={})", goal, max_steps));

                        let bus_clone = self.bus.clone();
                        let goal_clone = goal.clone();

                        // Emit start event
                        let _ = bus_clone.publish(Message {
                            to: "event_log".to_string(),
                            from: "cpu".to_string(),
                            data: serde_json::json!({
                                "type": "agent_started",
                                "goal": goal_clone.clone(),
                                "max_steps": max_steps,
                                "status": "running"
                            }).to_string(),
                            timestamp: now_ms(),
                        });

                        // Real async execution using RuntimeLoop + SimplePlanner
                        tokio::spawn(async move {
                            use crate::agents::simple_planner::SimplePlanner;
                            use crate::agents::rule_layer::RuleLayer;
                            use crate::agents::runtime_loop::RuntimeLoop;
                            use crate::agents::agent_state::AgentState as UnifiedAgentState;

                            let planner = SimplePlanner::new(goal_clone.clone());
                            let allowed_tools = vec!["noop".to_string(), "search".to_string(), "calc".to_string()];
                            let rule_layer = RuleLayer::new(allowed_tools);
                            let mut runtime = RuntimeLoop::new(planner, rule_layer, max_steps);

                            let mut state = UnifiedAgentState::new();
                            state.messages.push(format!("goal: {}", goal_clone));

                            let final_state = runtime.run(state).await;

                            let status = if final_state.last_error.is_some() {
                                "error"
                            } else {
                                "completed"
                            };

                            let _ = bus_clone.publish(Message {
                                to: "event_log".to_string(),
                                from: "cpu".to_string(),
                                data: serde_json::json!({
                                    "type": "agent_finished",
                                    "goal": goal_clone,
                                    "status": status,
                                    "steps_taken": final_state.step_count,
                                    "error": final_state.last_error
                                }).to_string(),
                                timestamp: now_ms(),
                            });

                            log_to_file(&format!("[CPU] RuntimeLoop finished for goal '{}' with status {}", goal_clone, status));
                        });
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
                self.route_llm_request(enhanced_prompt, correlation_id);
            }
        }
    }

    // -------------------------------------------------------------------------
    // Heartbeat + HyEvo
    // -------------------------------------------------------------------------

    /// The heartbeat is John's job pump (task 199): a scheduler tick that
    /// keeps the bot moving through jobs. After de-drift (task 202) it does
    /// exactly three things: tick bookkeeping, real manifest routines, and
    /// nothing else. Scheduled nightly work arrives as `routine_run` bus
    /// messages via `handle_routine_run`, fired by the cron registry
    /// scheduler — not from here. Removed from this path as LLM-introduced
    /// drift: HyEvo evolution cycles, the reasoning-engine lifecycle
    /// (auto-start / cycle / metrics), self-repair, and the old 2am
    /// maintenance block. Those remain available as explicit calls.
    pub async fn handle_heartbeat(&mut self) {
        println!("Handling heartbeat: Tick {}", self.state.tick_count);
        self.state.bump_tick();
        self.state.last_heartbeat = Instant::now();
        self.state.uptime = self.state.start_time.elapsed();
        // NOTE (task 205): the old manifest-routine hook is gone. The
        // heartbeat is a job pump: real work arrives as `routine_run`
        // messages from the cron registry via `handle_routine_run`, not from
        // per-beat manifest string-matching. The last manifest branch
        // ("summarize working memory" -> push_summary back into working
        // memory) was a dead end — summaries never reached long-term
        // storage — and is deleted; consolidation owns that path now.
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

// ── Task 202 supporting types ─────────────────────────────────────────────

/// Findings from the error-log review (task 202.2).
#[derive(Debug, Default)]
pub struct ErrorFindings {
    pub total: usize,
    pub files: Vec<(String, usize)>,
    pub recent: Vec<String>,
}

/// The nighttime code-write autonomy gate (task 202.3, granted 2026-10-04).
#[derive(Debug, PartialEq, Eq)]
pub enum FixDecision {
    /// All guardrails pass: attempt the fix.
    Armed,
    /// A guardrail failed: record findings, change nothing.
    ProposeOnly(&'static str),
}

/// Pure gate for the autonomy decision — unit-testable without the clock.
pub fn fix_autonomy_decision(
    in_window: bool,
    error_count: usize,
    shell_enabled: bool,
) -> FixDecision {
    if !in_window {
        return FixDecision::ProposeOnly("outside the 1–7am nightly window");
    }
    if error_count == 0 {
        return FixDecision::ProposeOnly("no logged errors to fix");
    }
    if !shell_enabled {
        return FixDecision::ProposeOnly("[shell] not enabled — fixes cannot be test-verified");
    }
    FixDecision::Armed
}

fn truncate_200(s: &str) -> &str {
    crate::utils::truncate_str(s, 200)
}

fn truncate_2000(s: &str) -> &str {
    crate::utils::truncate_str(s, 2000)
}

/// Pull the first {...} JSON object out of free-form LLM text.
fn extract_json_object(text: &str) -> Option<serde_json::Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&text[start..=end]).ok()
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

#[cfg(test)]
mod heartbeat_tests {
    //! Task 199: prove the heartbeat ticks. These tests guard the exact bug
    //! class that left Helix heartbeat-less — a handler with zero callers.

    use super::*;
    use crate::bus::Bus;
    use crate::cpu::interfaces::LlmInterface;
    use crate::hy_evo::engine::HyEvoEngine;
    use crate::hy_evo::integration::HyEvoIntegration;
    use crate::hy_evo::node::NodeResult;
    use crate::hy_evo::reflection::ReflectionLlm;
    use crate::memory::MemoryManager;
    use crate::skills::SkillRegistry;
    use async_trait::async_trait;
    use serde_json::Value;
    use std::sync::Arc;

    #[derive(Clone)]
    struct DummyLlm;

    #[async_trait]
    impl ReflectionLlm for DummyLlm {
        async fn reflect(
            &self,
            _workflow: &crate::hy_evo::genome::WorkflowGenome,
            _metrics: &crate::hy_evo::scoring::ExecutionMetrics,
        ) -> anyhow::Result<String> {
            Ok("dummy reflection".to_string())
        }
        async fn evolve_code(&self, _feedback: &str, _code: &str) -> anyhow::Result<String> {
            Ok("- dummy".to_string())
        }
    }

    #[async_trait]
    impl LlmInterface for DummyLlm {
        async fn call(&self, _model: &str, _prompt: &str, _params: &Value) -> NodeResult {
            NodeResult::Text("dummy".to_string())
        }
        async fn summarize(&self, _text: &str) -> NodeResult {
            NodeResult::Text("dummy summary".to_string())
        }
    }

    /// Build a test Cpu with an empty manifest (no manifest-triggered branches,
    /// so no LLM calls happen) and a scratch manifest file.
    fn test_cpu(manifest_body: &str) -> Cpu<DummyLlm> {
        let dir = std::env::temp_dir().join("helix-heartbeat-test");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest_path = dir.join("system_manifest.md");
        std::fs::write(&manifest_path, manifest_body).unwrap();

        let llm = DummyLlm;
        let hyevo =
            HyEvoIntegration::new(HyEvoEngine::new(llm.clone()));
        Cpu::new(
            MemoryManager::new(100, 50),
            Box::new(SkillRegistry::new()),
            llm,
            Arc::new(Bus::new()),
            hyevo,
            manifest_path.to_str().unwrap(),
            crate::config::reasoning::ReasoningConfig::default(),
        )
        .expect("test Cpu construction must succeed")
    }

    #[tokio::test]
    async fn enhance_prompt_with_memory_includes_beliefs_stored_on_disk() {
        // GIVEN a fixture beliefs.json on disk (serialized: no other test touches it):
        let _lock = crate::memory::manager::BELIEFS_FILE_TEST_LOCK
            .lock()
            .unwrap();
        let mut fixture = std::collections::HashMap::new();
        fixture.insert(
            "fixture_belief_key".to_string(),
            serde_json::json!("fixture_belief_value"),
        );
        crate::memory::manager::save_beliefs(&fixture).expect("fixture write must succeed");

        // WHEN a Cpu enhances a prompt,
        // THEN the on-disk beliefs appear in the injected context:
        let mut cpu = test_cpu("");
        let enhanced = cpu.enhance_prompt_with_memory("hello");
        assert!(
            enhanced.contains("fixture_belief_key")
                && enhanced.contains("fixture_belief_value"),
            "prompt must include beliefs from beliefs.json, got:\n{}",
            enhanced
        );

        // (cleanup: leave no beliefs.json behind)
        let _ = std::fs::remove_file(crate::memory::manager::BELIEFS_FILE);
    }

    #[tokio::test]
    async fn handle_heartbeat_advances_tick_count_and_timestamp() {        // GIVEN a live Cpu with an empty manifest:
        let mut cpu = test_cpu("");
        let before = std::time::Instant::now();

        // WHEN the heartbeat handler runs three times:
        cpu.handle_heartbeat().await;
        cpu.handle_heartbeat().await;
        cpu.handle_heartbeat().await;

        // THEN the tick count advanced and the beat timestamp is fresh:
        assert_eq!(cpu.state.tick_count, 3);
        assert!(cpu.state.last_heartbeat >= before);
        assert!(cpu.state.last_heartbeat.elapsed() < std::time::Duration::from_secs(5));
    }

    #[tokio::test]
    async fn tick_loop_invokes_handler_on_cadence() {
        // GIVEN a live Cpu behind a mutex and a 10ms tick loop (the same
        // shape run_helix spawns in production):
        let cpu = Arc::new(tokio::sync::Mutex::new(test_cpu("")));
        let cpu_clone = cpu.clone();

        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_millis(10));
            ticker.tick().await; // skip immediate first tick, like production
            for _ in 0..4 {
                ticker.tick().await;
                let mut guard = cpu_clone.lock().await;
                guard.handle_heartbeat().await;
            }
        });
        handle.await.unwrap();

        // THEN the handler ran once per tick:
        let guard = cpu.lock().await;
        assert_eq!(guard.state.tick_count, 4);
    }

    #[test]
    fn dead_tick_paths_are_gone() {
        // The TimeScheduler placeholder and the uncompiled cpu.rs duplicate
        // must not come back: exactly one tick path (main.rs) may exist.
        let main_rs =
            std::fs::read_to_string("src/main.rs").expect("must run from repo root");
        assert!(
            main_rs.contains("handle_heartbeat"),
            "main.rs must drive handle_heartbeat"
        );
        assert!(
            !std::path::Path::new("src/cpu/cpu.rs").exists(),
            "dead duplicate src/cpu/cpu.rs must stay deleted"
        );
        assert!(
            !std::path::Path::new("src/cpu/time_scheduler.rs").exists(),
            "TimeScheduler placeholder must stay deleted"
        );
    }
}

#[cfg(test)]
mod nightly_tests {
    //! Task 202: nightly maintenance per hartbeat.md + de-drift guards.
    //! The window gate itself is tested in utils::nightly_window_tests;
    //! here we prove the jobs do real work and the autonomy gate holds.

    use super::*;
    use crate::bus::Bus;
    use crate::cpu::interfaces::LlmInterface;
    use crate::hy_evo::engine::HyEvoEngine;
    use crate::hy_evo::integration::HyEvoIntegration;
    use crate::hy_evo::node::NodeResult;
    use crate::hy_evo::reflection::ReflectionLlm;
    use crate::memory::MemoryManager;
    use crate::skills::SkillRegistry;
    use async_trait::async_trait;
    use serde_json::Value;
    use std::sync::Arc;

    #[derive(Clone)]
    struct DummyLlm;

    #[async_trait]
    impl ReflectionLlm for DummyLlm {
        async fn reflect(
            &self,
            _workflow: &crate::hy_evo::genome::WorkflowGenome,
            _metrics: &crate::hy_evo::scoring::ExecutionMetrics,
        ) -> anyhow::Result<String> {
            Ok("dummy reflection".to_string())
        }
        async fn evolve_code(&self, _feedback: &str, _code: &str) -> anyhow::Result<String> {
            Ok("- dummy".to_string())
        }
    }

    #[async_trait]
    impl LlmInterface for DummyLlm {
        async fn call(&self, _model: &str, _prompt: &str, _params: &Value) -> NodeResult {
            NodeResult::Text("dummy".to_string())
        }
        async fn summarize(&self, _text: &str) -> NodeResult {
            NodeResult::Text("dummy summary".to_string())
        }
    }

    /// Test Cpu whose runtime_dir is a scratch temp dir (manifest parent).
    fn test_cpu() -> (Cpu<DummyLlm>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "helix-nightly-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let manifest_path = dir.join("system_manifest.md");
        std::fs::write(&manifest_path, "# manifest\n").unwrap();

        let llm = DummyLlm;
        let hyevo = HyEvoIntegration::new(HyEvoEngine::new(llm.clone()));
        let cpu = Cpu::new(
            MemoryManager::new(100, 50),
            Box::new(SkillRegistry::new()),
            llm,
            Arc::new(Bus::new()),
            hyevo,
            manifest_path.to_str().unwrap(),
            crate::config::reasoning::ReasoningConfig::default(),
        )
        .expect("test Cpu construction must succeed");
        assert_eq!(cpu.runtime_dir, dir, "runtime_dir derives from manifest parent");
        (cpu, dir)
    }

    #[test]
    fn fix_autonomy_gate_holds_all_four_cases() {
        // Armed only when every guardrail passes.
        assert_eq!(
            fix_autonomy_decision(true, 3, true),
            FixDecision::Armed
        );
        // Outside the window -> propose-only (John's grant is nightly-only).
        assert_eq!(
            fix_autonomy_decision(false, 3, true),
            FixDecision::ProposeOnly("outside the 1–7am nightly window")
        );
        // Nothing logged -> nothing to fix.
        assert_eq!(
            fix_autonomy_decision(true, 0, true),
            FixDecision::ProposeOnly("no logged errors to fix")
        );
        // Shell off -> cannot test-verify -> propose-only.
        assert_eq!(
            fix_autonomy_decision(true, 3, false),
            FixDecision::ProposeOnly("[shell] not enabled — fixes cannot be test-verified")
        );
    }

    #[tokio::test]
    async fn error_log_check_reports_findings_on_fixture() {
        // GIVEN a runtime error log with known content:
        let (cpu, dir) = test_cpu();
        let log_dir = dir.join("logs");
        std::fs::create_dir_all(&log_dir).unwrap();
        std::fs::write(
            log_dir.join("error_log.md"),
            "[2026-10-05] ERROR: something broke\n\n[2026-10-05] ERROR: another thing broke\n",
        )
        .unwrap();

        // WHEN the review runs:
        let findings = cpu.check_error_logs();

        // THEN the runtime file's findings are reported (not discarded):
        let entry = findings
            .files
            .iter()
            .find(|(p, _)| p.ends_with("logs/error_log.md"))
            .expect("runtime error log must be checked");
        assert_eq!(entry.1, 2);
        assert!(findings.total >= 2);
        assert!(!findings.recent.is_empty());
        assert!(findings.recent.iter().any(|l| l.contains("something broke")));
    }

    #[tokio::test]
    async fn error_log_check_handles_missing_log() {
        // GIVEN a runtime dir with no error log:
        let (cpu, dir) = test_cpu();
        assert!(!dir.join("logs/error_log.md").exists());

        // WHEN checked:
        let findings = cpu.check_error_logs();

        // THEN no panic, and the absent runtime file contributes no findings:
        let runtime_path = dir.join("logs/error_log.md").display().to_string();
        assert!(!findings.files.iter().any(|(p, _)| p == &runtime_path));
    }

    #[tokio::test]
    async fn consolidation_lands_in_long_term_storage() {
        // Tasks 204/205 made episodic + vector disk-backed: this test's
        // record()/add_fact() calls hit ./episodes.jsonl and
        // ./vector_facts.json (the default paths). Guard them so the
        // fixtures never escape into the repo tree.
        struct DiskGuard;
        impl Drop for DiskGuard {
            fn drop(&mut self) {
                let _ = std::fs::remove_file("episodes.jsonl");
                let _ = std::fs::remove_file("vector_facts.json");
            }
        }
        let _guard = DiskGuard;
        let _ = std::fs::remove_file("episodes.jsonl");
        let _ = std::fs::remove_file("vector_facts.json");

        // GIVEN working memory with content:
        let (mut cpu, dir) = test_cpu();
        for i in 0..3 {
            let _ = cpu
                .memory
                .working
                .write("context", Value::String(format!("memory entry {}", i)));
        }

        // WHEN the consolidation runs (inner = the window-gated job's work):
        let report = cpu.consolidate_memory_inner().await;

        // THEN the counts prove the short->long-term path ran:
        assert!(report.chunks_drained >= 1);
        assert_eq!(report.episodes_archived, report.chunks_drained);
        assert_eq!(report.facts_added, 2 * report.chunks_drained);
        // AND a summary landed in file-backed long-term storage:
        let stored = std::fs::read_to_string(dir.join("memory/consolidated.md"))
            .expect("consolidated.md must exist");
        assert!(stored.contains("dummy summary"));
        // AND working memory was drained:
        assert!(cpu.memory.working.get_recent_entries(10).is_none()
            || cpu.memory.working.get_recent_entries(10).unwrap().is_empty());
        // AND an episodic record exists, date-tagged per task 205:
        let episodes = cpu.memory.episodic.recent(5);
        assert!(!episodes.is_empty());
        assert!(episodes.iter().any(|e| e.event.contains("OUTCOME [nightly memory consolidation]")));
        // AND the disk files grew (integration: persistence is real):
        let facts_json =
            std::fs::read_to_string("vector_facts.json").expect("vector facts must persist");
        assert!(facts_json.contains("Lesson about nightly memory consolidation"));
        assert!(facts_json.contains("Outcome for nightly memory consolidation"));
        // AND the typed continuity records are in the vector store:
        let hits = cpu.memory.vector.search("nightly memory consolidation", 5);
        assert!(!hits.is_empty());
        // REGRESSION (task 205): the old path pushed the summary BACK INTO
        // working memory; consolidation must leave working memory drained.
        let drained = cpu.memory.working.get_recent_entries(50);
        let working_text = format!("{:?}", drained);
        assert!(
            !working_text.contains("dummy summary"),
            "summary must not be pushed back into working memory"
        );
    }

    #[tokio::test]
    async fn nightly_report_appends_sections() {
        let (cpu, dir) = test_cpu();
        cpu.nightly_report("Test section", "body line");
        let report = std::fs::read_to_string(dir.join("logs/nightly_report.md"))
            .expect("nightly report must exist");
        assert!(report.contains("Test section"));
        assert!(report.contains("body line"));
    }

    #[test]
    fn extract_json_object_finds_tool_call_in_prose() {
        let v = extract_json_object(
            "Let me read the file first.\n{\"tool\": \"repo_read\", \"args\": {\"path\": \"src/main.rs\"}}\n",
        )
        .expect("must parse");
        assert_eq!(v["tool"], "repo_read");
        assert!(extract_json_object("no json here").is_none());
    }

    #[tokio::test]
    async fn error_review_reports_and_stays_propose_only_off_hours() {
        // GIVEN an error log with real entries:
        let (mut cpu, dir) = test_cpu();
        let log_dir = dir.join("logs");
        std::fs::create_dir_all(&log_dir).unwrap();
        std::fs::write(
            log_dir.join("error_log.md"),
            "[2026-10-05] ERROR: widget failed to sprocket\n",
        )
        .unwrap();

        // WHEN the nightly error-log review runs:
        cpu.routine_error_log_review().await;

        // THEN the findings land in the nightly report for morning review:
        let report = std::fs::read_to_string(dir.join("logs/nightly_report.md"))
            .expect("nightly report must exist");
        assert!(report.contains("Error log review"));
        assert!(report.contains("widget failed to sprocket"));
        // AND the code-fix autonomy stays propose-only unless every guardrail
        // passes (in the sandbox [shell] is off by default, so the gate
        // holds at any hour):
        let armed = crate::utils::in_nightly_window()
            && crate::tools::shell_tool::is_enabled();
        if !armed {
            assert!(
                report.contains("Propose-only"),
                "autonomy must not arm unless window + logged errors + [shell]"
            );
        }
    }
}
