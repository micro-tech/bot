// cpu/state.rs
use crate::utils::log_to_file;
use log::debug;
use std::time::Instant;

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
    pub last_tick: Instant,
    pub step_counter: u64,
    pub current_task_id: Option<String>,
    pub working_memory_key: Option<String>, // pointer into memory/
    pub error: Option<String>,
}

impl AgentState {
    pub fn new() -> Self {
        Self {
            mode: AgentMode::Idle,
            last_tick: Instant::now(),
            step_counter: 0,
            current_task_id: None,
            working_memory_key: None,
            error: None,
        }
    }

    pub fn set_mode(&mut self, mode: AgentMode) {
        debug!("Mode changed from {:?} to {:?}", self.mode, mode);
        self.mode = mode;
        log_to_file(&format!("Agent mode changed to {:?}", mode));
    }

    pub fn bump_step(&mut self) {
        debug!("Bumping step to {}", self.step_counter + 1);
        self.step_counter += 1;
        self.last_tick = Instant::now();
        log_to_file(&format!(
            "Agent step counter bumped to {}",
            self.step_counter
        ));
    }
}
