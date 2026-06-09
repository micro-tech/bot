//! Planner — decomposes high-level goals into subtasks using LLM.
use crate::cpu::interfaces::LlmInterface;
use crate::io::ollama::llm::OllamaLlm;
#[allow(unused_imports)]
use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait PlannerInterface: Send + Sync {
    async fn decompose(&self, goal: &str) -> Result<Vec<String>>;
}

pub struct Planner {
    llm: OllamaLlm,
}

impl Planner {
    pub fn new(llm: OllamaLlm) -> Self {
        Self { llm }
    }
}

#[async_trait]
impl PlannerInterface for Planner {
    async fn decompose(&self, goal: &str) -> Result<Vec<String>> {
        let prompt = format!(
            "You are an expert task planner. Break down the following goal into 3-7 clear, actionable subtasks.\n\
             Return ONLY a JSON array of strings, nothing else.\n\nGoal: {}",
            goal
        );

        // Use the public LlmInterface method
        let response = match self.llm.call("default", &prompt, &serde_json::json!({})).await {
            crate::hy_evo::node::NodeResult::Text(text) => text,
            crate::hy_evo::node::NodeResult::Error(e) => return Err(anyhow::anyhow!(e)),
            _ => return Err(anyhow::anyhow!("Unexpected LLM response")),
        };
        
        // Try to parse JSON array from response
        let subtasks: Vec<String> = match serde_json::from_str(&response) {
            Ok(list) => list,
            Err(_) => {
                // Fallback: split by lines if not valid JSON
                response
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| l.trim_start_matches("- ").trim_start_matches("* ").to_string())
                    .collect()
            }
        };

        if subtasks.is_empty() {
            return Ok(vec![format!("Step 1: {}", goal)]);
        }

        Ok(subtasks)
    }
}
