// Bus System for Agent OS
// This module implements a central communication bus for inter-component messaging.
// Components publish messages to the bus and subscribe to receive relevant messages.

use crate::cpu::interfaces::BusInterface;
use crate::utils::{log_to_file, now_ms};
use chrono::Local;
use log::error;
use serde::Serialize;
use serde_json::Value;
use std::sync::mpsc::{Sender, channel};
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
/// One subscriber: (component name, channel sender).
type Subscriber = (String, tokio::sync::mpsc::UnboundedSender<Message>);

pub struct Bus {
    sender: Sender<Message>,
    subscribers: Arc<Mutex<Vec<Subscriber>>>, // List of subscribers (component name, channel sender)
}

// H6 (task 383): subscribers are now tokio::sync::mpsc receivers. The old
// std::sync::mpsc receivers blocked a Tokio worker forever on recv(), so
// four idle listeners could pin every worker on a 4-core box and starve
// the runtime. The router's inbound channel stays std mpsc (it lives on a
// plain thread::spawn router, not a runtime worker).
pub type BusHandle = tokio::sync::mpsc::UnboundedReceiver<Message>;

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

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus {
    /// Creates a new bus instance with router thread.
    pub fn new() -> Self {
        let (sender, receiver) = channel::<Message>();
        let subscribers = Arc::new(Mutex::new(Vec::<Subscriber>::new()));
        let subs_clone = Arc::clone(&subscribers);
        thread::spawn(move || {
            log::info!("[Bus] Router thread started");
            while let Ok(message) = receiver.recv() {
                // Task 239: recover from poison instead of killing the router thread.
                let subs_guard = subs_clone.lock().unwrap_or_else(|e| e.into_inner());
                let mut delivered = false;
                for (component, sub_sender) in subs_guard.iter() {
                    if component == &message.to && sub_sender.send(message.clone()).is_ok() {
                        delivered = true;
                    }
                }
                if !delivered {
                    // Optional: log undelivered messages. Task 265: the
                    // old println! here also dumped a 120-char data
                    // preview — bus data is untrusted content, so the
                    // eprintln! above (routing metadata only) is the
                    // whole report; no duplicate, no preview.
                    log::error!(
                        "[Bus] ⚠️ NO SUBSCRIBER for destination: '{}' (from='{}')",
                        message.to,
                        message.from
                    );
                }
            }
            log::info!("[Bus] Router thread exited");
        });
        Self {
            sender,
            subscribers,
        }
    }

    /// Allows a component to subscribe to messages destined for it.
    pub fn subscribe(&self, component_name: &str) -> tokio::sync::mpsc::UnboundedReceiver<Message> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
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
    /// Task 316: batched append (cached open handle in utils) instead of
    /// open-write-close per message; the per-token bus rate made the old
    /// pattern the hot path. Line content is identical.
    fn log_transaction(&self, message: &Message) {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string();
        let log_entry = format!(
            "[{}] {} -> {} ({} chars)\n",
            timestamp,
            message.from,
            message.to,
            message.data.len()
        );

        crate::utils::append_log_line(&std::path::PathBuf::from("logs/bus_log.md"), &log_entry);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// H6 (task 383): bus listeners used to block a Tokio worker forever on
    /// std `recv()`. Four listeners pinned four workers on a 4-core box and
    /// starved the runtime. The bus is now `tokio::sync::mpsc`, so an idle
    /// listener yields its worker: even a 1-worker runtime makes progress.
    #[tokio::test]
    async fn idle_bus_listeners_do_not_starve_runtime() {
        let bus = Bus::new();
        let mut rx = bus.subscribe("test_harness");
        // Idle listener: waits for a message that never comes.
        let listener = tokio::spawn(async move { while let Some(_msg) = rx.recv().await {} });
        // On a 1-worker (current-thread) runtime this completes only if the
        // listener yields its worker while idle — a blocking recv would wedge it.
        let done = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::task::yield_now().await;
            42
        })
        .await;
        assert_eq!(done, Ok(42), "runtime starved by idle bus listener");
        listener.abort();

        // Delivery still works on the async channel.
        let bus2 = Bus::new();
        let mut rx2 = bus2.subscribe("web_interface");
        bus2.publish(Message {
            to: "web_interface".into(),
            from: "test".into(),
            data: "hello".into(),
            timestamp: 0,
        })
        .unwrap();
        let got = tokio::time::timeout(Duration::from_secs(5), rx2.recv()).await;
        assert!(got.is_ok(), "published message must arrive");
        assert_eq!(got.unwrap().expect("channel open").data, "hello");
    }
}
