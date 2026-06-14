// router/e2e_tests.rs - End-to-end integration tests (Task 146.7)

use crate::router::context::{RoutingContext, LLMBackend};
use crate::router::config::RouterConfig;
use crate::router::telemetry::TelemetrySnapshot;
use crate::router::health::HealthStatus;
use crate::router::override_mod::{OverrideStore, OverrideCommand, OverrideTier};
use crate::router::strategy::route;
use chrono::Utc;
use std::collections::HashMap;

/// Full end-to-end routing decision with all context sources
fn decide_backend_full(
    prompt: &str,
    overrides: &OverrideStore,
    telemetry: Option<TelemetrySnapshot>,
    health: Option<HashMap<LLMBackend, HealthStatus>>,
    config: &RouterConfig,
) -> LLMBackend {
    let mut ctx = RoutingContext {
        prompt: prompt.to_string(),
        token_estimate: (prompt.len() / 4) as u32,
        has_code: prompt.contains("```"),
        complexity_score: 0.5, // Would normally come from complexity engine
        timestamp: Utc::now(),
        user_override: None,
        telemetry: telemetry.clone(),
        health: health.clone(),
    };

    // Apply override if present
    if let Some(backend) = overrides.resolve() {
        ctx.user_override = Some(backend);
    }

    route(&ctx, config)
}

#[cfg(test)]
mod e2e_tests {
    use super::*;

    #[test]
    fn e2e_normal_routing_no_overrides() {
        let overrides = OverrideStore::new();
        let config = RouterConfig::default();

        let backend = decide_backend_full(
            "Explain how photosynthesis works",
            &overrides,
            None,
            None,
            &config,
        );

        // Should return a valid backend
        assert!(matches!(backend, LLMBackend::Ollama | LLMBackend::Gemini | LLMBackend::Grok));
    }

    #[test]
    fn e2e_user_override_takes_precedence() {
        let mut overrides = OverrideStore::new();
        overrides.apply(OverrideCommand {
            user_id: "test".into(),
            backend: Some(LLMBackend::Grok),
            tier: OverrideTier::OneShot,
            ttl_seconds: None,
        });

        let config = RouterConfig::default();

        let backend = decide_backend_full(
            "Simple hello",
            &overrides,
            None,
            None,
            &config,
        );

        assert_eq!(backend, LLMBackend::Grok);
    }

    #[test]
    fn e2e_avoid_unhealthy_backend() {
        let mut health = HashMap::new();
        health.insert(LLMBackend::Ollama, HealthStatus {
            reachable: false,
            latency_ms: 9999,
            last_success: None,
            error_rate: 0.9,
            degraded: true,
            capabilities: vec![],
        });

        let overrides = OverrideStore::new();
        let config = RouterConfig::default();

        let backend = decide_backend_full(
            "Write a sorting algorithm",
            &overrides,
            None,
            Some(health),
            &config,
        );

        // Should not choose Ollama when it's unhealthy
        assert_ne!(backend, LLMBackend::Ollama);
    }

    #[test]
    fn e2e_high_load_prefers_remote() {
        let telemetry = TelemetrySnapshot {
            gpu_util: 95.0,
            vram_used: 23.5,
            cpu_load: 12.0,
            temp_c: 89.0,
            rtt_ms: HashMap::new(),
            timestamp: Utc::now(),
        };

        let overrides = OverrideStore::new();
        let config = RouterConfig::default();

        let backend = decide_backend_full(
            "Complex multi-step reasoning task",
            &overrides,
            Some(telemetry),
            None,
            &config,
        );

        // Under high load, should prefer Gemini or Grok
        assert!(matches!(backend, LLMBackend::Gemini | LLMBackend::Grok));
    }

    #[test]
    fn e2e_deterministic_same_input_same_output() {
        let overrides = OverrideStore::new();
        let config = RouterConfig::default();

        let b1 = decide_backend_full("Test prompt", &overrides, None, None, &config);
        let b2 = decide_backend_full("Test prompt", &overrides, None, None, &config);

        assert_eq!(b1, b2);
    }
}