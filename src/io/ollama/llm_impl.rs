use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use super::llm_trait::OllamaLlm;
use crate::cpu::interfaces::LlmInterface;
use crate::hy_evo::node::NodeResult;
use crate::io::ollama::build_client;

/// Concrete implementation of `OllamaLlm` for the Ollama backend.
pub struct OllamaLlmImpl {
    pub base_url: String,
    pub model: String,
    client: Client,
}

impl OllamaLlmImpl {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            client: build_client(),
        }
    }
}

#[async_trait]
impl OllamaLlm for OllamaLlmImpl {
    async fn generate(&self, prompt: &str) -> anyhow::Result<String> {
        let resp = self
            .client
            .post(format!("{}/api/generate", self.base_url))
            .json(&json!({
                "model": self.model,
                "prompt": prompt,
                "stream": false
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Ollama /api/generate failed: {}", body);
        }

        let parsed: Value = resp.json().await?;
        let content = parsed["response"].as_str().unwrap_or("").to_string();
        Ok(content)
    }

    async fn chat(&self, messages: &[Value]) -> anyhow::Result<String> {
        let resp = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&json!({
                "model": self.model,
                "messages": messages,
                "stream": false
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Ollama /api/chat failed: {}", body);
        }

        let parsed: Value = resp.json().await?;
        let content = parsed["message"]["content"].as_str().unwrap_or("").to_string();
        Ok(content)
    }

    async fn chat_with_tools(
        &self,
        messages: &[Value],
        tools_json: Value,
    ) -> anyhow::Result<String> {
        let mut history = messages.to_vec();
        let max_tool_rounds = 10usize;
        let mut tool_rounds = 0usize;

        loop {
            let resp = self
                .client
                .post(format!("{}/api/chat", self.base_url))
                .json(&json!({
                    "model": self.model,
                    "messages": history,
                    "tools": tools_json,
                    "stream": false
                }))
                .send()
                .await?;

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("Ollama chat_with_tools failed: {}", body);
            }

            let parsed: Value = resp.json().await?;
            let msg = &parsed["message"];
            history.push(msg.clone());

            if let Some(tool_calls) = msg["tool_calls"].as_array() {
                tool_rounds += 1;
                if tool_rounds > max_tool_rounds {
                    anyhow::bail!("Ollama tool-call loop exceeded safety limit");
                }

                for tool in tool_calls {
                    let name = tool["function"]["name"].as_str().unwrap_or("");
                    let args = tool["function"]["arguments"].clone();
                    let tool_result = crate::tools::execute(name, &args);

                    history.push(json!({
                        "role": "tool",
                        "tool_call_id": tool["id"],
                        "content": tool_result
                    }));
                }
            } else {
                let content = msg["content"].as_str().unwrap_or("").to_string();
                if content.is_empty() {
                    anyhow::bail!("Ollama returned empty content");
                }
                return Ok(content);
            }
        }
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

// Implement LlmInterface so OllamaLlmImpl can be used with Cpu<L>
#[async_trait]
impl LlmInterface for OllamaLlmImpl {
    async fn call(&self, _model: &str, prompt: &str, _params: &Value) -> NodeResult {
        match self.generate(prompt).await {
            Ok(text) => NodeResult::Text(text),
            Err(e) => NodeResult::Error(e.to_string()),
        }
    }

    async fn summarize(&self, text: &str) -> NodeResult {
        let prompt = format!("Summarize the following text concisely:\n\n{}", text);
        match self.generate(&prompt).await {
            Ok(summary) => NodeResult::Text(summary),
            Err(e) => NodeResult::Error(e.to_string()),
        }
    }
}

// --------------------------- Tests ---------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ollama_llm_impl_model_name() {
        let llm = OllamaLlmImpl::new("http://localhost:11434", "llama3.2");
        assert_eq!(llm.model_name(), "llama3.2");
    }

    #[tokio::test]
    #[ignore]
    async fn test_generate_live() {
        let llm = OllamaLlmImpl::new("http://localhost:11434", "llama3.2");
        let result = llm.generate("Say hello in one word.").await;
        assert!(result.is_ok());
        let text = result.unwrap();
        assert!(!text.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn test_chat_live() {
        let llm = OllamaLlmImpl::new("http://localhost:11434", "llama3.2");
        let messages = vec![json!({"role": "user", "content": "What is 2+2?"})];
        let result = llm.chat(&messages).await;
        assert!(result.is_ok());
    }
}
