// cpu/mod.rs

pub mod executor;
pub mod instructions;
pub mod interrupts;
pub mod scheduler;
pub mod state;

use executor::CpuExecutor;
use interrupts::Interrupts;
use scheduler::Scheduler;
use state::AgentState;
use std::sync::Arc;
use log::debug;

pub struct Cpu {
    state: AgentState,
    executor: CpuExecutor,
    bus: Arc<crate::bus::Bus>,
    receiver: crate::bus::BusHandle,
}

impl Cpu {
    pub fn new(
        executor: CpuExecutor,
        bus: Arc<crate::bus::Bus>,
        receiver: crate::bus::BusHandle,
    ) -> Self {
        Self {
            state: AgentState::new(),
            executor,
            bus,
            receiver,
        }
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        loop {
            debug!("Polling for events...");
            let event = Interrupts::poll_next_event(&self.receiver).await;

            let event_desc = event.as_ref().map(|e| format!("{:?}", e.kind));
            debug!("Received event: {:?}", event_desc);

            debug!("Scheduling instructions for mode {:?}, event {:?}", self.state.mode, event_desc);
            let instructions = Scheduler::schedule(&self.state, event);

            debug!("Scheduled {} instructions", instructions.len());
            for instr in instructions {
                self.executor.execute(&mut self.state, instr).await?;
                self.state.bump_step();
                debug!("Step bumped to {}", self.state.step_counter);
            }

            tokio::task::yield_now().await;
        }
    }
}