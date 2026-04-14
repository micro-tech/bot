pub mod bayesian;
use crate::hooks::bayesian::{BayesianHook, BayesianHookConfig};

/// Central registry for all hooks in the system.
/// Your CPU/state machine will hold one of these.
pub struct HookRegistry {
    pub bayesian: BayesianHook,
}

impl HookRegistry {
    pub fn new() -> Self {
        let config = BayesianHookConfig {
            priors: Default::default(),
        };

        Self {
            bayesian: BayesianHook::new(config),
        }
    }

    /// Run pre‑processing hook chain
    pub fn pre_process(&mut self, msg: &str) -> String {
        self.bayesian.pre_process(msg)
    }

    /// Run post‑processing hook chain
    pub fn post_process(&self, response: &str, belief: &str) -> String {
        self.bayesian.post_process(response, belief)
    }

    /// Dispatch hooks by phase name (pre_input, post_input, post_plan, etc.)
    pub fn run_phase(&mut self, phase: &str) {
        match phase {
            "pre_input" | "pre_process" => {
                self.pre_process("");
            }
            "post_input" | "post_plan" | "post_process" => {
                self.post_process("", "");
            }
            _ => {}
        }
    }
}
