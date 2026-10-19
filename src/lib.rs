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
pub mod security;
pub mod acp;
pub mod a2a;
pub mod ssh;
pub mod cron;
pub mod chat_history;
pub mod persona;
pub mod support_watch; // task 256: SYSADMIN-01 adaptive support-mail poll loop
pub mod voice; // task 267/268: voice-box TTS/STT integration

// Re-export commonly used types
pub use bus::{Bus, Message};
pub use memory::MemoryManager;
pub use skills::SkillRegistry;

/// Install the `env_logger` backend for the `log` facade (task 265).
///
/// Before this, nothing ever sank the facade: the crate used `log::`
/// macros all over (support watcher, MCP client, ...) but no logger was
/// ever installed, so every `info!`/`debug!`/`warn!` was a silent no-op
/// and `RUST_LOG` had no effect — watcher/MCP failures were invisible.
/// `env_logger` honors `RUST_LOG` (including per-module filters like
/// `RUST_LOG=helix::support_watch=debug`) and defaults to `info` when
/// unset. It writes to stderr, which systemd forwards to the journal.
///
/// Idempotent: uses `try_init`, so a second call (tests, re-entry) is a
/// no-op instead of a panic. Call once, early, from every binary entry
/// point (`main.rs`) before any code that logs.
pub fn init_logging() {
    let _ = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"),
    )
    .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_logging_is_idempotent_and_never_panics() {
        // GIVEN the logger may already be installed (or not):
        // WHEN init_logging() is called twice in a row,
        // THEN neither call panics (try_init swallows the second-init error).
        init_logging();
        init_logging();
        // AND the installed logger actually sinks records: visible with
        // `cargo test --lib init_logging -- --nocapture`.
        log::info!("init_logging test marker — logger is live");
    }
}
