// Gemini Handler for Agent OS
use crate::bus::{Bus, Message};
use log::info;
use reqwest::Client;
use serde_json::json;

pub fn handle_gemini_message(message: Message, _bus: &mut Bus) -> Option<String> {
    info!("Gemini msg: {}", message.data);
    call_gemini(&message.data).ok()
}

pub fn call_gemini(data: &str) -> Result<String, Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let client = Client::new();
        let api_key = "YOUR_GEMINI_KEY";
        let resp = client.post(format!("https://generativelanguage.googleapis.com/v1beta/models/gemini-pro:generateContent?key={}", api_key))
            .json(&json!({"contents": [{"parts": [{"text": data}]}]}))
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?
            ["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string();
        Ok(resp)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_mock_bus() -> Bus {
        Bus::new()
    }

    #[test]
    fn test_handle() {
        let mut bus = create_mock_bus();
        let message = Message {
            to: "gemini".to_string(),
            from: "test".to_string(),
            data: "test msg".to_string(),
            timestamp: 0,
        };
        let response = handle_gemini_message(message, &mut bus);
        assert!(response.is_some());
    }

    #[test]
    fn test_call() {
        let result = call_gemini("test");
        assert!(result.is_ok());
    }
}