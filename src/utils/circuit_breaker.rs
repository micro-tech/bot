//! Circuit Breaker stub (Task 55)
pub struct CircuitBreaker {
    pub failures: u32,
}

impl CircuitBreaker {
    pub fn new() -> Self {
        Self { failures: 0 }
    }
    pub fn record_failure(&mut self) {
        self.failures += 1;
    }
}
