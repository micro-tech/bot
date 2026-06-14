// router/fallback.rs - Fallback Engine & Degradation Logic
use crate::router::{LLMBackend, RoutingContext, RouterConfig};

#[derive(Debug, Clone)]
pub struct BackendSelection {
    pub chosen: LLMBackend,
    pub fallbacks_attempted: Vec<(LLMBackend, String)>,
    pub reason: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

pub fn resolve_with_fallback(
    ctx: &RoutingContext,
    config: &RouterConfig,
    health: Option<&crate::router::health::HealthStore>,
) -> BackendSelection {
    let mut attempted = Vec::new();
    let primary = crate::router::route(ctx, config);

    // Check health if available
    if let Some(h) = health {
        if let Some(status) = h.get(&format!("{:?}", primary)) {
            if status.degraded || !status.reachable {
                attempted.push((primary.clone(), "degraded".into()));
                // try first healthy fallback
                for fb in &config.fallback_chain {
                    if let Some(s) = h.get(&format!("{:?}", fb)) {
                        if s.reachable && !s.degraded {
                            return BackendSelection {
                                chosen: fb.clone(),
                                fallbacks_attempted: attempted,
                                reason: "primary unhealthy".into(),
                                timestamp: chrono::Utc::now(),
                            };
                        }
                    }
                }
            }
        }
    }

    BackendSelection {
        chosen: primary,
        fallbacks_attempted: attempted,
        reason: "primary selected".into(),
        timestamp: chrono::Utc::now(),
    }
}
