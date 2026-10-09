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
/// ## Machine-Assignment Policy (Task 217)
///
/// The commander (`spawn_subagent`) assigns work to backends on a fixed
/// ladder, fastest/cheapest first:
///
/// 1. **local_ollama** — the fast interactive box (John's desktop, RTX 3060).
///    First choice for quick work; free.
/// 2. **lan_ollama** — the ollama server box (overflow + embeddings).
///    Task 217.1 decision: it is the *overflow* rung, not a general peer of
///    local. The complexity router (`route()`) never selects it directly —
///    it is reached explicitly (commander picks `lan_ollama`, a cron routine
///    sets `backend: "lan_ollama"`) or via health-aware fallback when the
///    local box is down. Keeping it out of `route()` avoids splitting the
///    free tier across two machines for no reason.
/// 3. **gemini** — heavy reasoning; costs API budget. Gated by the daily
///    spend cap (`[router] daily_api_cap_usd`, default $5.00).
/// 4. **grok** — not implemented yet; never selected.
///
/// Assignment rule: an **explicit backend wins**; `backend: "auto"` (or the
/// commander's judgment) defers to the complexity router, which picks between
/// local_ollama and gemini. After assignment, the spawn path probes the
/// chosen rung (`/api/tags`, 60s cache) and **falls back down the ladder**
/// on unreachability — every fallback is logged and noted in the sub-agent
/// report, so the commander always knows what actually ran. If the spend cap
/// is hit, paid rungs refuse with a John-facing message and the refusal goes
/// back to the commander (John's chat), never silently dropped.
///
/// ```toml
/// [router]
/// daily_api_cap_usd = 5.00   # default; raise/lower per John's budget
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
