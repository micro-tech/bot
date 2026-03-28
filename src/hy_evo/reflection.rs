use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::genome::WorkflowGenome;
use super::scoring::ExecutionMetrics;

/// Reflection data stored after each workflow execution.
/// This helps HyEvo learn from past runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionRecord {
    pub workflow_id: Uuid,
    pub summary: String,
    pub suggestions: Vec<String>,
    pub metrics: ExecutionMetrics,
    pub timestamp: u64,
}

/// Interface for LLM reflection.
/// Your Ollama/Groq/GPT wrapper will implement this.
#[async_trait::async_trait]
pub trait ReflectionLlm {
    async fn reflect(
        &self,
        workflow: &WorkflowGenome,
        metrics: &ExecutionMetrics,
    ) -> anyhow::Result<String>;
}

/// Main reflection engine
pub struct ReflectionEngine<L: ReflectionLlm + Send + Sync> {
    pub llm: L,
}

impl<L: ReflectionLlm + Send + Sync> ReflectionEngine<L> {
    pub fn new(llm: L) -> Self {
        Self { llm }
    }

    /// Generate a reflection record for a workflow execution.
    pub async fn reflect(
        &self,
        workflow: &WorkflowGenome,
        metrics: &ExecutionMetrics,
    ) -> anyhow::Result<ReflectionRecord> {
        let llm_output = self
            .llm
            .reflect(workflow, metrics)
            .await
            .unwrap_or_else(|_| "No reflection available.".to_string());

        let suggestions = extract_suggestions(&llm_output);

        Ok(ReflectionRecord {
            workflow_id: workflow.id,
            summary: llm_output,
            suggestions,
            metrics: metrics.clone(),
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
        })
    }
}

/// Extract actionable suggestions from LLM output.
/// This keeps things simple and robust.
fn extract_suggestions(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| line.trim().starts_with("- "))
        .map(|line| line.trim().trim_start_matches("- ").to_string())
        .collect()
}
