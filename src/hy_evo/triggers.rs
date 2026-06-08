//! Workflow Triggers
//!
//! Defines events that can start workflow execution.
#[derive(Debug, Clone)]
pub enum WorkflowTrigger {
    /// Triggered by a cron schedule
    Cron(String),
    /// Triggered by a failure or error
    OnFailure(String),
    /// Triggered manually via bus message
    Manual(String),
}
