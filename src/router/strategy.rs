// router/strategy.rs - Strategy trait for pluggable routing logic
use crate::router::{LLMBackend, RouterConfig, RoutingContext};

pub trait RoutingStrategy: Send + Sync {
    fn select(&self, ctx: &RoutingContext, config: &RouterConfig) -> LLMBackend;
}

/// Default deterministic strategy
pub struct DefaultStrategy;

impl RoutingStrategy for DefaultStrategy {
    fn select(&self, ctx: &RoutingContext, config: &RouterConfig) -> LLMBackend {
        crate::router::route(ctx, config)
    }
}
