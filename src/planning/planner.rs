//! Planner — decomposes high-level goals into subtasks.
pub struct Planner;

impl Planner {
    pub fn new() -> Self {
        Self
    }

    /// Placeholder decomposition function.
    pub fn decompose(&self, goal: &str) -> Vec<String> {
        vec![format!("Step 1 for: {}", goal), "Step 2".to_string()]
    }
}
