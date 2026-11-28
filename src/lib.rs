//! Helix crate library root (for integration tests & reuse)
//! Re-exports the main public modules.

pub mod a2a;
pub mod acp;
pub mod agents;
pub mod bayesian;
pub mod browser_install;
pub mod bus;
pub mod chat_command; // task 295: chat-hub command loop (read-only diagnostics)
pub mod chat_history;
pub mod chat_mcp_install; // task 295: chat-mcp installer hook (binary + config + [[mcp.servers]] block)
pub mod config;
pub mod cpu;
pub mod cron;
pub mod hooks;
pub mod hy_evo;
pub mod io;
pub mod mcp_client;
pub mod memory;
pub mod network_capture; // task 376: hourly network capture log (24h archive)
pub mod okf;
pub mod persona;
pub mod planning;
pub mod reasoning;
pub mod router;
pub mod security;
pub mod self_improve; // tasks 277+: self-improving loop (Phase 0 config first)
pub mod skills;
pub mod ssh;
pub mod support_watch; // task 256: SYSADMIN-01 adaptive support-mail poll loop
pub mod tools;
pub mod utils;
pub mod voice; // task 267/268: voice-box TTS/STT integration // tasks 350/360: Firefox + mini-browser installer hook (APT repo, config section, dirs)

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
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
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

    /// M26 (task 414): shell scripts must have LF endings, enforced by
    /// `.gitattributes`. A CRLF script copied (not cloned) to the Linux
    /// host fails on `set -euo pipefail\r`, killing the nightly backup.
    /// This test scans every `*.sh` in the repo for CR bytes and asserts
    /// the `.gitattributes` rule is present, so a CRLF script can never
    /// land silently.
    #[test]
    fn shell_scripts_have_lf_endings() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let attrs = std::fs::read_to_string(root.join(".gitattributes"))
            .expect(".gitattributes must exist (M26)");
        assert!(
            attrs.lines().any(|l| {
                let l = l.trim();
                l.starts_with("*.sh") && l.contains("eol=lf")
            }),
            ".gitattributes must contain a `*.sh ... eol=lf` rule, got:\n{}",
            attrs
        );
        let mut files = Vec::new();
        collect_sh_files(root, &mut files);
        assert!(
            !files.is_empty(),
            "no *.sh files found — the test is vacuous"
        );
        let mut crlf: Vec<String> = Vec::new();
        for f in &files {
            let bytes = std::fs::read(f).unwrap_or_else(|e| panic!("cannot read {:?}: {}", f, e));
            if bytes.contains(&b'\r') {
                crlf.push(f.display().to_string());
            }
        }
        assert!(
            crlf.is_empty(),
            "shell scripts with CRLF endings (must be LF — see .gitattributes):\n{}",
            crlf.join("\n")
        );
    }

    fn collect_sh_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if !name.starts_with('.') && name != "target" {
                    collect_sh_files(&path, out);
                }
            } else if name.ends_with(".sh") {
                out.push(path);
            }
        }
    }
}
