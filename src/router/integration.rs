// router/integration.rs - Full Integration & Testing (Task 135)
use crate::router::{
    route, LLMBackend, RoutingContext, RouterConfig,
    resolve_with_fallback, BackendSelection,
    ConfigManager, OverrideStore, OverrideCommand, OverrideTier,
    HealthStore, TelemetryCollector,
};

/// Main integration point for CPU pipeline
pub fn decide_backend_with_full_context(
    prompt: &str,
    token_estimate: usize,
    has_code: bool,
    user_id: Option<&str>,
    config_manager: &ConfigManager,
    override_store: &OverrideStore,
    health_store: Option<&HealthStore>,
) -> BackendSelection {
    let config = config_manager.get_blocking(); // or await in real async context

    let mut ctx = RoutingContext {
        prompt: prompt.to_string(),
        token_estimate,
        has_code,
        complexity_score: if has_code { 0.75 } else { 0.35 },
        timestamp: chrono::Utc::now(),
        user_override: user_id.and_then(|id| override_store.resolve(id)),
        telemetry: None,
        health: None,
    };

    // Apply user override if present
    if let Some(backend) = &ctx.user_override {
        return BackendSelection::new(backend.clone(), "user override");
    }

    // Run full routing + fallback logic
    resolve_with_fallback(&ctx, &config, health_store)
}

/// Integration test helper
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_routing() {
        let config = RouterConfig::default();
        let ctx = RoutingContext {
            prompt: "Write a complex Rust program".into(),
            token_estimate: 1200,
            has_code: true,
            complexity_score: 0.8,
            timestamp: chrono::Utc::now(),
            user_override: None,
            telemetry: None,
            health: None,
        };

        let backend = route(&ctx, &config);
        assert!(matches!(backend, LLMBackend::LocalOllama | LLMBackend::Grok));
    }

    #[test]
    fn test_user_override_takes_precedence() {
        let mut overrides = OverrideStore::new();
        overrides.apply(OverrideCommand {
            user_id: "alice".into(),
            backend: Some(LLMBackend::Grok),
            tier: OverrideTier::OneShot,
            ttl_seconds: None,
        }).unwrap();

        let selection = decide_backend_with_full_context(
            "simple question",
            50,
            false,
            Some("alice"),
            &ConfigManager::new("config/router.toml"),
            &overrides,
            None,
        );

        assert_eq!(selection.chosen, LLMBackend::Grok);
        assert!(selection.reason.contains("override"));
    }
}
