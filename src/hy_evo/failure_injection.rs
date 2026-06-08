//! Failure Injection Testing (Task 45)
pub fn inject_failure(probability: f64) -> bool {
    rand::random::<f64>() < probability
}
