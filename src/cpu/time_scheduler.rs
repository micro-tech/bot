use crate::cpu::Cpu;
use crate::hy_evo::reflection::ReflectionLlm;
use std::sync::{Arc, Mutex};
use tokio::time::{Duration, interval};

/// Basic TimeScheduler for periodic events (e.g., every ms).
pub struct TimeScheduler;

impl TimeScheduler {
    /// Start the heartbeat scheduler loop.
    /// Fires every `interval_ms` milliseconds.
    /// The `cpu` handle can be used for periodic tasks such as triggering reflection.
    pub async fn start<L: ReflectionLlm + Send + Sync>(cpu: Arc<Mutex<Cpu<L>>>, interval_ms: u64) {
        let mut tick = interval(Duration::from_millis(interval_ms));
        loop {
            tick.tick().await;
            // Placeholder: log the tick.
            // Future: call cpu.executor or push Instruction::ReflectOnLastStep to cpu.bus.
            println!(
                "Time tick: {}ms - Triggering periodic reflection",
                interval_ms
            );
        }
    }
}
