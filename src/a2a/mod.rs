//! A2A v1.0 (Linux Foundation `lf.a2a.v1`) support for Helix.
//!
//! Served from the axum web server: Agent Card discovery at
//! `/.well-known/agent-card.json` and the JSON-RPC endpoint at `/a2a`.
//! Tasks execute on the same agent loop as ACP
//! ([`HelixAcpAgent`](crate::acp::agent::HelixAcpAgent)) — one agent loop,
//! multiple protocol surfaces.

pub mod a2a_handler;
pub mod tasks;
pub mod types;

pub use a2a_handler::{agent_card, a2a_jsonrpc, A2aState};
