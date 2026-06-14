// router/integration.rs - Wire router into CPU pipeline
use crate::router::{route, LLMBackend, RoutingContext, RouterConfig};

/// Called from CPU before any LLM call.
/// Returns the selected backend and a possibly mutated RoutingContext.
pub fn decide_backend(prompt: &str, token_estimate: usize, has_code: bool) -> (LLMBackend, RoutingContext) {
    let ctx = RoutingContext {
        prompt: prompt.to_string(),
        token_estimate,
        has_code,
        complexity_score: if has_code { 0.6 } else { 0.3 },
        timestamp: chrono::Utc::now(),
        user_override: None,
        telemetry: None,
        health: None,
    };

    let config = RouterConfig::default();
    let backend = route(&ctx, &config);
    (backend, ctx)
}
