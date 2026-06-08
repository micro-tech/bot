//! Failure Injection Testing (Task 45)

pub struct FailureInjector {
    pub failure_rate: f64,
}

impl FailureInjector {
    pub fn new(rate: f64) -> Self {
        Self { failure_rate: rate.clamp(0.0, 1.0) }
    }

    pub fn should_fail(&self) -> bool {
        rand::random::<f64>() < self.failure_rate
    }

    pub fn inject_error(&self, operation: &str) -> Option<String> {
        if self.should_fail() {
            Some(format!("Injected failure in {}", operation))
        } else {
            None
        }
    }
}
