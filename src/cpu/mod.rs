use self::executor::CpuExecutor;
use crate::bus::Bus;
use crate::bus::Message;
use crate::cpu::state::AgentState;
use crate::hy_evo::integration::HyEvoIntegration;
use crate::hy_evo::reflection::ReflectionLlm;
use crate::serde_json::Value;
use crate::serde_json::json;
use crate::utils::now_ms;
use crate::memory::MemoryManager;

use std::sync::Arc;

pub mod executor;
pub mod instructions;
pub mod interfaces;
pub mod interrupts;
pub mod scheduler;
pub mod state;
pub mod time_scheduler;

// Re-export commonly used types
pub use instructions::Instruction;

/// The main CPU struct, generic over the LLM type used by HyEvo.
pub struct Cpu<L: ReflectionLlm + Send + Sync> {
    pub state: AgentState,
    pub executor: CpuExecutor,
    pub bus: Arc<Bus>,
    pub llm: L,
    pub hyevo: Option<HyEvoIntegration<L>>,
    pub memory: MemoryManager,
}

impl<L: ReflectionLlm + Send + Sync> Cpu<L> {
    /// Construct a new Cpu with an executor, bus, LLM, and HyEvo integration.
    pub fn new(executor: CpuExecutor, bus: Arc<Bus>, llm: L, hyevo: HyEvoIntegration<L>) -> Self {
        Self {
            state: AgentState::new(),
            executor,
            bus,
            llm,
            hyevo: Some(hyevo),
            memory: MemoryManager::new(1000, 1000),
        }
    }

    /// Access the HyEvo integration layer, if present.
    pub fn hyevo(&self) -> Option<&HyEvoIntegration<L>> {
        self.hyevo.as_ref()
    }

    /// Handle an incoming bus message.
    pub fn handle_bus_message(&mut self, msg: Message) {
        self.state.bump_tick();

        // Parse JSON payload
        let payload: Value = match serde_json::from_str(&msg.data) {
            Ok(v) => v,
            Err(_) => {
                // Not JSON → ignore or log
                println!("CPU received non-JSON message: {:?}", msg.data);
                return;
            }
        };

        // Extract message type FIRST
        let msg_type = payload["type"].as_str().unwrap_or("");

        // ---- USER INPUT HANDLING ----
        if msg_type == "user_input" {
            let content_type = payload["content_type"].as_str().unwrap_or("");
            let content = payload["content"].as_str().unwrap_or("");

            if content_type == "text" {
                self.memory.record_user_message(content);
                let facts = self.memory.search_facts(&content, 5);

                // inject facts into prompt later

                // Build LLM request
                let prompt = if facts.is_empty() {
                    content.to_string()
                } else {
                    format!("{}\n\nRelevant facts:\n{}", content, facts.join("\n"))
                };
                let llm_msg = Message {
                    to: "ollama".to_string(),
                    from: "cpu".to_string(),
                    data: json!({
                        "type": "llm_request",
                        "prompt": prompt,
                        "correlation_id": payload["correlation_id"]
                    })
                    .to_string(),
                    timestamp: now_ms(),
                };

                let _ = self.bus.publish(llm_msg);
                return;
            }
        }

        // ---- FALLBACK DEBUG ----
        println!("CPU received bus message: {:?}", msg);
    }
}
