// Ollama Handler for Agent OS
use crate::bus::{Bus, Message};
use log::info;
use ollama_rs::Ollama;
use ollama_rs::generation::GenerateRequest;

pub fn handle_ollama_message(message: Message, _bus: &mut Bus) -> Option<String> {
    info!("Ollama msg from {}: {}", message.from, message.data);
    match call_ollama(&message.data) {
        Ok(r) => Some(r),
        Err(_) => None,
    }
}

pub fn call_ollama(data: &str) -> Result<String, Box<dyn std::error::Error>> {
    let client = Ollama::new(r#\"192.168.1.149\"#.to_string(), 11434u16);
    let req = GenerateRequest {
        model: r#\"llama3\"#.to_string(),
        prompt: data.to_string(),
        stream: false,
        ..Default::default()
    };
    let rt = tokio::runtime::Runtime::new()?;
    let resp = rt.block_on(client.generate(req))?;
    Ok(resp.response.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn create_mock_bus() -> Bus {
        Bus::new()
    }

    #[test]
    fn test_handle() {
        let mut bus = create_mock_bus();
        let message = Message {
            to: \"ollama\".to_string(),
            from: \"test\".to_string(),
            data: \"test msg\".to_string(),
            timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64,
        };
        let response = handle_ollama_message(message, &mut bus);
        assert!(response.is_some());
    }

    #[test]
    fn test_call() {
        let result = call_ollama(\"test\");
        assert!(result.is_ok());
    }
}