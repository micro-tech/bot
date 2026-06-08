//! Workflow Registry — stores and retrieves best workflows for given input types.
use crate::hy_evo::genome::WorkflowGenome;
use std::collections::HashMap;

pub struct WorkflowRegistry {
    store: HashMap<String, WorkflowGenome>,
}

impl WorkflowRegistry {
    pub fn new() -> Self {
        Self { store: HashMap::new() }
    }

    pub fn register(&mut self, input_type: &str, genome: WorkflowGenome) {
        self.store.insert(input_type.to_string(), genome);
    }

    pub fn get_best(&self, input_type: &str) -> Option<&WorkflowGenome> {
        self.store.get(input_type)
    }

    pub fn list_types(&self) -> Vec<String> {
        self.store.keys().cloned().collect()
    }
}
