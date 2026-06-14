// router/mod.rs - LLM Router core module
pub mod context;
pub use context::{LLMBackend, RoutingContext};

use std::collections::HashMap;

/// Pure routing function - deterministic selection logic
pub fn route(ctx: &RoutingContext, config: &RouterConfig) -> LLMBackend {
    // 1. User override takes absolute precedence
    if let Some(backend) = &ctx.user_override {
        return backend.clone();
    }

    // 2. Check health / telemetry thresholds (stub for now)
    if let Some(health) = &ctx.health {
        if health.get("degraded").and_then(|v| v.as_bool()).unwrap_or(false) {
            return LLMBackend::Fallback;
        }
    }

    // 3. Complexity + token heuristics
    if ctx.complexity_score > config.complexity_threshold || ctx.token_estimate > config.token_threshold {
        return LLMBackend::Grok; // heavy reasoning
    }

    if ctx.has_code {
        return LLMBackend::LocalOllama;
    }

    // 4. Default / time-of-day stub
    LLMBackend::Gemini
}

#[derive(Debug, Clone)]
pub struct RouterConfig {
    pub complexity_threshold: f32,
    pub token_threshold: usize,
    pub fallback_chain: Vec<LLMBackend>,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            complexity_threshold: 0.7,
            token_threshold: 2048,
            fallback_chain: vec![LLMBackend::LocalOllama, LLMBackend::Gemini, LLMBackend::Fallback],
        }
    }
}
