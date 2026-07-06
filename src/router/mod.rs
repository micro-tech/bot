// router/mod.rs - LLM Router Core (updated with all modules)
pub mod complexity;
pub mod config;
pub mod context;
pub mod fallback;
pub mod health;
pub mod integration;
pub mod user_override;
pub mod route;
pub mod schedule;
pub mod strategy;
pub mod telemetry;

pub use context::{LLMBackend, RoutingContext};
pub use config::{RouterConfig, ConfigManager, ComplexityConfig, ScheduleConfig, LoadThresholds, HealthThresholds};
pub use strategy::{RoutingStrategy, DefaultStrategy};
pub use fallback::{BackendSelection, resolve_with_fallback};
pub use route::route;
pub use user_override::{OverrideStore, OverrideCommand, OverrideTier};
pub use health::HealthStore;
pub use telemetry::TelemetryCollector;
