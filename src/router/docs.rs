// router/docs.rs - Documentation & Examples (Task 136)
// This module contains inline documentation and usage examples for the router.

/// # LLM Router Usage Guide
///
/// ## Basic Routing
/// ```rust
/// let ctx = RoutingContext {
///     prompt: "Explain quantum computing".into(),
///     token_estimate: 800,
///     has_code: false,
///     complexity_score: 0.6,
///     timestamp: Utc::now(),
///     user_override: None,
///     telemetry: None,
///     health: None,
/// };
/// let backend = route(&ctx, &RouterConfig::default());
/// ```
///
/// ## Configuration (TOML)
/// ```toml
/// [complexity]
/// global_threshold = 0.65
/// token_weight = 0.2
///
/// [load_thresholds]
/// gpu_max = 85.0
/// vram_max = 90.0
/// ```
///
/// ## User Overrides
/// ```rust
/// overrides.apply(OverrideCommand {
///     user_id: "user123".into(),
///     backend: Some(LLMBackend::Grok),
///     tier: OverrideTier::Session,
///     ttl_seconds: Some(3600),
/// });
/// ```
pub mod docs {}
