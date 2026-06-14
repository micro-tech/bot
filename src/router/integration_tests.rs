// router/integration_tests.rs - Integration test harness with mock stores (Task 146.4)

use crate::router::context::{RoutingContext, LLMBackend};
use crate::router::config::RouterConfig;
use crate::router::telemetry::TelemetrySnapshot;
use crate::router::health::HealthStatus;
use crate::router::override_mod::{OverrideStore, OverrideCommand, OverrideTier};
use chrono::Utc;
use std::collections::HashMap;

/// Mock telemetry store for testing
pub struct MockTelemetryStore {
    pub snapshot: Option<TelemetrySnapshot>,
}

impl MockTelemetryStore {
    pub fn new() -> Self {
        Self { snapshot: None }
    }

    pub fn with_snapshot(mut self, snap: TelemetrySnapshot) -> Self {
        self.snapshot = Some(snap);
        self
    }

    pub fn get_latest(&self) -> Option<TelemetrySnapshot> {
        self.snapshot.clone()
    }
}

/// Mock health store for testing
pub struct MockHealthStore {
    pub statuses: HashMap<LLMBackend, HealthStatus>,
}

impl MockHealthStore {
    pub fn new() -> Self {
        Self { statuses: HashMap::new() }
    }

    pub fn set_health(&mut self, backend: LLMBackend, status: HealthStatus) {
        self.statuses.insert(backend, status);
    }

    pub fn get_health(&self, backend: &LLMBackend) -> Option<HealthStatus> {
        self.statuses.get(backend).cloned()
    }
}

/// Mock override store for testing
pub struct MockOverrideStore {
    pub overrides: OverrideStore,
}

impl MockOverrideStore {
    pub fn new() -> Self {
        Self { overrides: OverrideStore::new() }
    }

    pub fn apply(&mut self, cmd: OverrideCommand) {
        self.overrides.apply(cmd);
    }
}

/// Helper to build a test RoutingContext
pub fn build_test_context(
    prompt: &str,
    complexity_score: f32,
    telemetry: Option<TelemetrySnapshot>,
    health: Option<HashMap<LLMBackend, HealthStatus>>,
    user_override: Option<OverrideCommand>,
) -> RoutingContext {
    RoutingContext {
        prompt: prompt.to_string(),
        token_estimate: prompt.len() as u32 / 4,
        has_code: prompt.contains("```"),
        complexity_score,
        timestamp: Utc::now(),
        user_override: user_override.map(|o| o.backend.unwrap_or(LLMBackend::Ollama)),
        telemetry,
        health,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::strategy::route;

    #[test]
    fn test_routing_with_mocked_telemetry() {
        let mut telemetry = MockTelemetryStore::new();
        telemetry.snapshot = Some(TelemetrySnapshot {
            gpu_util: 90.0,
            vram_used: 22.0,
            cpu_load: 5.0,
            temp_c: 78.0,
            rtt_ms: HashMap::new(),
            timestamp: Utc::now(),
        });

        let ctx = build_test_context(
            "Explain quantum computing",
            0.8,
            telemetry.get_latest(),
            None,
            None,
        );

        let config = RouterConfig::default();
        let backend = route(&ctx, &config);
        assert!(matches!(backend, LLMBackend::Gemini | LLMBackend::Ollama));
    }

    #[test]
    fn test_routing_with_mocked_health() {
        let mut health = MockHealthStore::new();
        health.set_health(LLMBackend::Ollama, HealthStatus {
            reachable: false,
            latency_ms: 5000,
            last_success: None,
            error_rate: 0.8,
            degraded: true,
            capabilities: vec![],
        });

        let ctx = build_test_context(
            "Write a Rust function",
            0.6,
            None,
            Some(health.statuses.clone()),
            None,
        );

        let config = RouterConfig::default();
        let backend = route(&ctx, &config);
        // Should avoid Ollama because it's unhealthy
        assert_ne!(backend, LLMBackend::Ollama);
    }
}