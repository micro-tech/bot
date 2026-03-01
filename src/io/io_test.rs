// Tests for IO Module in Agent OS
// This file contains tests for the functions in io.rs to ensure proper routing of bus messages.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::bus::{Bus, Message};
    use std::sync::mpsc::channel;
    use std::time::SystemTime;

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

    // TODO: Add more tests for start() and handler functions when mock bus behavior is fully implemented
    // e.g., mocking bus messages and verifying routing to correct handlers
}
