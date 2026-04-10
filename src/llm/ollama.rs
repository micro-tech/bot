use crate::config::{OllamaConfig, OllamaRouter};
use crate::cpu::interfaces::LlmInterface;
use crate::hy_evo::genome::WorkflowGenome;
use crate::hy_evo::reflection::ReflectionLlm;
use crate::hy_evo::scoring::ExecutionMetrics;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;

/// Ollama-backed LLM client.
/// Implements `ReflectionLlm` so it can drive HyEvo reflection and evolution.
#[derive(Clone)]
pub struct OllamaLlm {
    client: Arc<Client>,
    router: Arc<OllamaRouter>,
}

impl OllamaLlm {
    pub fn new(router: Arc<OllamaRouter>) -> Self {
        Self {
            client: Arc::new(Client::new()),
            router,
        }
    }

    /// Low-level call to the Ollama `/api/generate` endpoint.
    async fn call_ollama(&self, prompt: &str) -> anyhow::Result<String> {
        let backend = self
            .router
            .default()
            .expect("No Ollama backends configured");

        let url = format!("{}/api/generate", backend.url);
        let model = backend.model.clone();

        let response = self
            .client
            .post(url)
            .json(&serde_json::json!({
                "model": model,
                "prompt": prompt,
                "stream": false
            }))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Ollama request failed: {}", e))?;

        let json: Value = response
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse Ollama response: {}", e))?;

        let result = json["response"].as_str().unwrap_or("").to_string();
        Ok(result)
    }
}

#[async_trait]
impl ReflectionLlm for OllamaLlm {
    /// Reflect on a completed workflow execution.
    /// Sends a structured prompt to Ollama describing the workflow and its metrics.
    async fn reflect(
        &self,
        workflow: &WorkflowGenome,
        metrics: &ExecutionMetrics,
    ) -> anyhow::Result<String> {
        let prompt = format!(
            "You are an AI agent runtime optimizer.\n\
             Reflect on the following workflow execution and suggest improvements.\n\n\
             Workflow ID: {}\n\
             Nodes: {}\n\n\
             Execution Metrics:\n\
             - Latency: {}ms\n\
             - LLM calls: {}\n\
             - Skill calls: {}\n\
             - Memory ops: {}\n\
             - Bus ops: {}\n\
             - Errors: {}\n\
             - Success: {}\n\n\
             Provide a concise reflection and list specific improvements as bullet points starting with '- '.",
            workflow.id,
            workflow.nodes.len(),
            metrics.latency_ms,
            metrics.llm_calls,
            metrics.skill_calls,
            metrics.memory_ops,
            metrics.bus_ops,
            metrics.errors,
            metrics.success,
        );

        self.call_ollama(&prompt).await
    }

    /// Suggest workflow evolution steps given LLM feedback and a genome description.
    async fn evolve_code(&self, feedback: &str, code: &str) -> anyhow::Result<String> {
        let prompt = format!(
            "You are an AI agent workflow optimizer.\n\
             Given the following performance feedback and current workflow description,\n\
             suggest concrete improvements as bullet points starting with '- '.\n\n\
             Feedback:\n{}\n\n\
             Current Workflow:\n{}",
            feedback, code
        );

        self.call_ollama(&prompt).await
    }
}

#[async_trait]
impl LlmInterface for OllamaLlm {
    async fn call(
        &self,
        model: &str,
        prompt: &str,
        params: &Value,
    ) -> crate::hy_evo::node::NodeResult {
        match self.call_ollama(prompt).await {
            Ok(response) => crate::hy_evo::node::NodeResult::Text(response),
            Err(e) => crate::hy_evo::node::NodeResult::Error(format!("LLM call failed: {}", e)),
        }
    }

    async fn summarize(&self, text: &str) -> crate::hy_evo::node::NodeResult {
        let prompt = format!("Summarize the following text concisely:\n\n{}", text);
        match self.call_ollama(&prompt).await {
            Ok(summary) => crate::hy_evo::node::NodeResult::Text(summary),
            Err(e) => crate::hy_evo::node::NodeResult::Error(format!("Summarize failed: {}", e)),
        }
    }
}
