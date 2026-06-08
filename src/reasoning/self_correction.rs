//! Self-correction and error recovery (Task 73)

use crate::reasoning::state::ReasoningState;
use tracing::info;

/// Self-correction controller
pub struct SelfCorrectionLoop {
    max_cycles: u32,
}

impl SelfCorrectionLoop {
    pub fn new(max_cycles: u32) -> Self {
        Self { max_cycles }
    }

    /// Check if self-correction should be triggered based on signals
    pub fn should_correct(&self, state: &ReasoningState, error_signals: &[String]) -> bool {
        if state.correction_cycles >= self.max_cycles {
            return false;
        }
        !error_signals.is_empty()
    }

    /// Log a correction event
    pub fn log_correction(&self, correlation_id: &str, cycle: u32, reason: &str) {
        info!(
            correlation_id = correlation_id,
            cycle = cycle,
            reason = reason,
            "Self-correction cycle executed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_max_cycles_prevents_infinite_loop() {
        let loop_controller = SelfCorrectionLoop::new(2);
        let mut state = ReasoningState::new("test");
        state.correction_cycles = 2;
        assert!(!loop_controller.should_correct(&state, &["error".to_string()]));
    }
}
