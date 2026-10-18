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
use std::sync::{Arc, Mutex, OnceLock};
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

// ── Process-global bus handle (task 230) ──────────────────────────────────────
// The sub-agent registry lives in the tools layer and has no bus handle of its
// own, but must publish `subagent_event` messages. main.rs installs the live
// bus here at startup; publishers degrade to a silent no-op when it is absent
// (unit tests, binaries that never start the bus).
static GLOBAL_BUS: OnceLock<Arc<Bus>> = OnceLock::new();

/// Install the process-global bus. Called once by main.rs; later calls are ignored.
pub fn set_global_bus(bus: Arc<Bus>) {
    let _ = GLOBAL_BUS.set(bus);
}

/// The process-global bus, if one was installed.
pub fn global_bus() -> Option<Arc<Bus>> {
    GLOBAL_BUS.get().cloned()
}

impl Bus {
    /// Creates a new bus instance with router thread.
    pub fn new() -> Self {
        let (sender, receiver) = channel::<Message>();
        let subscribers = Arc::new(Mutex::new(Vec::<(String, Sender<Message>)>::new()));
        let subs_clone = Arc::clone(&subscribers);
        thread::spawn(move || {
            println!("[Bus] Router thread started");
            while let Ok(message) = receiver.recv() {
                // Task 239: recover from poison instead of killing the router thread.
                let subs_guard = subs_clone.lock().unwrap_or_else(|e| e.into_inner());
                let mut delivered = false;
                for (component, sub_sender) in subs_guard.iter() {
                    if component == &message.to {
                        if sub_sender.send(message.clone()).is_ok() {
                            delivered = true;
                        }
                    }
                }
                if !delivered {
                    // Optional: log undelivered messages. Task 265: the
                    // old println! here also dumped a 120-char data
                    // preview — bus data is untrusted content, so the
                    // eprintln! above (routing metadata only) is the
                    // whole report; no duplicate, no preview.
                    eprintln!("[Bus] ⚠️ NO SUBSCRIBER for destination: '{}' (from='{}')", message.to, message.from);
                }
            }
            println!("[Bus] Router thread exited");
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
    /// Task 265 (secrets): metadata only (from/to/length). The old code
    /// wrote the first 300 chars of every message payload into this
    /// tracked file — bus messages carry chat text, drafts, and config,
    /// so that was a standing secret/body leak. Full content stays in
    /// the primary stores (chat history, drafts, notes).
    fn log_transaction(&self, message: &Message) {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string();
        let log_entry = format!(
            "[{}] {} -> {} ({} chars)",
            timestamp,
            message.from,
            message.to,
            message.data.len()
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
