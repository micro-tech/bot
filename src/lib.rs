//! Helix crate library root (for integration tests & reuse)
//! Re-exports the main public modules.

pub mod bus;
pub mod cpu;
pub mod memory;
pub mod skills;
pub mod hy_evo;
pub mod config;
pub mod utils;
pub mod io;
pub mod hooks;
pub mod tools;
pub mod mcp_client;
pub mod planning;
pub mod reasoning;
pub mod bayesian;
pub mod agents;
pub mod okf;
pub mod router;
pub mod acp;
pub mod a2a;
pub mod ssh;
pub mod cron;

// Re-export commonly used types
pub use bus::{Bus, Message};
pub use memory::MemoryManager;
pub use skills::SkillRegistry;
