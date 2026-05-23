// Bus System for Agent OS
// This module implements a central communication bus for inter-component messaging.
// Components publish messages to the bus and subscribe to receive relevant messages.

use crate::cpu::interfaces::BusInterface;
use crate::utils::{log_to_file, now_ms};
use chrono::Local;
use log::error;
use serde::Serialize;
use serde_json::Value;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;

/// Represents a message on the bus with source, destination, and data payload.
#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub to: String,     // Destination component (e.g., "ollama", "web_interface")
    pub from: String,   // Source component (e.g., "hartbeat", "terminal")
    pub data: String,   // Payload or data content
    pub timestamp: u64, // Unix timestamp in milliseconds for when the message was created
}

/// Bus system to route messages between components.
pub struct Bus {
    sender: Sender<Message>,
    subscribers: Arc<Mutex<Vec<(String, Sender<Message>)>>>, // List of subscribers (component name, channel sender)
}

pub type BusHandle = std::sync::mpsc::Receiver<Message>;

impl Bus {
    /// Creates a new bus instance with router thread.
    pub fn new() -> Self {
        let (sender, receiver) = channel::<Message>();
        let subscribers = Arc::new(Mutex::new(Vec::<(String, Sender<Message>)>::new()));
        let subs_clone = Arc::clone(&subscribers);
        thread::spawn(move || {
            while let Ok(message) = receiver.recv() {
                let subs_guard = subs_clone.lock().unwrap();
                for (component, sub_sender) in subs_guard.iter() {
                    if component == &message.to {
                        let _ = sub_sender.send(message.clone());
                    }
                }
            }
        });
        Self {
            sender,
            subscribers,
        }
    }

    /// Allows a component to subscribe to messages destined for it.
    pub fn subscribe(&self, component_name: &str) -> Receiver<Message> {
        let (tx, rx) = channel();
        self.subscribers
            .lock()
            .unwrap()
            .push((component_name.to_string(), tx));
        rx
    }

    /// Publishes a message to the bus, logging the transaction.
    pub fn publish(&self, message: Message) -> Result<(), String> {
        self.log_transaction(&message);

        if let Err(e) = self.sender.send(message.clone()) {
            let error_msg = format!("Failed to publish message to bus: {}", e);
            log_to_file(&error_msg);
            error!("{}", error_msg);
            return Err(error_msg);
        }

        Ok(())
    }

    /// Logs bus transactions to logs/bus_log.md
    fn log_transaction(&self, message: &Message) {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string();
        let summary = if message.data.len() > 300 {
            format!("{}...", &message.data[..300])
        } else {
            message.data.clone()
        };

        let log_entry = format!(
            "[{}] {} -> {}: {}",
            timestamp, message.from, message.to, summary
        );

        // Try to write to bus_log.md, fall back gracefully on error
        if let Ok(mut file) = OpenOptions::new()
            .append(true)
            .create(true)
            .open("logs/bus_log.md")
        {
            if let Err(e) = writeln!(file, "{}", log_entry) {
                eprintln!("Warning: Failed to write to bus_log.md: {}", e);
            }
        } else {
            eprintln!("Warning: Could not open logs/bus_log.md for writing");
        }
    }
}

impl Clone for Bus {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            subscribers: Arc::clone(&self.subscribers),
        }
    }
}

#[async_trait::async_trait]
impl BusInterface for Bus {
    async fn publish(&self, to: &str, data: Value) -> crate::hy_evo::node::NodeResult {
        let msg = Message {
            to: to.to_string(),
            from: "cpu".to_string(),
            data: data.to_string(),
            timestamp: now_ms(),
        };
        match self.publish(msg) {
            Ok(()) => crate::hy_evo::node::NodeResult::None,
            Err(e) => crate::hy_evo::node::NodeResult::Error(e),
        }
    }
}
