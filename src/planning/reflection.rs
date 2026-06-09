//! Reflection — analyzes past execution and suggests improvements using LLM.
use crate::cpu::interfaces::LlmInterface;
use crate::io::ollama::llm::OllamaLlm;
#[allow(unused_imports)]
use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait ReflectionInterface: Send + Sync {
    async fn reflect(&self, outcome: &str, metrics: Option<&str>) -> Result<String>;
}

pub struct Reflector {
    llm: OllamaLlm,
}

impl Reflector {
    pub fn new(llm: OllamaLlm) -> Self {
        Self { llm }
    }
}

#[async_trait]
impl ReflectionInterface for Reflector {
    async fn reflect(&self, outcome: &str, metrics: Option<&str>) -> Result<String> {
        let prompt = format!(
            "You are an expert AI agent analyst. Analyze the following execution outcome and provide:\n\
             1. What went well\n\
             2. What could be improved\n\
             3. Specific actionable suggestions\n\n\
             Execution outcome:\n{}\n\n\
             Metrics (if available):\n{}\n\n\
             Return your analysis in clear bullet points.",
            outcome,
            metrics.unwrap_or("No metrics provided")
        );

        let response = match self.llm.call("default", &prompt, &serde_json::json!({})).await {
            crate::hy_evo::node::NodeResult::Text(text) => text,
            crate::hy_evo::node::NodeResult::Error(e) => return Err(anyhow::anyhow!(e)),
            _ => return Err(anyhow::anyhow!("Unexpected LLM response")),
        };
        Ok(response)
    }
}
