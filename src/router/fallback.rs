// router/fallback.rs - Full Fallback Engine & Degradation Logic (Task 134)
use crate::router::{LLMBackend, RoutingContext, RouterConfig};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct BackendSelection {
    pub chosen: LLMBackend,
    pub fallbacks_attempted: Vec<(LLMBackend, String)>,
    pub reason: String,
    pub timestamp: DateTime<Utc>,
    pub health_snapshot: Option<String>,
}

impl BackendSelection {
    pub fn new(chosen: LLMBackend, reason: impl Into<String>) -> Self {
        Self {
            chosen,
            fallbacks_attempted: Vec::new(),
            reason: reason.into(),
            timestamp: Utc::now(),
            health_snapshot: None,
        }
    }
}

/// Main fallback resolver — respects override precedence and health
pub fn resolve_with_fallback(
    ctx: &RoutingContext,
    config: &RouterConfig,
    health: Option<&crate::router::health::HealthStore>,
) -> BackendSelection {
    let mut attempted = Vec::new();

    // 1. User override takes absolute precedence (already handled in route())
    let primary = crate::router::route(ctx, config);

    // 2. Check health of primary choice
    if let Some(h) = health {
        if let Some(status) = h.get(&format!("{:?}", primary)) {
            if !status.reachable || status.degraded {
                attempted.push((primary.clone(), "primary degraded/unreachable".into()));

                // Try fallback chain from config
                for fb_name in &config.fallback_chain {
                    if let Some(fb_status) = h.get(fb_name) {
                        if fb_status.reachable && !fb_status.degraded {
                            return BackendSelection {
                                chosen: parse_backend(fb_name),
                                fallbacks_attempted: attempted,
                                reason: "primary unhealthy, used fallback".into(),
                                timestamp: Utc::now(),
                                health_snapshot: Some(format!("{:?}", fb_status)),
                            };
                        }
                    }
                }

                // Final fallback
                return BackendSelection {
                    chosen: LLMBackend::Fallback,
                    fallbacks_attempted: attempted,
                    reason: "all backends unhealthy".into(),
                    timestamp: Utc::now(),
                    health_snapshot: None,
                };
            }
        }
    }

    // Healthy primary selection
    BackendSelection {
        chosen: primary,
        fallbacks_attempted: attempted,
        reason: "primary selected".into(),
        timestamp: Utc::now(),
        health_snapshot: None,
    }
}

fn parse_backend(name: &str) -> LLMBackend {
    match name.to_lowercase().as_str() {
        "localollama" | "ollama" => LLMBackend::LocalOllama,
        "lanollama" => LLMBackend::LanOllama,
        "gemini" => LLMBackend::Gemini,
        "grok" => LLMBackend::Grok,
        _ => LLMBackend::Fallback,
    }
}
