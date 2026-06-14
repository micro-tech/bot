// router/observability.rs - Monitoring & Observability (Task 139)
use log::info;
use std::collections::HashMap;
use chrono::Utc;

/// Emit a structured routing decision event
pub fn log_routing_decision(
    prompt_preview: &str,
    chosen: &str,
    reason: &str,
    latency_ms: u64,
    complexity: f32,
) {
    info!(
        target: "router",
        "routing_decision prompt=\"{}\" chosen=\"{}\" reason=\"{}\" latency_ms={} complexity={:.2} ts={}",
        prompt_preview.chars().take(60).collect::<String>(),
        chosen,
        reason,
        latency_ms,
        complexity,
        Utc::now().to_rfc3339()
    );
}

/// Simple in-memory metrics store (can be replaced with Prometheus later)
pub struct RouterMetrics {
    pub decisions: HashMap<String, u64>,
    pub total_latency_ms: u64,
    pub decision_count: u64,
}

impl RouterMetrics {
    pub fn new() -> Self {
        Self {
            decisions: HashMap::new(),
            total_latency_ms: 0,
            decision_count: 0,
        }
    }

    pub fn record(&mut self, backend: &str, latency_ms: u64) {
        *self.decisions.entry(backend.to_string()).or_insert(0) += 1;
        self.total_latency_ms += latency_ms;
        self.decision_count += 1;
    }

    pub fn avg_latency(&self) -> f64 {
        if self.decision_count == 0 {
            0.0
        } else {
            self.total_latency_ms as f64 / self.decision_count as f64
        }
    }
}
