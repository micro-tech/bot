//! A/B Testing Framework (Task 44)

use crate::hy_evo::genome::WorkflowGenome;

pub struct ABTest {
    pub name: String,
    pub variant_a: WorkflowGenome,
    pub variant_b: WorkflowGenome,
    pub results_a: u32,
    pub results_b: u32,
}

impl ABTest {
    pub fn new(name: &str, a: WorkflowGenome, b: WorkflowGenome) -> Self {
        Self {
            name: name.to_string(),
            variant_a: a,
            variant_b: b,
            results_a: 0,
            results_b: 0,
        }
    }

    pub fn record_result(&mut self, variant: char, success: bool) {
        if success {
            if variant == 'a' { self.results_a += 1; } else { self.results_b += 1; }
        }
    }

    pub fn winner(&self) -> Option<char> {
        if self.results_a > self.results_b { Some('a') } else if self.results_b > self.results_a { Some('b') } else { None }
    }
}
