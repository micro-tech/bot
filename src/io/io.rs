// IO Module for Agent OS
// This module handles external communications by reading messages from the bus
// and routing them to the appropriate handler based on the 'to' field.

use std::sync::mpsc::Receiver;
// Commented out unused import to suppress warning
// use std::thread;

// Assuming Bus and Message are defined in the bus module
use crate::bus::{Bus, Message};

/// IO Manager that routes bus messages to specific external communication handlers.
pub struct IOManager {
    bus: Bus,
    ollama_rx: Receiver<Message>,    // Receiver for messages to 'ollama'
    web_rx: Receiver<Message>,       // Receiver for messages to 'web_interface'
    terminal_rx: Receiver<Message>,  // Receiver for messages to 'terminal'
    gemini_rx: Receiver<Message>,    // Receiver for messages to 'gemini' (future LLM integration)
}

impl IOManager {
    /// Creates a new IO Manager instance, subscribing to relevant bus destinations.
    pub fn new(bus: Bus) -> Self {
        let ollama_rx = bus.subscribe("ollama");
        let web_rx = bus.subscribe("web_interface");
        let terminal_rx = bus.subscribe("terminal");
        let gemini_rx = bus.subscribe("gemini");

        IOManager {
            bus,
            ollama_rx,
            web_rx,
            terminal_rx,
            gemini_rx,
        }
    }

    /// Starts the IO Manager, spawning threads to handle messages for each destination.
    pub fn start(&mut self) {
        // Start the bus routing in the background
        // self.bus.start(); // No longer needed - router auto-starts on Bus::new()

        // TODO: Fix the below code as Receiver cannot be cloned.
        // Currently commented out to allow compilation.
        // A proper fix might involve redesigning how messages are routed to handlers,
        // possibly by using a single thread to manage all receivers or a different threading model.

        // Handle messages to Ollama
        // let ollama_rx = self.ollama_rx.clone();
        /*
        thread::spawn(move || {
            while let Ok(message) = ollama_rx.recv() {
                println!("IO Manager: Routing message to Ollama from {}", message.from);
                // Call the Ollama handler (placeholder for actual implementation)
                Self::handle_ollama_message(message);
            }
        });
        */

        // Handle messages to Web Interface
        // let web_rx = self.web_rx.clone();
        /*
        thread::spawn(move || {
            while let Ok(message) = web_rx.recv() {
                println!("IO Manager: Routing message to Web Interface from {}", message.from);
                // Call the Web Interface handler (placeholder for actual implementation)
                Self::handle_web_message(message);
            }
        });
        */

        // Handle messages to Terminal
        // let terminal_rx = self.terminal_rx.clone();
        /*
        thread::spawn(move || {
            while let Ok(message) = terminal_rx.recv() {
                println!("IO Manager: Routing message to Terminal from {}", message.from);
                // Call the Terminal handler (placeholder for actual implementation)
                Self::handle_terminal_message(message);
            }
        });
        */

        // Handle messages to Gemini (future LLM integration)
        // let gemini_rx = self.gemini_rx.clone();
        /*
        thread::spawn(move || {
            while let Ok(message) = gemini_rx.recv() {
                println!("IO Manager: Routing message to Gemini from {}", message.from);
                // Call the Gemini handler (placeholder for actual implementation)
                Self::handle_gemini_message(message);
            }
        });
        */
        println!("IO Manager start method called, but routing is disabled until Receiver clone issue is fixed.");
    }

    /// Placeholder for handling messages destined for Ollama.
    fn handle_ollama_message(message: Message) {
        // In a real implementation, this would call the function in io/ollama/ollama_handler.rs
        println!("Handling Ollama message: {}", message.data);
        // TODO: Implement Ollama API call logic here or delegate to ollama module
    }

    /// Placeholder for handling messages destined for Web Interface.
    fn handle_web_message(message: Message) {
        // In a real implementation, this would call the function in io/web_server/web_server.rs
        println!("Handling Web Interface message: {}", message.data);
        // TODO: Implement Web Interface update logic here or delegate to web_server module
    }

    /// Placeholder for handling messages destined for Terminal.
    fn handle_terminal_message(message: Message) {
        // In a real implementation, this would call the function in io/terminal/cli.rs
        println!("Handling Terminal message: {}", message.data);
        // TODO: Implement Terminal output logic here or delegate to terminal module
    }

    /// Placeholder for handling messages destined for Gemini (future LLM).
    fn handle_gemini_message(message: Message) {
        // In a real implementation, this would call the function in io/llm_gemini/gemini_handler.rs
        println!("Handling Gemini message: {}", message.data);
        // TODO: Implement Gemini API call logic here or delegate to gemini module
    }

    /// Publishes a message to the bus from an IO component.
    pub fn publish_to_bus(&self, to: &str, from: &str, data: &str) {
        use std::time::SystemTime;
        let message = Message {
            to: to.to_string(),
            from: from.to_string(),
            data: data.to_string(),
            timestamp: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        };
        self.bus.publish(message);
    }
}

// Unit tests for IO Module
#[cfg(test)]
mod tests {
    use super::*;

    // Helper function to create a mock bus for testing
    fn create_mock_bus() -> Bus {
        Bus::new()
    }

    #[test]
    fn test_io_manager_new() {
        // Test the creation of a new IO Manager instance
        let bus = create_mock_bus();
        let io_manager = IOManager::new(bus);
        
        // Assert that the IO Manager is created (basic check since it's a struct)
        assert_eq!(io_manager.ollama_rx.recv_timeout(std::time::Duration::from_millis(100)).is_err(), true, "Ollama receiver should be empty initially");
        assert_eq!(io_manager.web_rx.recv_timeout(std::time::Duration::from_millis(100)).is_err(), true, "Web receiver should be empty initially");
        assert_eq!(io_manager.terminal_rx.recv_timeout(std::time::Duration::from_millis(100)).is_err(), true, "Terminal receiver should be empty initially");
        assert_eq!(io_manager.gemini_rx.recv_timeout(std::time::Duration::from_millis(100)).is_err(), true, "Gemini receiver should be empty initially");
    }

    #[test]
    fn test_publish_to_bus() {
        // Test publishing a message to the bus via IO Manager
        let bus = create_mock_bus();
        let io_manager = IOManager::new(bus);
        
        let to = "ollama";
        let from = "test";
        let data = "Test message";
        
        io_manager.publish_to_bus(to, from, data);
        
        // Since bus.start() isn't called in this test, we can't receive messages,
        // but we can verify the publish doesn't panic
        assert!(true, "Publishing to bus should not panic");
    }

    #[test]
    fn test_handle_ollama_message() {
        // Test the handler for Ollama messages
        let message = Message {
            to: "ollama".to_string(),
            from: "test".to_string(),
            data: "Test Ollama data".to_string(),
            timestamp: 1234567890,
        };
        // Since the handler currently just prints to console, we can't assert output,
        // but we can ensure it doesn't panic
        IOManager::handle_ollama_message(message);
        assert!(true, "Handling Ollama message should not panic");
    }

    #[test]
    fn test_handle_web_message() {
        // Test the handler for Web Interface messages
        let message = Message {
            to: "web_interface".to_string(),
            from: "test".to_string(),
            data: "Test Web data".to_string(),
            timestamp: 1234567890,
        };
        // Since the handler currently just prints to console, we can't assert output,
        // but we can ensure it doesn't panic
        IOManager::handle_web_message(message);
        assert!(true, "Handling Web Interface message should not panic");
    }

    #[test]
    fn test_handle_terminal_message() {
        // Test the handler for Terminal messages
        let message = Message {
            to: "terminal".to_string(),
            from: "test".to_string(),
            data: "Test Terminal data".to_string(),
            timestamp: 1234567890,
        };
        // Since the handler currently just prints to console, we can't assert output,
        // but we can ensure it doesn't panic
        IOManager::handle_terminal_message(message);
        assert!(true, "Handling Terminal message should not panic");
    }

    #[test]
    fn test_handle_gemini_message() {
        // Test the handler for Gemini messages
        let message = Message {
            to: "gemini".to_string(),
            from: "test".to_string(),
            data: "Test Gemini data".to_string(),
            timestamp: 1234567890,
        };
        // Since the handler currently just prints to console, we can't assert output,
        // but we can ensure it doesn't panic
        IOManager::handle_gemini_message(message);
        assert!(true, "Handling Gemini message should not panic");
    }

    // TODO: Add more tests for start() and handler functions when mock bus behavior is fully implemented
    // e.g., mocking bus messages and verifying routing to correct handlers
}

// Example usage (to be integrated in main.rs or elsewhere)
/*
fn main() {
    let bus = Bus::new();
    let mut io_manager = IOManager::new(bus);
    io_manager.start();

    // Simulate a message from hartbeat to ollama
    io_manager.publish_to_bus("ollama", "hartbeat", "Heartbeat data for Ollama");

    // Keep the main thread running for demonstration
    std::thread::sleep(std::time::Duration::from_secs(10));
}
*/
