//! SSH server into Helix (russh 0.63, key-only, loopback by default).
//!
//! Lets John (or his tooling) open an SSH session into the Helix service
//! from the box itself (or the Tailscale interface) and run commands.
//! Sessions execute through the [`run_shell`](crate::tools::shell_tool)
//! policy — one shell path, not two.
//!
//! The full threat model lives in `src/config/ssh.rs` and `docs/ssh.md`.

pub mod handler;
pub mod server;

pub use handler::{HelixSshHandler, SharedState};
pub use server::start_ssh_server;
