// Ollama Handler for Agent OS
// This module handles communication with the Ollama API.

use crate::bus::{Bus, Message};
use crate::utils::log_to_file;
use std::error::Error;
use log::error;

/// Handles a message destined for Ollama, returning a response if applicable.
pub fn handle_ollama_message(message: Message, _bus: &mut Bus) -> Option<String> {
    // Placeholder implementation
    println!("Handling Ollama message: {}", message.data);
    Some(format!("Ollama response to {}: {}", message.from, message.data))
}

/// Calls the Ollama API with the provided data, returning the response.
pub fn call_ollama(data: &str) -> Result<String, Box<dyn Error>> {
    // Placeholder implementation
    Ok(format!("Ollama response: {}", data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    // Helper function to create a mock bus for testing
    fn create_mock_bus() -> Bus {
        Bus::new()
    }

    #[test]
    fn test_handle_ollama_message() {
        // Test handling a message destined for Ollama
        let mut bus = create_mock_bus();
        let message = Message {
            to: "ollama".to_string(),
            from: "hartbeat".to_string(),
            data: "Test heartbeat data".to_string(),
            timestamp: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        };
        
        let response = handle_ollama_message(message.clone(), &mut bus);
        
        // Verify that a response is generated
        assert!(response.is_some(), "handle_ollama_message should return a response");
        let response_str = response.unwrap();
        assert!(response_str.contains("Ollama response"), "Response should contain expected text");
        assert!(response_str.contains(&message.from), "Response should reference the sender");
        assert!(response_str.contains(&message.data), "Response should reference the data");
    }

    #[test]
    fn test_call_ollama() {
        // Test the direct call to Ollama API (placeholder)
        let data = "Test data for Ollama";
        let result = call_ollama(data);
        
        // Verify that the call returns a successful result
        if result.is_err() {
            let error_msg = format!("call_ollama failed: {}", result.unwrap_err());
            log_to_file(&error_msg);
            error!("{}", error_msg);
        }
        assert!(result.is_ok(), "call_ollama should return Ok result");
        let response = result.unwrap();
        assert!(response.contains("Ollama response"), "Response should contain expected text");
        assert!(response.contains(data), "Response should include input data");
    }

    // TODO: Add more tests with mocked HTTP responses when actual API logic is implemented
}
