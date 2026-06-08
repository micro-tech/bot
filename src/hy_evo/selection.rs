//! Workflow Selection by Input Type
//!
//! Uses the WorkflowRegistry to pick the best workflow for a given input.
use crate::hy_evo::registry::WorkflowRegistry;
use crate::hy_evo::genome::WorkflowGenome;

pub fn select_workflow(registry: &WorkflowRegistry, input_type: &str) -> Option<WorkflowGenome> {
    registry.get_best(input_type).cloned()
}
