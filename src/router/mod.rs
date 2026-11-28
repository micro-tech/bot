// router/mod.rs - LLM Router Core (updated with all modules)
pub mod complexity;
pub mod config;
pub mod context;
pub mod fallback;
pub mod health;
pub mod integration;
pub mod route;
pub mod schedule;
pub mod spend;
pub mod strategy;
pub mod telemetry;
pub mod user_override;

pub use config::{ConfigManager, RouterConfig};
pub use context::{LLMBackend, RoutingContext};

pub use fallback::{BackendSelection, resolve_with_fallback};
pub use health::HealthStore;
pub use route::route;
pub use user_override::OverrideStore;

pub use spend::{check_api_allowed, note_api_call, reserve_api_spend};
