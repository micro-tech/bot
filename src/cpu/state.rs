use crate::utils::log_to_file;
use log::debug;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMode {
    Idle,
    Conversational,
    ToolUse,
    Planning,
    Reflecting,
    Sleeping,
    Error,
}

#[derive(Debug, Clone)]
pub struct AgentState {
    pub mode: AgentMode,

    // Timing
    pub start_time: Instant,
    pub last_tick: Instant,
    pub last_heartbeat: Instant,
    pub uptime: Duration,

    // Counters
    pub tick_count: u64,
    pub step_counter: u64,
    pub error_count: u64,

    // Task / memory pointers
    pub current_task_id: Option<String>,
    pub working_memory_key: Option<String>,

    // Last error message
    pub error: Option<String>,

    // Reasoning control
    pub reasoning_paused: bool,
}

impl AgentState {
    pub fn new() -> Self {
        let now = Instant::now();

        Self {
            mode: AgentMode::Idle,

            start_time: now,
            last_tick: now,
            last_heartbeat: now,
            uptime: Duration::ZERO,

            tick_count: 0,
            step_counter: 0,
            error_count: 0,

            current_task_id: None,
            working_memory_key: None,

            error: None,
            reasoning_paused: false,
        }
    }

    pub fn set_mode(&mut self, mode: AgentMode) {
        debug!("Mode changed from {:?} to {:?}", self.mode, mode);
        self.mode = mode;
        log_to_file(&format!("Agent mode changed to {:?}", mode));
    }

    pub fn bump_step(&mut self) {
        self.step_counter += 1;
        self.last_tick = Instant::now();
        log_to_file(&format!(
            "Agent step counter bumped to {}",
            self.step_counter
        ));
    }

    pub fn bump_tick(&mut self) {
        self.tick_count += 1;
        self.last_tick = Instant::now();
    }

    pub fn record_error(&mut self, msg: String) {
        self.error_count += 1;
        self.error = Some(msg.clone());
        self.set_mode(AgentMode::Error);
        log_to_file(&format!("Agent error: {}", msg));
    }
}
