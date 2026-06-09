use uuid::Uuid;

use super::genome::WorkflowGenome;
use super::node::{Node, NodeMetadata, NodeResult};

use crate::cpu::interfaces::{BusInterface, LlmInterface, MemoryInterface, SkillInterface};

/// A validated, executable workflow derived from a WorkflowGenome.
/// The CPU uses this structure to run nodes in order.
#[derive(Debug, Clone)]
pub struct Workflow {
    /// Unique workflow ID
    pub id: Uuid,

    /// Ordered list of nodes (after topological sort)
    pub ordered_nodes: Vec<(NodeMetadata, Node)>,

    /// Original genome (for scoring, reflection, mutation)
    pub genome: WorkflowGenome,
}

impl Workflow {
    /// Convert a WorkflowGenome into a validated Workflow.
    /// Performs:
    /// - edge validation
    /// - cycle detection
    /// - topological sorting
    pub fn from_genome(genome: WorkflowGenome) -> anyhow::Result<Self> {
        let order = topological_sort(&genome)?;

        let ordered_nodes = order
            .into_iter()
            .map(|idx| genome.nodes[idx].clone())
            .collect();

        Ok(Self {
            id: genome.id,
            ordered_nodes,
            genome,
        })
    }

    /// Execute a single node.
    /// The CPU will call this for each node in ordered_nodes.
    pub async fn execute_node(&self, index: usize, ctx: &mut WorkflowContext<'_>) -> NodeResult {
        let (_, node) = &self.ordered_nodes[index];

        match node {
            Node::Skill { name, args } => {
                let params_val = serde_json::to_value(args).unwrap_or_default();
                ctx.execute_skill(name, &params_val).await
            }

            Node::Llm {
                model,
                prompt,
                params,
            } => {
                let params_val = serde_json::to_value(params).unwrap_or_default();
                ctx.execute_llm(model, prompt, &params_val).await
            }

            Node::Code { function, params } => {
                let params_val = serde_json::to_value(params).unwrap_or_default();
                ctx.execute_code(function, &params_val).await
            }

            Node::MemoryRead { key } => ctx.memory_read(key),

            Node::MemoryWrite { key, value } => ctx.memory_write(key, value.clone()),

            Node::Memory { operation, key, value } => {
                match operation.as_str() {
                    "read" => ctx.memory_read(key),
                    "write" => {
                        if let Some(v) = value {
                            ctx.memory_write(key, v.clone())
                        } else {
                            NodeResult::Error("Missing value for memory write".into())
                        }
                    }
                    _ => NodeResult::Error("Unknown memory operation".into()),
                }
            }

            Node::Reflection { target: _, prompt } => {
                NodeResult::Text(format!("Reflection: {}", prompt))
            }

            Node::BusPublish { to, data } => ctx.bus_publish(to, data.clone()).await,

            Node::Conditional {
                condition,
                then_branch,
                else_branch,
            } => {
                #[allow(unused_variables)]
                {
                    ctx.execute_conditional(condition, then_branch, else_branch)
                        .await
                }
            }
        }
    }
}

/// Execution context passed to each node.
/// This is the glue between HyEvo and your agent OS subsystems.
pub struct WorkflowContext<'a> {
    pub memory: &'a mut dyn MemoryInterface,
    pub skills: &'a dyn SkillInterface,
    pub llm: &'a dyn LlmInterface,
    pub bus: &'a dyn BusInterface,
}

impl<'a> WorkflowContext<'a> {
    pub async fn execute_skill(&self, name: &str, params: &serde_json::Value) -> NodeResult {
        self.skills.call(name, params).await
    }

    pub async fn execute_llm(
        &self,
        model: &str,
        prompt: &str,
        params: &serde_json::Value,
    ) -> NodeResult {
        self.llm.call(model, prompt, params).await
    }

    pub async fn execute_code(&self, function: &str, params: &serde_json::Value) -> NodeResult {
        self.skills.call(function, params).await
    }

    pub fn memory_read(&mut self, key: &str) -> NodeResult {
        self.memory.read(key)
    }

    pub fn memory_write(&mut self, key: &str, value: serde_json::Value) -> NodeResult {
        self.memory.write(key, value)
    }

    pub async fn bus_publish(&self, to: &str, data: serde_json::Value) -> NodeResult {
        self.bus.publish(to, data).await
    }

    pub async fn execute_conditional(
        &mut self,
        condition: &super::node::ConditionNode,
        then_branch: &Vec<Uuid>,
        else_branch: &Vec<Uuid>,
    ) -> NodeResult {
        // Placeholder — real logic will be added in integration.rs
        NodeResult::None
    }
}

/// Perform a topological sort and validate the workflow graph.
fn topological_sort(genome: &WorkflowGenome) -> anyhow::Result<Vec<usize>> {
    let node_count = genome.nodes.len();
    let mut indegree = vec![0usize; node_count];

    for (from, to) in &genome.edges {
        indegree[*to] += 1;
    }

    let mut queue: Vec<usize> = indegree
        .iter()
        .enumerate()
        .filter(|(_, deg)| **deg == 0)
        .map(|(i, _)| i)
        .collect();

    let mut order = Vec::new();

    while let Some(node) = queue.pop() {
        order.push(node);

        for (from, to) in &genome.edges {
            if *from == node {
                indegree[*to] -= 1;
                if indegree[*to] == 0 {
                    queue.push(*to);
                }
            }
        }
    }

    if order.len() != node_count {
        anyhow::bail!("Cycle detected in workflow graph");
    }

    Ok(order)
}
