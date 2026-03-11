// Gemini Handler for Agent OS
// This module handles communication with the Gemini API.

use crate::bus::{Bus, Message};
use log::info;
use reqwest::Client;
use serde_json::json;

/// Handles a message destined for Gemini, returning a response if applicable.
pub fn handle_gemini_message(message: Message, _bus: &mut Bus) -> Option<String> {
    info!("Handling Gemini message: {}", message.data);
    let resp = call_gemini(&message.data);
    match resp {
        Ok(r) => Some(r),
        Err(_) => None,
    }
}

/// Calls the Gemini API with the provided data, returning the response.
pub fn call_gemini(data: &str) -> Result<String, Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let client = Client::new();
        let api_key = "YOUR_GEMINI_KEY"; // From config.toml
        let resp = client.post(format!("https://generativelanguage.googleapis.com/v1beta/models/gemini-pro:generateContent?key={}", api_key))
            .json(&json!({"contents": [{"parts": [{"text": data}]}]}))
            .send()
            .await?
            .text()
            .await?;
        Ok(resp)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn create_mock_bus() -> Bus {
        Bus::new()
    }

    #[test]
    fn test_handle_gemini_message() {
        let mut bus = create_mock_bus();
        let message = Message {
            to: "gemini".to_string(),
            from: "hartbeat".to_string(),
            data: "Test data".to_string(),
            timestamp: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        };
        
        let response = handle_gemini_message(message, &mut bus);
        
        assert!(response.is_some());
    }

    #[test]
    fn test_call_gemini() {
        let data = "Test data";
        let result = call_gemini(data);
        assert!(result.is_ok());
    }
}