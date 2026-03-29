use self::executor::CpuExecutor;
use crate::bus::Bus;
use crate::bus::Message;
use crate::cpu::state::AgentState;
use crate::hy_evo::integration::HyEvoIntegration;
use crate::hy_evo::reflection::ReflectionLlm;
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
        }
    }

    /// Access the HyEvo integration layer, if present.
    pub fn hyevo(&self) -> Option<&HyEvoIntegration<L>> {
        self.hyevo.as_ref()
    }

    /// Handle an incoming bus message.
    pub fn handle_bus_message(&mut self, msg: Message) {
        self.state.bump_tick();
        // TODO: Process the message, perhaps by scheduling instructions or updating state
        println!("Received bus message: {:?}", msg);
    }
}
