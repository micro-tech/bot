//! Task-handoff envelope parser (task 309, spec §§3–4).
//!
//! A `!` message is either a plain command (Phase 1 behavior) or a
//! structured JSON envelope that dispatches to the command path or
//! the LLM task handler (Phase 3). The parse-or-fallback rule is
//! load-bearing:
//!
//! - Body is not JSON, or is JSON without a valid `how` field
//!   (`"command"` / `"llm"`) → FALL BACK to the plain command path
//!   (today's behavior: `asus:` split, allowlist, executor). A
//!   hand-typed `!uptime` must keep working.
//! - Body is JSON with a valid `how` → it IS a structured message
//!   and is validated strictly. Any validation failure posts a
//!   rejection reply NAMING the problem and executes NOTHING.
//!
//! The sender gate runs BEFORE this parser (in `process_message`),
//! so a structured message from an unauthorized sender (including
//! `"helix"`) never reaches parsing at all.
//!
//! Envelope schema (spec §3):
//! ```json
//! {"how": "command", "which": "local", "time": 30, "scope": "read", "command": "uptime"}
//! {"how": "llm", "which": "server", "time": 300, "scope": "gmail:read", "task": "Check Gmail for spam..."}
//! ```
//! - `how`: `"command"` or `"llm"` (required to count as structured).
//! - `which`: command target (`"local"` default, `"asus"`); LLM
//!   backend (`"server"`, `"desktop"`, `"gemini"` — REQUIRED, no
//!   default, per spec decision Q1).
//! - `time`: budget in seconds, optional (command defaults to the
//!   configured `command_timeout_secs`; LLM defaults to 300 s).
//!   When present it must be a positive integer ≤ 3600.
//! - `scope`: required for both paths, must be `"read"` or
//!   `"write:<domain>"` with a known domain (spec decision Q3 is
//!   still OPEN — this vocabulary is the proposal, not a decided
//!   rule; enforcement by tool allowlist is the handler's job).
//! - `command` (for `how: command`) / `task` (for `how: llm`):
//!   required, non-empty.
//! - `to`: optional addressee (hub agent name, spec §3). Absent =
//!   broadcast — every watcher acts on it. A message whose `to`
//!   does not match this agent's own name is ignored by the
//!   dispatch layer (spec §4 step 4); the name itself is NOT
//!   validated here, so an unknown agent parses fine and is simply
//!   never acted on by anyone.
//! - Extra fields are ignored (forward compatibility); the field
//!   irrelevant to `how` (`task` on a command, `command` on an LLM
//!   task) is ignored.
//! - Oversize: a structured envelope over 8 KB (the hub's cap) is
//!   rejected. Plain (non-structured) messages are NOT subject to
//!   this cap — the existing command path has its own output cap.

use super::Target;

/// Hub message cap: structured envelopes over this many bytes are
/// rejected (spec §2). Checked only after `how` is confirmed valid,
/// so a long plain command is not rejected by this rule.
pub const MAX_ENVELOPE_BYTES: usize = 8192;

/// Max `time` budget in seconds (1 hour). A larger budget is a
/// rejection, not a clamp — the sender must state a sane budget.
pub const MAX_TIME_SECS: u64 = 3600;

/// Known write domains for the `write:<domain>` scope vocabulary.
/// Shared with the task handler (single source of truth): the
/// handler enforces these same domains by tool allowlist. Gmail
/// write tool names are NOT confirmed against the live google-mcp
/// tool list yet — the handler refuses a `write:gmail` spawn when
/// no Gmail write tool is advertised (fail closed).
pub const KNOWN_WRITE_DOMAINS: &[&str] = &["notes", "gmail"];

/// Scope, parsed from the envelope's `scope` string. This is the
/// ENFORCEMENT class (what the handler allowlists), not the raw
/// spelling: the spec's §3 table says `"read"` / `"write:<domain>"`,
/// but its §3 LLM example says `"gmail:read"` (domain first). Both
/// orders are accepted for known domains, since the enforcement is
/// identical either way — rejecting the spec's own example would be
/// worse than normalizing it. `display()` normalizes to the §3 table
/// form (`read` / `write:<domain>`) for reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// Report only — no write tools of any kind.
    Read,
    /// Writes only in this domain (plus read tools).
    Write(String),
}

impl Scope {
    /// Normalized form for reports (`read` / `write:<domain>`).
    pub fn display(&self) -> String {
        match self {
            Scope::Read => "read".to_string(),
            Scope::Write(domain) => format!("write:{domain}"),
        }
    }
}

/// Parse a `scope` string into its enforcement class. Accepts
/// `"read"`, `"write:<domain>"`, `"<domain>:read"`, and
/// `"<domain>:write"` (domain must be known); anything else is an
/// error naming the expected forms.
pub fn parse_scope(s: &str) -> Result<Scope, String> {
    if s == "read" {
        return Ok(Scope::Read);
    }
    if let Some(domain) = s.strip_prefix("write:") {
        if domain.is_empty() {
            return Err("malformed scope 'write:' (a domain is required)".to_string());
        }
        if KNOWN_WRITE_DOMAINS.contains(&domain) {
            return Ok(Scope::Write(domain.to_string()));
        }
        return Err(format!(
            "unknown scope domain '{domain}' (known: {})",
            KNOWN_WRITE_DOMAINS.join(", ")
        ));
    }
    // Domain-first forms (the spec's §3 example is "gmail:read").
    if let Some((domain, perm)) = s.split_once(':') {
        if !domain.is_empty() && KNOWN_WRITE_DOMAINS.contains(&domain) {
            return match perm {
                "read" => Ok(Scope::Read),
                "write" => Ok(Scope::Write(domain.to_string())),
                _ => Err(format!(
                    "malformed scope '{s}' (expected read, write:<domain>, or <domain>:read)"
                )),
            };
        }
        if !domain.is_empty() && !perm.contains(':') {
            return Err(format!(
                "unknown scope domain '{domain}' (known: {})",
                KNOWN_WRITE_DOMAINS.join(", ")
            ));
        }
    }
    Err(format!(
        "malformed scope '{s}' (expected read, write:<domain>, or <domain>:read)"
    ))
}

/// LLM backend named by `which` on an `how: llm` envelope.
/// `spawn_backend` is the name `dispatch_spawn` understands (the
/// existing task-217.1 mapping: server → the LAN ollama box,
/// desktop → the local ollama box, gemini → Gemini).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LlmBackend {
    Server,
    Desktop,
    Gemini,
}

impl LlmBackend {
    /// The `which` value from the envelope.
    pub fn display(&self) -> &'static str {
        match self {
            LlmBackend::Server => "server",
            LlmBackend::Desktop => "desktop",
            LlmBackend::Gemini => "gemini",
        }
    }

    /// Backend name for `SpawnSpec` (existing spawn mapping).
    pub fn spawn_backend(&self) -> &'static str {
        match self {
            LlmBackend::Server => "lan_ollama",
            LlmBackend::Desktop => "local_ollama",
            LlmBackend::Gemini => "gemini",
        }
    }
}

/// A validated structured envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Structured {
    Command {
        target: Target,
        /// `time` override, if present (else the configured timeout).
        time_secs: Option<u64>,
        /// Scope, validated here; enforcement is the handler's job
        /// (commands stay read-only via the executor allowlist
        /// regardless of scope — scope does not widen commands).
        scope: Scope,
        command: String,
        /// Addressee (`to`), if present. Absent = broadcast. The
        /// dispatch layer ignores a message whose `to` does not
        /// match this agent's own name (spec §4 step 4).
        to: Option<String>,
    },
    Llm {
        backend: LlmBackend,
        /// `time` override, if present (else the handler default).
        time_secs: Option<u64>,
        scope: Scope,
        task: String,
        /// Addressee (`to`), if present. Absent = broadcast.
        to: Option<String>,
    },
}

impl Structured {
    /// The addressee (`to`), if the envelope named one. `None`
    /// means broadcast — every watcher acts on the message.
    pub fn addressee(&self) -> Option<&str> {
        match self {
            Structured::Command { to, .. } => to.as_deref(),
            Structured::Llm { to, .. } => to.as_deref(),
        }
    }
}

/// Parse result for one `!` rest (text after `!`, trimmed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    /// Not a structured message — run the plain command path.
    Plain(String),
    /// Validated structured envelope — dispatch it.
    Structured(Structured),
    /// Structured message that failed validation — post a rejection
    /// naming this problem and execute nothing.
    Reject(String),
}

fn reject(msg: impl Into<String>) -> Parsed {
    Parsed::Reject(msg.into())
}

/// Parse one `!` rest into plain / structured / rejection.
/// Pure function (no I/O) — the unit tests cover every branch.
pub fn parse(rest: &str) -> Parsed {
    let value: serde_json::Value = match serde_json::from_str(rest) {
        Ok(v) => v,
        Err(_) => return Parsed::Plain(rest.to_string()),
    };
    let obj = match value.as_object() {
        Some(o) => o,
        None => return Parsed::Plain(rest.to_string()),
    };
    // `how` missing, non-string, or unknown → FALL BACK to plain.
    // (A valid `how` is what makes a message "structured".)
    let how = match obj.get("how").and_then(|h| h.as_str()) {
        Some("command") => "command",
        Some("llm") => "llm",
        _ => return Parsed::Plain(rest.to_string()),
    };
    // Structured from here on: every failure is a rejection.
    if rest.len() > MAX_ENVELOPE_BYTES {
        return reject(format!(
            "message too large: envelope is {} bytes (cap 8 KB), nothing executed",
            rest.len()
        ));
    }
    let time_secs = match parse_time(obj) {
        Ok(t) => t,
        Err(reason) => return reject(reason),
    };
    let to = match parse_to(obj) {
        Ok(t) => t,
        Err(reason) => return reject(reason),
    };
    let scope = match parse_scope_field(obj) {
        Ok(s) => s,
        Err(reason) => return reject(reason),
    };
    if how == "command" {
        let target = match parse_command_target(obj) {
            Ok(t) => t,
            Err(reason) => return reject(reason),
        };
        let command = match obj
            .get("command")
            .and_then(|c| c.as_str())
            .map(str::trim)
            .filter(|c| !c.is_empty())
        {
            Some(c) => c.to_string(),
            None => return reject("command task needs a command string"),
        };
        Parsed::Structured(Structured::Command {
            target,
            time_secs,
            scope,
            command,
            to,
        })
    } else {
        let backend = match parse_llm_backend(obj) {
            Ok(b) => b,
            Err(reason) => return reject(reason),
        };
        let task = match obj
            .get("task")
            .and_then(|t| t.as_str())
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            Some(t) => t.to_string(),
            None => return reject("llm task needs a task string"),
        };
        Parsed::Structured(Structured::Llm {
            backend,
            time_secs,
            scope,
            task,
            to,
        })
    }
}

/// `time`, when present, must be a positive integer ≤ MAX_TIME_SECS.
/// Absent → None (caller applies its default).
fn parse_time(obj: &serde_json::Map<String, serde_json::Value>) -> Result<Option<u64>, String> {
    let Some(v) = obj.get("time") else {
        return Ok(None);
    };
    match v.as_u64() {
        Some(0) | None => Err(format!(
            "time must be a positive number of seconds (1-{MAX_TIME_SECS})"
        )),
        Some(n) if n > MAX_TIME_SECS => Err(format!(
            "time budget too large (max {MAX_TIME_SECS}s)"
        )),
        Some(n) => Ok(Some(n)),
    }
}

/// `to`, when present, must be a string (the addressee's hub agent
/// name, spec §3). Absent → broadcast (`None`). A non-string `to`
/// is a rejection — structured messages are validated strictly.
/// The name itself is NOT validated here (an unknown agent is
/// simply never acted on by anyone); the dispatch layer compares
/// it to this agent's own name (spec §4 step 4).
fn parse_to(obj: &serde_json::Map<String, serde_json::Value>) -> Result<Option<String>, String> {
    let Some(v) = obj.get("to") else {
        return Ok(None);
    };
    match v.as_str() {
        Some(s) => Ok(Some(s.to_string())),
        None => Err("to must be a string (a hub agent name)".to_string()),
    }
}

/// `scope`: required; parsed by `parse_scope` (accepts the §3 table
/// form and the §3 example's domain-first form for known domains).
fn parse_scope_field(obj: &serde_json::Map<String, serde_json::Value>) -> Result<Scope, String> {
    let Some(v) = obj.get("scope") else {
        return Err("missing scope (expected read or write:<domain>)".to_string());
    };
    let Some(s) = v.as_str() else {
        return Err("scope must be a string (read or write:<domain>)".to_string());
    };
    parse_scope(s)
}

/// `which` for a command: absent → local; `local`/`asus`; else reject.
fn parse_command_target(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<Target, String> {
    let Some(v) = obj.get("which") else {
        return Ok(Target::Local);
    };
    match v.as_str() {
        Some("local") => Ok(Target::Local),
        Some("asus") => Ok(Target::Asus),
        Some(other) => Err(format!(
            "unknown command target '{other}' (expected local or asus)"
        )),
        None => Err("which must be a string (local or asus)".to_string()),
    }
}

/// `which` for an LLM task: REQUIRED, must be server/desktop/gemini.
fn parse_llm_backend(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<LlmBackend, String> {
    let Some(v) = obj.get("which") else {
        return Err(
            "llm task needs a backend: which must be server, desktop, or gemini".to_string(),
        );
    };
    match v.as_str() {
        Some("server") => Ok(LlmBackend::Server),
        Some("desktop") => Ok(LlmBackend::Desktop),
        Some("gemini") => Ok(LlmBackend::Gemini),
        Some(other) => Err(format!(
            "unknown backend '{other}' (expected server, desktop, or gemini)"
        )),
        None => Err("which must be a string (server, desktop, or gemini)".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(json: &str) -> Parsed {
        parse(json)
    }

    #[test]
    fn valid_command_json_routes_local_by_default() {
        assert_eq!(
            cmd(r#"{"how": "command", "scope": "read", "command": "uptime"}"#),
            Parsed::Structured(Structured::Command {
                target: Target::Local,
                time_secs: None,
                scope: Scope::Read,
                command: "uptime".to_string(),
                to: None,
            })
        );
    }

    #[test]
    fn valid_command_json_honors_asus_and_time() {
        assert_eq!(
            cmd(r#"{"how": "command", "which": "asus", "time": 45, "scope": "read", "command": "hostname"}"#),
            Parsed::Structured(Structured::Command {
                target: Target::Asus,
                time_secs: Some(45),
                scope: Scope::Read,
                command: "hostname".to_string(),
                to: None,
            })
        );
    }

    #[test]
    fn valid_llm_json_all_backends() {
        for (which, backend) in [
            ("server", LlmBackend::Server),
            ("desktop", LlmBackend::Desktop),
            ("gemini", LlmBackend::Gemini),
        ] {
            assert_eq!(
                cmd(&format!(
                    r#"{{"how": "llm", "which": "{which}", "time": 300, "scope": "read", "task": "Check disk usage"}}"#
                )),
                Parsed::Structured(Structured::Llm {
                    backend,
                    time_secs: Some(300),
                    scope: Scope::Read,
                    task: "Check disk usage".to_string(),
                    to: None,
                }),
                "backend {which}"
            );
        }
    }

    #[test]
    fn spec_example_llm_json_parses_verbatim() {
        // The spec §3 spam example, verbatim — including its
        // domain-first "gmail:read" scope. It must NOT reject.
        assert_eq!(
            cmd(r#"{"how": "llm", "which": "server", "time": 300, "scope": "gmail:read", "task": "Check Gmail for spam. Report back the senders and subjects of anything that looks like spam; do not label or archive anything."}"#),
            Parsed::Structured(Structured::Llm {
                backend: LlmBackend::Server,
                time_secs: Some(300),
                scope: Scope::Read,
                task: "Check Gmail for spam. Report back the senders and subjects of anything that looks like spam; do not label or archive anything.".to_string(),
                to: None,
            })
        );
    }

    #[test]
    fn scope_forms_both_orders_accepted_for_known_domains() {
        // Table form and domain-first form normalize to the same
        // enforcement class; extra fields and the irrelevant
        // `command` are ignored.
        assert!(matches!(
            cmd(r#"{"how": "llm", "which": "server", "scope": "write:gmail", "task": "Label spam", "command": "uptime", "future": 1}"#),
            Parsed::Structured(Structured::Llm { scope: Scope::Write(d), .. }) if d == "gmail"
        ));
        assert!(matches!(
            cmd(r#"{"how": "llm", "which": "server", "scope": "gmail:write", "task": "Label spam"}"#),
            Parsed::Structured(Structured::Llm { scope: Scope::Write(d), .. }) if d == "gmail"
        ));
        assert!(matches!(
            cmd(r#"{"how": "llm", "which": "server", "scope": "notes:read", "task": "Summarize notes"}"#),
            Parsed::Structured(Structured::Llm { scope: Scope::Read, .. })
        ));
        assert_eq!(Scope::Write("gmail".to_string()).display(), "write:gmail");
        assert_eq!(Scope::Read.display(), "read");
    }

    #[test]
    fn non_json_and_bad_how_fall_back_to_plain() {
        for rest in [
            "uptime",
            "asus: hostname",
            "not json at all",
            r#"{"how": "bogus", "task": "x"}"#,
            r#"{"task": "no how field"}"#,
            r#"{"how": 42, "task": "x"}"#,
            "[1, 2, 3]",
            "42",
        ] {
            assert_eq!(cmd(rest), Parsed::Plain(rest.to_string()), "must fall back: {rest}");
        }
    }

    #[test]
    fn structured_rejections_name_the_problem() {
        let cases: &[(&str, &str)] = &[
            // Missing payload.
            (
                r#"{"how": "command", "scope": "read"}"#,
                "command task needs a command string",
            ),
            (
                r#"{"how": "command", "scope": "read", "command": "   "}"#,
                "command task needs a command string",
            ),
            (
                r#"{"how": "llm", "which": "server", "scope": "read"}"#,
                "llm task needs a task string",
            ),
            // Missing / unknown backend.
            (
                r#"{"how": "llm", "scope": "read", "task": "x"}"#,
                "llm task needs a backend",
            ),
            (
                r#"{"how": "llm", "which": "mars", "scope": "read", "task": "x"}"#,
                "unknown backend 'mars'",
            ),
            (
                r#"{"how": "command", "which": "mars", "scope": "read", "command": "uptime"}"#,
                "unknown command target 'mars'",
            ),
            // Missing / malformed / unknown scope.
            (
                r#"{"how": "command", "command": "uptime"}"#,
                "missing scope",
            ),
            (
                r#"{"how": "llm", "which": "server", "scope": "everything", "task": "x"}"#,
                "malformed scope 'everything'",
            ),
            (
                r#"{"how": "llm", "which": "server", "scope": "write:", "task": "x"}"#,
                "malformed scope 'write:'",
            ),
            (
                r#"{"how": "llm", "which": "server", "scope": "write:calendar", "task": "x"}"#,
                "unknown scope domain 'calendar'",
            ),
            (
                r#"{"how": "llm", "which": "server", "scope": "calendar:read", "task": "x"}"#,
                "unknown scope domain 'calendar'",
            ),
            (
                r#"{"how": "llm", "which": "server", "scope": "gmail:delete", "task": "x"}"#,
                "malformed scope 'gmail:delete'",
            ),
            // Non-string `to`.
            (
                r#"{"how": "command", "scope": "read", "command": "uptime", "to": 42}"#,
                "to must be a string",
            ),
            // Bad time.
            (
                r#"{"how": "command", "scope": "read", "command": "uptime", "time": 0}"#,
                "time must be a positive number",
            ),
            (
                r#"{"how": "command", "scope": "read", "command": "uptime", "time": -5}"#,
                "time must be a positive number",
            ),
            (
                r#"{"how": "command", "scope": "read", "command": "uptime", "time": "soon"}"#,
                "time must be a positive number",
            ),
            (
                r#"{"how": "command", "scope": "read", "command": "uptime", "time": 99999}"#,
                "time budget too large",
            ),
        ];
        for (json, needle) in cases {
            match cmd(json) {
                Parsed::Reject(reason) => {
                    assert!(reason.contains(needle), "{json} → {reason} (want {needle})");
                }
                other => panic!("{json} must reject, got {other:?}"),
            }
        }
    }

    #[test]
    fn to_field_parsed_for_both_paths() {
        // Addressed to this agent and to another — both parse; the
        // dispatch layer decides by comparing to its own name.
        assert!(matches!(
            cmd(r#"{"how": "command", "to": "helix", "scope": "read", "command": "uptime"}"#),
            Parsed::Structured(Structured::Command { to: Some(t), .. }) if t == "helix"
        ));
        assert!(matches!(
            cmd(r#"{"how": "command", "to": "john", "scope": "read", "command": "uptime"}"#),
            Parsed::Structured(Structured::Command { to: Some(t), .. }) if t == "john"
        ));
        assert!(matches!(
            cmd(r#"{"how": "llm", "which": "server", "to": "john", "scope": "read", "task": "x"}"#),
            Parsed::Structured(Structured::Llm { to: Some(t), .. }) if t == "john"
        ));
        // An unknown agent name parses fine — it is simply never
        // acted on by anyone (spec §4 step 4).
        assert!(matches!(
            cmd(r#"{"how": "command", "to": "grok-cli", "scope": "read", "command": "uptime"}"#),
            Parsed::Structured(Structured::Command { to: Some(t), .. }) if t == "grok-cli"
        ));
    }

    #[test]
    fn addressee_accessor_broadcast_is_none() {
        let addressed = match cmd(
            r#"{"how": "command", "to": "helix", "scope": "read", "command": "uptime"}"#,
        ) {
            Parsed::Structured(s) => s,
            other => panic!("expected structured, got {other:?}"),
        };
        assert_eq!(addressed.addressee(), Some("helix"));
        let broadcast = match cmd(r#"{"how": "command", "scope": "read", "command": "uptime"}"#)
        {
            Parsed::Structured(s) => s,
            other => panic!("expected structured, got {other:?}"),
        };
        assert_eq!(broadcast.addressee(), None);
    }

    #[test]
    fn oversize_structured_envelope_rejects() {
        let big_task = "x".repeat(MAX_ENVELOPE_BYTES);
        let json = format!(
            r#"{{"how": "llm", "which": "server", "scope": "read", "task": "{big_task}"}}"#
        );
        match cmd(&json) {
            Parsed::Reject(reason) => assert!(reason.contains("message too large"), "{reason}"),
            other => panic!("oversize must reject, got {other:?}"),
        }
        // Same length but NOT structured → plain (no oversize rule).
        let plain = "y".repeat(MAX_ENVELOPE_BYTES + 100);
        assert_eq!(cmd(&plain), Parsed::Plain(plain));
    }

    #[test]
    fn backend_spawn_mapping_matches_task_217() {
        assert_eq!(LlmBackend::Server.spawn_backend(), "lan_ollama");
        assert_eq!(LlmBackend::Desktop.spawn_backend(), "local_ollama");
        assert_eq!(LlmBackend::Gemini.spawn_backend(), "gemini");
    }
}
