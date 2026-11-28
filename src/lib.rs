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

    /// M18: production output goes through `log::` — no `println!` /
    /// `eprintln!` in daemon code, so the journal stays filterable via
    /// `RUST_LOG`.
    ///
    /// `println!` / `eprintln!` are allowed ONLY for direct user-facing
    /// CLI output (task 265 policy): the startup banner / "Using config
    /// file" / ACP-mode messages in `src/main.rs`, and the standalone
    /// binaries and interactive tools (`src/bin/installer.rs`,
    /// `src/bin/test_ollama.rs`, `src/router/tui.rs`,
    /// `src/io/unix_cli/documentation_updates.rs`,
    /// `src/browser_install.rs`, `src/chat_mcp_install.rs` — the last
    /// two are shared with the installer binary via `#[path]`, so their
    /// warnings are installer output too).
    ///
    /// The test scans the crate source (via `CARGO_MANIFEST_DIR`, so it
    /// works regardless of CWD) and asserts:
    ///   1. no `println!`/`eprintln!` in any file outside the allowlist
    ///      (ignoring `#[cfg(test)]` modules, comments, and string
    ///      literals);
    ///   2. the total count inside the allowlisted files equals the
    ///      known baseline (112) — a new print anywhere, daemon or
    ///      CLI, fails the test until the baseline is deliberately
    ///      updated.
    ///
    /// A source scan is the only way to enforce this: the compiler
    /// cannot lint for `println!`. If this test fails on a file you
    /// just edited, convert the print to
    /// `log::{debug,info,warn,error}!` (daemon code) or add the file
    /// to the allowlist with a comment (CLI output only).
    #[test]
    fn no_println_in_daemon_code() {
        use std::path::{Path, PathBuf};
        // Files where println!/eprintln! is intentional CLI output.
        const ALLOWLIST: &[&str] = &[
            "bin/installer.rs",
            "bin/test_ollama.rs",
            "router/tui.rs",
            "io/unix_cli/documentation_updates.rs",
            "browser_install.rs",
            "chat_mcp_install.rs",
            "main.rs",
        ];
        // Count of real println!/eprintln! calls in the allowlisted
        // files (outside test mods, comments, and strings). Update
        // deliberately when CLI output is added or removed.
        const ALLOWLIST_BASELINE: usize = 112;

        fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
            let Ok(rd) = std::fs::read_dir(dir) else {
                return;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    collect_rs(&p, out);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    out.push(p);
                }
            }
        }

        // Byte spans of `#[cfg(test)] mod ... { ... }` blocks, so
        // prints inside tests are not counted as production prints.
        fn test_mod_spans(text: &str) -> Vec<(usize, usize)> {
            let bytes = text.as_bytes();
            let mut spans = Vec::new();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i..].starts_with(b"#[cfg(test)]") {
                    // Find the `mod name {` that follows.
                    let rest = &text[i..];
                    if let Some(mod_at) = rest.find("mod ") {
                        let after_mod = &rest[mod_at..];
                        if let Some(brace_at) = after_mod.find('{') {
                            let brace_pos = i + mod_at + brace_at;
                            let mut depth = 0usize;
                            let mut j = brace_pos;
                            let mut in_str = false;
                            let mut in_char = false;
                            let mut in_line_comment = false;
                            let mut in_block_comment = false;
                            while j < bytes.len() {
                                let c = bytes[j] as char;
                                if in_line_comment {
                                    if c == '\n' {
                                        in_line_comment = false;
                                    }
                                } else if in_block_comment {
                                    if c == '*' && j + 1 < bytes.len() && bytes[j + 1] == b'/' {
                                        in_block_comment = false;
                                        j += 1;
                                    }
                                } else if in_str {
                                    if c == '\\' {
                                        j += 1;
                                    } else if c == '"' {
                                        in_str = false;
                                    }
                                } else if in_char {
                                    if c == '\\' {
                                        j += 1;
                                    } else if c == '\'' {
                                        in_char = false;
                                    }
                                } else if c == '"' && (j == 0 || bytes[j - 1] != b'\\') {
                                    in_str = true;
                                } else if c == '\'' && j + 1 < bytes.len() && bytes[j + 1] != b'\\'
                                {
                                    in_char = true;
                                } else if c == '/' && j + 1 < bytes.len() && bytes[j + 1] == b'/' {
                                    in_line_comment = true;
                                } else if c == '/' && j + 1 < bytes.len() && bytes[j + 1] == b'*' {
                                    in_block_comment = true;
                                } else if c == '{' {
                                    depth += 1;
                                } else if c == '}' {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                j += 1;
                            }
                            spans.push((i, j));
                            i = j;
                            continue;
                        }
                    }
                }
                i += 1;
            }
            spans
        }

        // `println!` / `eprintln!` tokens in code (not in strings or
        // comments), as (byte_offset, line_number, is_eprintln).
        fn print_tokens(text: &str) -> Vec<(usize, usize, bool)> {
            let bytes = text.as_bytes();
            let mut out = Vec::new();
            let mut i = 0;
            let mut line = 1usize;
            while i < bytes.len() {
                let c = bytes[i] as char;
                if c == '\n' {
                    line += 1;
                    i += 1;
                    continue;
                }
                if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                    continue;
                }
                if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                    i += 2;
                    while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                        if bytes[i] == b'\n' {
                            line += 1;
                        }
                        i += 1;
                    }
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                if c == '"' {
                    i += 1;
                    while i < bytes.len() {
                        if bytes[i] == b'\\' {
                            i += 2;
                            continue;
                        }
                        if bytes[i] == b'"' {
                            i += 1;
                            break;
                        }
                        if bytes[i] == b'\n' {
                            line += 1;
                        }
                        i += 1;
                    }
                    continue;
                }
                if c == '\'' {
                    // Char literal (best effort; lifetimes like `'a` are
                    // also skipped harmlessly — they contain no println!).
                    i += 1;
                    if i < bytes.len() && bytes[i] == b'\\' {
                        i += 2;
                    } else if i < bytes.len() {
                        i += 1;
                    }
                    if i < bytes.len() && bytes[i] == b'\'' {
                        i += 1;
                    }
                    continue;
                }
                let mut matched = false;
                for (name, is_e) in [("eprintln!", true), ("println!", false)] {
                    if bytes[i..].starts_with(name.as_bytes())
                        && (i == 0
                            || !(bytes[i - 1].is_ascii_alphanumeric()
                                || bytes[i - 1] == b'_'
                                || bytes[i - 1] == b':'))
                    {
                        out.push((i, line, is_e));
                        i += name.len();
                        matched = true;
                        break;
                    }
                }
                if !matched {
                    i += 1;
                }
            }
            out
        }

        let src_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        collect_rs(&src_root, &mut files);
        let mut violations: Vec<String> = Vec::new();
        let mut allowlisted_count = 0usize;
        for path in files {
            let rel = path
                .strip_prefix(&src_root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let spans = test_mod_spans(&text);
            let tokens = print_tokens(&text);
            for (off, line, _is_e) in tokens {
                if spans.iter().any(|(s, e)| *s <= off && off < *e) {
                    continue; // inside #[cfg(test)] — allowed
                }
                if ALLOWLIST.contains(&rel.as_str()) {
                    allowlisted_count += 1;
                } else {
                    violations.push(format!("{}:{}", rel, line));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "M18: println!/eprintln! in daemon code (convert to log:: or add the file to the allowlist with a comment): {}",
            violations.join(", ")
        );
        assert_eq!(
            allowlisted_count, ALLOWLIST_BASELINE,
            "M18: allowlisted println!/eprintln! count changed ({} != {}). A print was added or removed in a CLI-output file — update ALLOWLIST_BASELINE deliberately, or convert a daemon print to log::.",
            allowlisted_count, ALLOWLIST_BASELINE
        );
    }
}
