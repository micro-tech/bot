// router/debug_tests.rs - Tests for Task 147

use crate::router::debug::{generate_explain, redact_sensitive};
use crate::router::context::{RoutingContext, LLMBackend};
use chrono::Utc;

#[test]
fn test_generate_explain_basic() {
    let ctx = RoutingContext {
        prompt: "Test prompt".to_string(),
        token_estimate: 100,
        has_code: false,
        complexity_score: 0.5,
        timestamp: Utc::now(),
        user_override: None,
        telemetry: None,
        health: None,
    };

    let tree = generate_explain(&ctx, LLMBackend::Ollama, 12);
    assert_eq!(tree.final_backend, LLMBackend::Ollama);
    assert!(!tree.steps.is_empty());
}

#[test]
fn test_redact_sensitive() {
    let ctx = RoutingContext {
        prompt: "Secret prompt with user data".to_string(),
        token_estimate: 50,
        has_code: false,
        complexity_score: 0.3,
        timestamp: Utc::now(),
        user_override: None,
        telemetry: None,
        health: None,
    };

    let tree = generate_explain(&ctx, LLMBackend::Gemini, 8);
    let redacted = redact_sensitive(tree);

    assert_eq!(redacted.prompt, "[REDACTED]");
    assert_ne!(redacted.prompt, "Secret prompt with user data");
}