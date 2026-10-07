//! ACP (Agent Client Protocol) server for Helix.
//!
//! Lets Zed and other ACP clients drive Helix natively over stdio
//! JSON-RPC. The wire format comes from the official
//! `agent-client-protocol` crate; [`agent::HelixAcpAgent`] adapts it to
//! Helix's [`RuntimeLoop`](crate::agents::runtime_loop::RuntimeLoop).
//!
//! Entry point: `helix acp` (see `src/main.rs`). The server is opt-in via
//! the `[acp]` config section and does nothing unless enabled.

pub mod agent;
pub mod elicitation;
pub mod protocol;
pub mod server;
pub mod status_bar;
pub mod tools;

// Re-exported for the `helix acp` entry point in src/main.rs.
pub use agent::HelixAcpAgent;
