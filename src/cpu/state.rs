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

    // Timing. Task 315: heartbeat timing (start_time, last_heartbeat,
    // uptime, tick_count) moved to HeartbeatState below, which carries
    // its own lock — the heartbeat loop must not contend on the Cpu
    // mutex. `last_tick` stays here: it tracks agent steps (bump_step),
    // not beats, and nothing reads it outside this struct.
    pub last_tick: Instant,

    // Counters
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

/// Task 315: the heartbeat's timing state, behind its own lock instead
/// of inside `AgentState` behind the Cpu mutex. The heartbeat loop
/// (main.rs) locks ONLY this state, so a routine holding the Cpu mutex
/// for minutes — a self-improve cycle doing LLM work, a nightly `cargo
/// test` — can no longer starve the beat. The critical section is a
/// synchronous increment + two Instant stamps; it never awaits.
/// `start_time` is immutable after construction, so `uptime` is derived
/// on each beat. Shared as `Arc<std::sync::Mutex<HeartbeatState>>`
/// (std Mutex, not tokio: the section never awaits).
#[derive(Debug)]
pub struct HeartbeatState {
    pub start_time: Instant,
    pub last_heartbeat: Instant,
    pub uptime: Duration,
    pub tick_count: u64,
}

impl HeartbeatState {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            start_time: now,
            last_heartbeat: now,
            uptime: Duration::ZERO,
            tick_count: 0,
        }
    }

    /// One beat: advance the tick, stamp the beat time, refresh uptime.
    /// Called by the heartbeat loop and by `Cpu::handle_heartbeat`;
    /// never takes the Cpu lock.
    pub fn handle_heartbeat(&mut self) {
        println!("Handling heartbeat: Tick {}", self.tick_count);
        self.tick_count += 1;
        self.last_heartbeat = Instant::now();
        self.uptime = self.start_time.elapsed();
    }
}

impl Default for HeartbeatState {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentState {
    pub fn new() -> Self {
        let now = Instant::now();

        Self {
            mode: AgentMode::Idle,

            last_tick: now,

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

    pub fn record_error(&mut self, msg: String) {
        self.error_count += 1;
        self.error = Some(msg.clone());
        self.set_mode(AgentMode::Error);
        log_to_file(&format!("Agent error: {}", msg));
    }
}

impl Default for AgentState {
    fn default() -> Self {
        Self::new()
    }
}
