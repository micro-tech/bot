//! Workflow Execution on CPU
//!
//! This module allows the CPU to load and execute HyEvo workflows.
use crate::hy_evo::genome::WorkflowGenome;
use crate::hy_evo::node::{Node, NodeResult};
use crate::hy_evo::workflow::{Workflow, WorkflowContext};
use crate::cpu::interfaces::{BusInterface, LlmInterface, MemoryInterface, SkillInterface};

pub struct WorkflowExecutor;

impl WorkflowExecutor {
    pub fn new() -> Self {
        Self
    }

    /// Execute a WorkflowGenome by converting it to a Workflow and running it.
    pub async fn execute_genome(
        &self,
        genome: &WorkflowGenome,
        ctx: &mut WorkflowContext<'_>,
    ) -> anyhow::Result<crate::hy_evo::scoring::ExecutionMetrics> {
        let workflow = Workflow::from_genome(genome);
        self.execute(&workflow, ctx).await
    }

    /// Execute a full Workflow.
    pub async fn execute(
        &self,
        workflow: &Workflow,
        ctx: &mut WorkflowContext<'_>,
    ) -> anyhow::Result<crate::hy_evo::scoring::ExecutionMetrics> {
        let mut metrics = crate::hy_evo::scoring::ExecutionMetrics::default();
        let start = std::time::Instant::now();

        for (i, node) in workflow.ordered_nodes.iter().enumerate() {
            let result = workflow.execute_node(i, ctx).await;

            match result {
                NodeResult::Error(_) => {
                    metrics.errors += 1;
                }
                NodeResult::Success => {
                    metrics.success_count += 1;
                }
                _ => {}
            }

            // Record LLM calls if the node was an LLM node
            if matches!(node, Node::Llm { .. }) {
                metrics.llm_calls += 1;
            }
        }

        metrics.latency_ms = start.elapsed().as_millis() as u64;
        metrics.success = metrics.errors == 0;
        Ok(metrics)
    }
}
