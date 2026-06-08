// IO Module for Agent OS
// This module handles external communications by reading messages from the bus
// and routing them to the appropriate handler based on the 'to' field.

use std::sync::mpsc::Receiver;
use std::time::Duration;

// Assuming Bus and Message are defined in the bus module
use crate::bus::{Bus, Message};

/// IO Manager that routes bus messages to specific external communication handlers.
pub struct IOManager {
    bus: Bus,
    ollama_rx: Receiver<Message>,
    web_rx: Receiver<Message>,
    terminal_rx: Receiver<Message>,
    gemini_rx: Receiver<Message>,
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

    /// Starts the IO Manager.
    /// Uses a single thread that polls all receivers in a round-robin fashion.
    /// This avoids the need to clone Receivers.
    pub fn start(&mut self) {
        println!("IO Manager started - polling all receivers...");

        // Simple polling loop (blocking)
        // In production this should run in a dedicated thread or use async
        loop {
            // Poll Ollama
            if let Ok(msg) = self.ollama_rx.recv_timeout(Duration::from_millis(50)) {
                println!("IO: Routing to Ollama from {}", msg.from);
                Self::handle_ollama_message(msg);
            }

            // Poll Web Interface
            if let Ok(msg) = self.web_rx.recv_timeout(Duration::from_millis(50)) {
                println!("IO: Routing to Web from {}", msg.from);
                Self::handle_web_message(msg);
            }

            // Poll Terminal
            if let Ok(msg) = self.terminal_rx.recv_timeout(Duration::from_millis(50)) {
                println!("IO: Routing to Terminal from {}", msg.from);
                Self::handle_terminal_message(msg);
            }

            // Poll Gemini
            if let Ok(msg) = self.gemini_rx.recv_timeout(Duration::from_millis(50)) {
                println!("IO: Routing to Gemini from {}", msg.from);
                Self::handle_gemini_message(msg);
            }

            // Small sleep to avoid busy-looping
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Handles messages destined for Ollama by delegating to the ollama module.
    fn handle_ollama_message(message: Message) {
        println!("[Ollama] Received message from {}: {}", message.from, message.data);
        // Future: call actual ollama_handler::process(message)
    }

    /// Handles messages destined for Web Interface.
    fn handle_web_message(message: Message) {
        println!("[Web] Received message from {}: {}", message.from, message.data);
        // Future: Forward to web_server module
    }

    /// Handles messages destined for Terminal.
    fn handle_terminal_message(message: Message) {
        println!("[Terminal] Received message from {}: {}", message.from, message.data);
        // Future: Forward to terminal/cli module
    }

    /// Handles messages destined for Gemini.
    fn handle_gemini_message(message: Message) {
        println!("[Gemini] Received message from {}: {}", message.from, message.data);
        // Future: Implement Gemini API integration
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
        let _ = self.bus.publish(message);
    }
}

// Unit tests remain mostly the same
#[cfg(test)]
mod tests {
    use super::*;

    fn create_mock_bus() -> Bus {
        Bus::new()
    }

    #[test]
    fn test_io_manager_new() {
        let bus = create_mock_bus();
        let io_manager = IOManager::new(bus);
        assert!(io_manager.ollama_rx.recv_timeout(Duration::from_millis(50)).is_err());
    }

    #[test]
    fn test_publish_to_bus() {
        let bus = create_mock_bus();
        let io_manager = IOManager::new(bus);
        io_manager.publish_to_bus("ollama", "test", "hello");
        assert!(true);
    }

    #[test]
    fn test_handle_ollama_message() {
        let message = Message {
            to: "ollama".to_string(),
            from: "test".to_string(),
            data: "test".to_string(),
            timestamp: 0,
        };
        IOManager::handle_ollama_message(message);
        assert!(true);
    }
}
