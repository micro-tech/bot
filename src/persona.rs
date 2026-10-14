//! `agent.md` persona files (task 233).
//!
//! Helix and its sub-agents get editable, self-authoring personas. John's
//! locked design:
//!
//! - Persona + job description are SEPARATE blocks, persona first. The
//!   hardcoded `COMMANDER_SYSTEM_PROMPT` is the job block; `<runtime>/agent.md`
//!   (or `<runtime>/agents/<role>.md` for sub-agents) is the persona block.
//!   Missing file -> the job block plus the task-235 injection-boundary
//!   reminder (the pre-235 byte-identical guarantee was superseded).
//! - Each role file may declare a tool allowlist in YAML frontmatter. The
//!   persona module strips the frontmatter before injection (the model never
//!   sees the allowlist as prose); `dispatch_spawn` filters the sub-agent's
//!   toolset to the allowlist. Least-privilege containment: a compromised
//!   searcher literally cannot call file-write tools it was never given.
//! - Self-authoring is opt-in (`[helix.persona] self_author`, default false)
//!   and NEVER writes live files: drafts land as `.pending`, John approves
//!   via `/persona approve`. Revision guards: 4KB cap, 3 rotating backups,
//!   max 1 revision/file/hour, runaway tripwire.
//!
//! Why a hand-rolled frontmatter parser instead of serde_yaml: the only
//! machine-read key is `tools: [...]`; one more dependency for that would
//! just slow John's builds down.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

// ── Runtime dir + self-author gate ──────────────────────────────────────────

static PERSONA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Called once at startup (main.rs) with the runtime dir (parent of the
/// loaded config.toml, task 188 precedent).
pub fn set_persona_dir(dir: PathBuf) {
    let _ = PERSONA_DIR.set(dir);
}

fn persona_dir() -> PathBuf {
    PERSONA_DIR
        .get()
        .cloned()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

static SELF_AUTHOR: Mutex<bool> = Mutex::new(false);

/// Set at startup from `[helix.persona] self_author` in config.toml (main.rs).
/// Default false: persona self-authoring is strictly opt-in.
pub fn set_self_author_enabled(enabled: bool) {
    *SELF_AUTHOR.lock().unwrap_or_else(|e| e.into_inner()) = enabled;
}

pub fn self_author_enabled() -> bool {
    *SELF_AUTHOR.lock().unwrap_or_else(|e| e.into_inner())
}

// ── Persona loading ─────────────────────────────────────────────────────────

/// A loaded persona: prose body (frontmatter stripped) plus the optional
/// tool allowlist from the frontmatter.
#[derive(Debug, Clone)]
pub struct Persona {
    pub body: String,
    pub tools: Option<Vec<String>>,
}

/// Sanitize a role hint into a safe file stem: lowercase `[a-z0-9-_]`.
/// Anything smelling of path traversal (`/`, `\`, `..`) is rejected to
/// None (caller falls back to the default behavior).
pub fn sanitize_role(role: &str) -> Option<String> {
    if role.contains('/') || role.contains('\\') || role.contains("..") {
        return None;
    }
    let stem: String = role
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if stem.is_empty() {
        None
    } else {
        Some(stem)
    }
}

/// Resolve the persona file path. `None` role -> `<rt>/agent.md`;
/// `Some(role)` -> `<rt>/agents/<role>.md`. Returns None when the role
/// fails sanitization (caller uses the fallback path).
pub fn persona_path(role: Option<&str>) -> Option<PathBuf> {
    let dir = persona_dir();
    match role {
        None => Some(dir.join("agent.md")),
        Some(r) => sanitize_role(r).map(|stem| dir.join("agents").join(format!("{}.md", stem))),
    }
}

/// Split `---`-fenced YAML frontmatter off the top of a persona file.
/// Returns (body, tools). Only the `tools` key is machine-read; everything
/// else in the frontmatter is ignored. Supports inline (`tools: [a, b]`)
/// and dash-list forms.
fn split_frontmatter(text: &str) -> (String, Option<Vec<String>>) {
    let mut lines = text.lines();
    let first = lines.next().unwrap_or("");
    if first.trim() != "---" {
        return (text.to_string(), None);
    }
    let mut fm_lines: Vec<&str> = Vec::new();
    let mut body_start: Option<usize> = None;
    let mut idx = first.len() + 1; // byte offset past the first line
    for line in lines {
        if line.trim() == "---" {
            body_start = Some(idx + line.len() + 1);
            break;
        }
        fm_lines.push(line);
        idx += line.len() + 1;
    }
    let body_start = match body_start {
        Some(i) => i,
        None => return (text.to_string(), None), // unterminated fence: not frontmatter
    };
    let tools = parse_tools_key(&fm_lines);
    let body = text.get(body_start..).unwrap_or("").trim_start_matches('\n').to_string();
    (body, tools)
}

/// Parse the `tools:` key out of frontmatter lines. Returns None when the
/// key is absent (meaning: no allowlist declared -> current behavior).
fn parse_tools_key(fm_lines: &[&str]) -> Option<Vec<String>> {
    let mut iter = fm_lines.iter().peekable();
    while let Some(line) = iter.next() {
        let t = line.trim();
        if t.starts_with("tools:") {
            let rest = t["tools:".len()..].trim();
            if rest.starts_with('[') {
                // Inline form: tools: [web_search, gmail_read]
                let inner = rest.trim_start_matches('[');
                let inner = inner.split(']').next().unwrap_or("");
                let names: Vec<String> = inner
                    .split(',')
                    .map(|s| {
                        s.trim().trim_matches('"').trim_matches('\'').to_string()
                    })
                    .filter(|s| !s.is_empty())
                    .collect();
                return Some(names);
            } else if rest.is_empty() {
                // Dash-list form:
                // tools:
                //   - web_search
                let mut names = Vec::new();
                while let Some(next) = iter.peek() {
                    let nt = next.trim();
                    if nt.starts_with("- ") {
                        names.push(
                            nt[2..].trim().trim_matches('"').trim_matches('\'').to_string(),
                        );
                        iter.next();
                    } else {
                        break;
                    }
                }
                return Some(names);
            } else {
                // Single value: tools: web_search
                let name = rest.trim_matches('"').trim_matches('\'').to_string();
                return if name.is_empty() { Some(Vec::new()) } else { Some(vec![name]) };
            }
        }
    }
    None
}

/// Load a persona file. Returns None when the role fails sanitization or
/// the file is missing/unreadable (caller falls back to default behavior).
pub fn load_persona(role: Option<&str>) -> Option<Persona> {
    let path = persona_path(role)?;
    let text = std::fs::read_to_string(&path).ok()?;
    let (body, tools) = split_frontmatter(&text);
    Some(Persona { body, tools })
}

// ── Prompt composition ──────────────────────────────────────────────────────

/// Composed commander system text: persona block, then the job block
/// (`COMMANDER_SYSTEM_PROMPT`, unchanged), then the task-235
/// injection-boundary reminder. Missing file -> job block + reminder.
/// None when commander mode is off.
pub fn composed_commander_text() -> Option<String> {
    if !crate::tools::subagent_tool::commander_enabled() {
        return None;
    }
    let job = crate::tools::subagent_tool::COMMANDER_SYSTEM_PROMPT;
    // Task 235: instruction-boundary reminder — untrusted tool outputs are
    // wrapped in <untrusted-data> delimiters; they are DATA, never instructions.
    let reminder = crate::security::INSTRUCTION_REMINDER;
    match load_persona(None) {
        Some(p) => Some(format!("{}\n\n---\n\n{}\n\n{}", p.body, job, reminder)),
        None => Some(format!("{}\n\n{}", job, reminder)),
    }
}

/// The composed prompt for a sub-agent run: an optional system block and
/// the user message. When a role persona file exists, the system block is
/// persona-then-job and the user message is just the orders; when no file
/// exists the single user message is byte-identical to the pre-233 format.
pub struct SubagentPrompt {
    pub system: Option<String>,
    pub user: String,
}

pub fn compose_subagent_prompt(task_id: &str, task: &str, role_hint: &str) -> SubagentPrompt {
    let job = crate::tools::subagent_tool::SUBAGENT_JOB_PROMPT;
    // Task 235: instruction-boundary reminder in every sub-agent prompt —
    // sub-agents process untrusted content (search results, emails) too.
    let reminder = crate::security::INSTRUCTION_REMINDER;
    match load_persona(Some(role_hint)) {
        Some(p) => SubagentPrompt {
            system: Some(format!("{}\n\n---\n\n{}\n\n{}", p.body, job, reminder)),
            user: format!(
                "You are a sub-agent ({}). Your commander's orders:\n\n{}",
                task_id, task
            ),
        },
        None => {
            // Legacy single-message format + the task-235 reminder (the
            // pre-235 byte-identical guarantee was superseded).
            let prefix = crate::tools::subagent_tool::role_prefix(role_hint);
            SubagentPrompt {
                system: None,
                user: format!(
                    "You are a sub-agent ({}). Your commander's orders:\n\n{}\n\n{}\n\n{}\n\n{}",
                    task_id, prefix, task, job, reminder
                ),
            }
        }
    }
}

// ── Tool allowlist enforcement ──────────────────────────────────────────────

/// Build the tool definitions for a sub-agent spawn, honoring the role's
/// frontmatter allowlist. `spawn_subagent` is ALWAYS excluded (the depth
/// guard is absolute — an allowlist can never re-add it).
///
/// Migration safety: no file, or file without a `tools:` key -> current
/// behavior (everything except spawn_subagent). Unknown tool names are
/// warned about and ignored; if zero known names remain, we fall back to
/// the full set with a loud warning — a typo must never brick a role.
pub fn subagent_tool_definitions(role_hint: &str) -> serde_json::Value {
    let base = crate::tools::tool_definitions_excluding(&["spawn_subagent"]);
    let allowlist = match load_persona(Some(role_hint)).and_then(|p| p.tools) {
        Some(list) => list,
        None => return base,
    };

    let all_names: Vec<String> = crate::tools::tool_definitions()
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|d| {
                    d.get("function")?
                        .get("name")?
                        .as_str()
                        .map(|s| s.to_string())
                })
                .collect()
        })
        .unwrap_or_default();

    let mut unknown = Vec::new();
    let mut known: Vec<&str> = Vec::new();
    for name in &allowlist {
        if all_names.iter().any(|n| n == name) {
            known.push(name.as_str());
        } else {
            unknown.push(name.clone());
        }
    }
    for name in &unknown {
        log::warn!(
            "persona: role '{}' allowlists unknown tool '{}'; ignoring",
            role_hint,
            name
        );
    }
    if known.is_empty() {
        log::warn!(
            "persona: role '{}' allowlist matched ZERO known tools ({} unknown); \
             falling back to the full toolset so the role is not bricked",
            role_hint,
            unknown.len()
        );
        return base;
    }

    let arr = base.as_array().cloned().unwrap_or_default();
    let filtered: Vec<serde_json::Value> = arr
        .into_iter()
        .filter(|d| {
            let name = d
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");
            // spawn_subagent is already gone from `base`, but belt and
            // suspenders: never let it back in through an allowlist.
            name != "spawn_subagent" && known.contains(&name)
        })
        .collect();
    serde_json::Value::Array(filtered)
}

// ── Revision guards ─────────────────────────────────────────────────────────

static REVISION_TIMES: OnceLock<Mutex<HashMap<PathBuf, Instant>>> = OnceLock::new();

fn revision_times() -> &'static Mutex<HashMap<PathBuf, Instant>> {
    REVISION_TIMES.get_or_init(|| Mutex::new(HashMap::new()))
}

static TRIPWIRE: OnceLock<Mutex<HashMap<PathBuf, Vec<Instant>>>> = OnceLock::new();

fn tripwire() -> &'static Mutex<HashMap<PathBuf, Vec<Instant>>> {
    TRIPWIRE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record a revision proposal. Returns false when the per-file rate limit
/// (1/hour) refuses it. Three proposals within the hour — which the limiter
/// should make impossible — trips the runaway guard: self-authoring is
/// disabled process-wide and the caller must alert John.
pub fn note_revision(path: &Path) -> Result<(), String> {
    // Tripwire first (defense in depth).
    {
        let mut tw = tripwire().lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let attempts = tw.entry(path.to_path_buf()).or_default();
        attempts.retain(|t| now.duration_since(*t) < Duration::from_secs(3600));
        attempts.push(now);
        if attempts.len() >= 3 {
            set_self_author_enabled(false);
            return Err(format!(
                "persona runaway tripwire: 3 revision proposals for {} within the hour; \
                 self-authoring DISABLED process-wide. John must re-enable it.",
                path.display()
            ));
        }
    }
    let mut times = revision_times().lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    if let Some(&last) = times.get(path) {
        if now.duration_since(last) < Duration::from_secs(3600) {
            return Err(format!(
                "persona revision rate limit: {} was already revised within the last hour",
                path.display()
            ));
        }
    }
    times.insert(path.to_path_buf(), now);
    Ok(())
}

/// Rotate backups: `path` -> `path.bak.1`, shifting older ones down, keeping
/// exactly 3. Append-only history: nothing is ever silently overwritten.
pub fn rotate_backups(path: &Path) -> io::Result<()> {
    let bak = |n: u8| {
        let mut s = path.as_os_str().to_owned();
        s.push(format!(".bak.{}", n));
        PathBuf::from(s)
    };
    let _ = std::fs::remove_file(bak(3));
    if bak(2).exists() {
        std::fs::rename(bak(2), bak(3))?;
    }
    if bak(1).exists() {
        std::fs::rename(bak(1), bak(2))?;
    }
    if path.exists() {
        std::fs::rename(path, bak(1))?;
    }
    Ok(())
}

/// Maximum persona file size: revisions beyond this are refused.
pub const MAX_PERSONA_BYTES: u64 = 4096;

/// Apply a `.pending` persona draft: enforce the size cap, rotate backups,
/// then move the pending file live. Returns a human-readable summary.
pub fn apply_pending_persona(live_path: &Path) -> Result<String, String> {
    let pending = pending_path(live_path);
    if !pending.is_file() {
        return Err(format!(
            "no pending persona draft at {}; nothing to approve",
            pending.display()
        ));
    }
    let size = std::fs::metadata(&pending)
        .map_err(|e| e.to_string())?
        .len();
    if size > MAX_PERSONA_BYTES {
        return Err(format!(
            "pending persona is {} bytes, over the {}-byte cap; refusing to apply",
            size, MAX_PERSONA_BYTES
        ));
    }
    rotate_backups(live_path).map_err(|e| e.to_string())?;
    std::fs::rename(&pending, live_path).map_err(|e| e.to_string())?;
    Ok(format!(
        "applied {} ({} bytes); previous version kept as {}.bak.1",
        live_path.display(),
        size,
        live_path.display()
    ))
}

/// Discard a `.pending` persona draft without applying it.
pub fn discard_pending_persona(live_path: &Path) -> Result<String, String> {
    let pending = pending_path(live_path);
    if !pending.is_file() {
        return Err(format!(
            "no pending persona draft at {}; nothing to discard",
            pending.display()
        ));
    }
    std::fs::remove_file(&pending).map_err(|e| e.to_string())?;
    Ok(format!("discarded pending draft {}", pending.display()))
}

fn pending_path(live_path: &Path) -> PathBuf {
    let mut s = live_path.as_os_str().to_owned();
    s.push(".pending");
    PathBuf::from(s)
}

/// List pending drafts under the persona dir (for `/persona status`).
pub fn list_pending() -> Vec<PathBuf> {
    let dir = persona_dir();
    let mut out = Vec::new();
    for candidate in [
        dir.join("agent.md.pending"),
        dir.join("agents"),
    ] {
        if candidate.is_file() {
            out.push(candidate);
        } else if candidate.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&candidate) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.extension().and_then(|x| x.to_str()) == Some("pending") {
                        out.push(p);
                    }
                }
            }
        }
    }
    out
}

// ── Self-authoring ──────────────────────────────────────────────────────────

/// Draft persona text via a meta-prompt. With `HELIX_PERSONA_TEST_STUB=1`
/// returns canned text (no LLM needed); otherwise one cheap call on the
/// local Ollama endpoint, no tools.
pub async fn draft_persona_text(role: Option<&str>) -> Result<String, String> {
    if std::env::var("HELIX_PERSONA_TEST_STUB").is_ok() {
        let who = role.unwrap_or("helix");
        return Ok(format!(
            "# Draft persona: {}\n\nYou are {} — a test-stub persona draft. \
             Replace this with who you actually are: name, role, tone, and \
             how you work. Keep it short; the job description follows separately.",
            who, who
        ));
    }
    let subject = match role {
        Some(r) => format!("a sub-agent with role '{}'", r),
        None => "Helix, John's personal AI agent".to_string(),
    };
    let meta = format!(
        "You are {}. Write your agent.md persona file: who you are, how you \
         work, your tone. Freeform markdown, no YAML frontmatter, under 2KB. \
         Write ONLY the file content, no preamble.",
        subject
    );
    let backend = crate::router::LLMBackend::LocalOllama;
    let ep = crate::tools::subagent_tool::resolve_ollama_endpoint(&backend)?;
    let client = crate::io::ollama::llm_impl::OllamaLlmImpl::new(ep.url, ep.model);
    use crate::io::ollama::llm_trait::OllamaLlm;
    let messages = vec![serde_json::json!({"role": "user", "content": meta})];
    client
        .chat(&messages)
        .await
        .map_err(|e| format!("persona draft LLM call failed: {}", e))
}

/// Ensure a persona file exists, drafting it when self-authoring is on.
/// Never writes live: drafts land as `.pending` for John's approval.
/// Returns the live path when it already exists, else the pending path.
pub async fn ensure_persona_file(role: Option<&str>) -> Result<PathBuf, String> {
    let live = persona_path(role).ok_or_else(|| "invalid role name".to_string())?;
    if live.is_file() {
        return Ok(live);
    }
    if !self_author_enabled() {
        return Err(
            "no persona file and self-authoring is disabled \
             ([helix.persona] self_author). Create the file by hand or enable self-authoring."
                .to_string(),
        );
    }
    if let Some(parent) = live.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let draft = draft_persona_text(role).await?;
    if draft.len() as u64 > MAX_PERSONA_BYTES {
        return Err(format!(
            "drafted persona is {} bytes, over the {}-byte cap; refusing to stage",
            draft.len(),
            MAX_PERSONA_BYTES
        ));
    }
    let pending = pending_path(&live);
    std::fs::write(&pending, draft).map_err(|e| e.to_string())?;
    log::info!(
        "persona: drafted {} -> {} (pending John's approval)",
        live.display(),
        pending.display()
    );
    Ok(pending)
}

/// Propose a lesson-learned revision to a role's persona file (revision
/// loop trigger: sub-agent ended Failed/TimedOut). Appends to the
/// `.pending` draft (seeded from the live file), never live. Enforces the
/// rate limiter and tripwire.
pub fn propose_lesson(role: &str, lesson: &str) -> Result<PathBuf, String> {
    let live = persona_path(Some(role)).ok_or_else(|| "invalid role name".to_string())?;
    if !live.is_file() {
        return Err(format!(
            "no persona file for role '{}'; nothing to revise",
            role
        ));
    }
    note_revision(&live)?;
    let pending = pending_path(&live);
    let current = std::fs::read_to_string(&pending)
        .or_else(|_| std::fs::read_to_string(&live))
        .map_err(|e| e.to_string())?;
    let entry = format!(
        "\n\n## Lessons learned\n\n- {}",
        lesson.trim()
    );
    let mut revised = current;
    if !revised.contains("## Lessons learned") {
        revised.push_str(&entry);
    } else {
        revised.push_str(&format!("\n- {}", lesson.trim()));
    }
    if revised.len() as u64 > MAX_PERSONA_BYTES {
        return Err(format!(
            "revision would exceed the {}-byte cap; refusing",
            MAX_PERSONA_BYTES
        ));
    }
    if let Some(parent) = pending.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&pending, revised).map_err(|e| e.to_string())?;
    log::info!(
        "persona: lesson proposed for role '{}' -> {} (pending approval)",
        role,
        pending.display()
    );
    Ok(pending)
}

#[cfg(test)]
pub fn reset_revision_guards() {
    revision_times().lock().unwrap_or_else(|e| e.into_inner()).clear();
    tripwire().lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// Test-only: reset just the rate limiter, keeping tripwire history.
/// Lets tests simulate the "limiter bypassed" burst the tripwire guards.
#[cfg(test)]
pub fn reset_rate_limiter_only() {
    revision_times().lock().unwrap_or_else(|e| e.into_inner()).clear();
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    static DIR_LOCK: OnceLock<StdMutex<()>> = OnceLock::new();
    fn dir_lock() -> std::sync::MutexGuard<'static, ()> {
        DIR_LOCK
            .get_or_init(|| StdMutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Point the persona dir at a fresh temp dir for this test.
    fn use_temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("helix-persona-test-{}", tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();
        // OnceLock: only the first test's dir sticks process-wide, so make
        // the tag part of the path AND chdir-independent by writing files
        // through persona_path() after setting once. Simpler: since the dir
        // is process-global, all tests share one temp root namespaced per
        // test via distinct filenames.
        static INIT: OnceLock<()> = OnceLock::new();
        INIT.get_or_init(|| set_persona_dir(dir.clone()));
        dir
    }

    fn write_persona(role: Option<&str>, content: &str) -> PathBuf {
        let path = persona_path(role).unwrap();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        // Namespace per test through a marker comment so the shared temp
        // dir never collides.
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn load_present_returns_content() {
        let _l = dir_lock();
        let _d = use_temp_dir("load");
        // GIVEN a persona file with frontmatter + prose:
        write_persona(
            None,
            "---\ntools: [web_search]\n---\n# Loader\n\nI load things.",
        );
        // WHEN we load it,
        let p = load_persona(None).expect("persona should load");
        // THEN the body and allowlist come back parsed:
        assert!(p.body.contains("I load things"), "body: {}", p.body);
        assert_eq!(p.tools, Some(vec!["web_search".to_string()]));
    }

    #[test]
    fn frontmatter_stripped_from_injected_persona() {
        let _l = dir_lock();
        let _d = use_temp_dir("strip");
        // GIVEN a role file with a tools allowlist in frontmatter:
        write_persona(
            Some("stripper"),
            "---\ntools:\n  - web_search\n  - gmail_read\n---\n# Stripper\n\nSearches the web.",
        );
        let p = load_persona(Some("stripper")).expect("should load");
        // THEN the injected body never mentions the allowlist:
        assert!(!p.body.contains("web_search"), "body leaked allowlist: {}", p.body);
        assert!(!p.body.contains("---"), "body leaked fence: {}", p.body);
        assert!(p.body.contains("Searches the web"), "body: {}", p.body);
        assert_eq!(
            p.tools,
            Some(vec!["web_search".to_string(), "gmail_read".to_string()])
        );
    }

    #[test]
    fn load_missing_returns_none() {
        let _l = dir_lock();
        let _d = use_temp_dir("missing");
        // GIVEN no file for this role,
        // WHEN we load,
        // THEN we get None (caller falls back to default behavior):
        assert!(load_persona(Some("definitely-not-a-real-role-xyz")).is_none());
    }

    #[test]
    fn compose_order_persona_then_job() {
        let _l = dir_lock();
        let _d = use_temp_dir("order");
        // GIVEN a persona file exists:
        write_persona(None, "# Orderly\n\nI am very orderly.");
        // WHEN we compose the commander text,
        let text = composed_commander_text().expect("commander defaults to enabled");
        // THEN persona comes first, job block second:
        let persona_at = text.find("I am very orderly").expect("persona missing");
        let job_at = text
            .find("You are the commander")
            .expect("job block missing");
        assert!(persona_at < job_at, "persona must precede job block");
    }

    #[test]
    fn compose_missing_file_appends_injection_reminder() {
        let _l = dir_lock();
        let _d = use_temp_dir("identical");
        // GIVEN the shared temp dir has no agent.md for this check: use a
        // role-free load against a guaranteed-absent path by removing it.
        if let Some(p) = persona_path(None) {
            let _ = std::fs::remove_file(&p);
        }
        // WHEN no persona file exists,
        let text = composed_commander_text().expect("commander defaults to enabled");
        // THEN the job block is intact and the task-235 injection reminder
        // is appended (the pre-235 byte-identical guarantee was superseded
        // by the instruction-boundary requirement):
        assert!(text.starts_with(crate::tools::subagent_tool::COMMANDER_SYSTEM_PROMPT));
        assert!(text.contains(crate::security::INSTRUCTION_REMINDER));
    }

    #[test]
    fn role_sanitization_rejects_traversal() {
        // GIVEN traversal-y role hints,
        // WHEN sanitized,
        // THEN they are rejected (None -> caller falls back):
        assert!(sanitize_role("../../etc").is_none());
        assert!(sanitize_role("a/b").is_none());
        assert!(sanitize_role("..").is_none());
        assert!(sanitize_role("..\\windows").is_none());
        assert!(sanitize_role("").is_none());
        assert!(sanitize_role("!!!").is_none());
        // AND normal roles pass through lowercased:
        assert_eq!(sanitize_role("Searcher"), Some("searcher".to_string()));
        assert_eq!(sanitize_role("code-builder_2"), Some("code-builder_2".to_string()));
    }

    #[test]
    fn revision_rate_limiter_refuses_second_write_within_hour() {
        let _l = dir_lock();
        reset_revision_guards();
        let path = PathBuf::from("/tmp/helix-persona-test-rate/agent.md");
        // GIVEN one recorded revision,
        assert!(note_revision(&path).is_ok());
        // WHEN a second revision is proposed within the hour,
        // THEN the limiter refuses it:
        assert!(note_revision(&path).is_err());
    }

    #[test]
    fn runaway_tripwire_disables_self_authoring() {
        let _l = dir_lock();
        reset_revision_guards();
        set_self_author_enabled(true);
        let a = PathBuf::from("/tmp/helix-persona-test-trip/a.md");
        // GIVEN two proposals for the same file with the limiter reset
        // between them (simulating a limiter bypass — defense in depth),
        assert!(note_revision(&a).is_ok());
        reset_rate_limiter_only();
        assert!(note_revision(&a).is_ok());
        reset_rate_limiter_only();
        // WHEN a third proposal lands within the hour,
        let err = note_revision(&a).unwrap_err();
        // THEN the tripwire fires and self-authoring is off:
        assert!(err.contains("tripwire"), "got: {}", err);
        assert!(!self_author_enabled(), "tripwire must disable self-authoring");
        set_self_author_enabled(false);
    }

    #[test]
    fn bak_rotation_keeps_exactly_3() {
        let _l = dir_lock();
        let dir = std::env::temp_dir().join("helix-persona-test-bak");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let live = dir.join("agent.md");
        // GIVEN a live file,
        std::fs::write(&live, "v0").unwrap();
        // WHEN we rotate 5 times,
        for i in 1..=5 {
            std::fs::write(&live, format!("v{}", i)).unwrap();
            rotate_backups(&live).unwrap();
            std::fs::write(&live, format!("v{}", i)).unwrap();
        }
        // THEN exactly .bak.1..3 exist, .bak.4 never does:
        assert!(dir.join("agent.md.bak.1").is_file());
        assert!(dir.join("agent.md.bak.2").is_file());
        assert!(dir.join("agent.md.bak.3").is_file());
        assert!(!dir.join("agent.md.bak.4").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_tools_frontmatter_inline_and_list_forms() {
        // GIVEN inline form,
        let (body, tools) = split_frontmatter("---\ntools: [a, b]\n---\nHello");
        assert_eq!(tools, Some(vec!["a".to_string(), "b".to_string()]));
        assert_eq!(body, "Hello");
        // GIVEN dash-list form,
        let (body, tools) = split_frontmatter("---\ntools:\n  - a\n  - b\n---\nHello");
        assert_eq!(tools, Some(vec!["a".to_string(), "b".to_string()]));
        assert_eq!(body, "Hello");
        // GIVEN no frontmatter,
        let (body, tools) = split_frontmatter("# Just prose");
        assert_eq!(tools, None);
        assert_eq!(body, "# Just prose");
        // GIVEN frontmatter without a tools key,
        let (body, tools) = split_frontmatter("---\nmood: grumpy\n---\nHello");
        assert_eq!(tools, None);
        assert_eq!(body, "Hello");
    }

    #[test]
    fn unknown_tool_names_warned_and_ignored() {
        let _l = dir_lock();
        let _d = use_temp_dir("unknown");
        // GIVEN a role allowlisting one real and one bogus tool:
        write_persona(
            Some("picky"),
            "---\ntools: [read_email, not_a_real_tool_xyz]\n---\n# Picky\n\nPicky role.",
        );
        // WHEN we build its toolset,
        let defs = subagent_tool_definitions("picky");
        let names: Vec<String> = defs
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|d| {
                d.get("function")?.get("name")?.as_str().map(|s| s.to_string())
            })
            .collect();
        // THEN the bogus name is ignored, the real one survives:
        assert!(!names.iter().any(|n| n == "not_a_real_tool_xyz"));
        assert!(names.iter().any(|n| n == "read_email"), "visible: {:?}", names);
    }

    #[test]
    fn empty_after_filter_falls_back_to_full_set() {
        let _l = dir_lock();
        let _d = use_temp_dir("emptyfilter");
        // GIVEN an allowlist of only bogus names,
        write_persona(
            Some("bricked"),
            "---\ntools: [nope_xyz, nah_xyz]\n---\n# Bricked\n\nWould be bricked.",
        );
        // WHEN we build its toolset,
        let defs = subagent_tool_definitions("bricked");
        let full = crate::tools::tool_definitions_excluding(&["spawn_subagent"]);
        // THEN we fall back to the full set (a typo must not brick the role):
        assert_eq!(
            defs.as_array().map(|a| a.len()),
            full.as_array().map(|a| a.len()),
            "zero known tools must fall back to the full set"
        );
    }

    #[test]
    fn spawn_subagent_never_allowlisted() {
        let _l = dir_lock();
        let _d = use_temp_dir("nodepth");
        // GIVEN a role that explicitly allowlists spawn_subagent (naughty):
        write_persona(
            Some("ambitious"),
            "---\ntools: [spawn_subagent, read_email]\n---\n# Ambitious\n\nWants to delegate.",
        );
        // WHEN we build its toolset,
        let defs = subagent_tool_definitions("ambitious");
        let names: Vec<String> = defs
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|d| {
                d.get("function")?.get("name")?.as_str().map(|s| s.to_string())
            })
            .collect();
        // THEN the depth guard holds absolutely:
        assert!(
            !names.iter().any(|n| n == "spawn_subagent"),
            "spawn_subagent must never appear in a sub-agent toolset"
        );
        assert!(names.iter().any(|n| n == "read_email"));
    }

    #[test]
    fn no_file_means_unfiltered_toolset() {
        let _l = dir_lock();
        let _d = use_temp_dir("nofile");
        // GIVEN no persona file for the role,
        // WHEN we build its toolset,
        let defs = subagent_tool_definitions("ghost-role-xyz");
        let full = crate::tools::tool_definitions_excluding(&["spawn_subagent"]);
        // THEN it matches today's behavior exactly:
        assert_eq!(
            defs.as_array().map(|a| a.len()),
            full.as_array().map(|a| a.len())
        );
    }

    #[test]
    fn searcher_allowlist_hides_file_writing_tools() {
        let _l = dir_lock();
        let _d = use_temp_dir("searcher");
        // GIVEN John's canonical example: a read-only researcher role,
        write_persona(
            Some("searcher"),
            "---\ntools: [read_email, check_inbox]\n---\n# Searcher\n\nReads mail. Never touches files.",
        );
        // WHEN we build its toolset,
        let defs = subagent_tool_definitions("searcher");
        let names: Vec<String> = defs
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|d| {
                d.get("function")?.get("name")?.as_str().map(|s| s.to_string())
            })
            .collect();
        let blob = names.join(" ");
        // THEN no shell, file-write, or send tools are visible:
        for dangerous in ["run_shell", "write_note", "send_email", "spawn_subagent"] {
            assert!(
                !names.iter().any(|n| n == dangerous),
                "allowlist leaked '{}'; visible: {}",
                dangerous,
                blob
            );
        }
        assert!(names.iter().any(|n| n == "read_email"));
    }

    #[test]
    fn pending_round_trip_apply_and_discard() {
        let _l = dir_lock();
        let dir = std::env::temp_dir().join("helix-persona-test-pending");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let live = dir.join("agent.md");
        std::fs::write(&live, "live v1").unwrap();
        let mut pending_os = live.as_os_str().to_owned();
        pending_os.push(".pending");
        let pending = PathBuf::from(pending_os);
        // GIVEN a pending draft,
        std::fs::write(&pending, "draft v2").unwrap();
        // WHEN John approves it,
        let msg = apply_pending_persona(&live).expect("apply should work");
        // THEN it goes live and the old version is backed up:
        assert!(msg.contains("applied"), "got: {}", msg);
        assert_eq!(std::fs::read_to_string(&live).unwrap(), "draft v2");
        assert_eq!(
            std::fs::read_to_string(dir.join("agent.md.bak.1")).unwrap(),
            "live v1"
        );
        assert!(!pending.exists(), "pending file must be consumed");
        // AND discarding with nothing pending is a clean error:
        assert!(discard_pending_persona(&live).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_refuses_oversize_pending() {
        let _l = dir_lock();
        let dir = std::env::temp_dir().join("helix-persona-test-big");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let live = dir.join("agent.md");
        std::fs::write(&live, "live").unwrap();
        let mut pending_os = live.as_os_str().to_owned();
        pending_os.push(".pending");
        let pending = PathBuf::from(pending_os);
        // GIVEN a pending draft over the 4KB cap,
        std::fs::write(&pending, "x".repeat(5000)).unwrap();
        // WHEN John tries to approve it,
        // THEN it is refused and the live file is untouched:
        let err = apply_pending_persona(&live).unwrap_err();
        assert!(err.contains("over"), "got: {}", err);
        assert_eq!(std::fs::read_to_string(&live).unwrap(), "live");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stub_draft_writes_pending_not_live() {
        let _l = dir_lock();
        let _d = use_temp_dir("stubdraft");
        // SAFETY: serialized by dir_lock; stub only affects this process.
        unsafe { std::env::set_var("HELIX_PERSONA_TEST_STUB", "1") };
        set_self_author_enabled(true);
        let rt = tokio::runtime::Runtime::new().unwrap();
        // GIVEN no persona file and self-authoring on,
        // WHEN we ensure the persona file (stubbed draft),
        let out = rt.block_on(ensure_persona_file(Some("stubrole")));
        unsafe { std::env::remove_var("HELIX_PERSONA_TEST_STUB") };
        set_self_author_enabled(false);
        // THEN a .pending draft is staged, never live:
        let pending = out.expect("ensure should stage a pending draft");
        assert!(
            pending.to_string_lossy().ends_with(".pending"),
            "got: {}",
            pending.display()
        );
        assert!(pending.is_file());
        // AND applying it makes it live:
        let live_path = persona_path(Some("stubrole")).unwrap();
        assert!(!live_path.is_file(), "live file must not exist before approval");
        apply_pending_persona(&live_path).expect("apply should work");
        assert!(load_persona(Some("stubrole")).is_some());
    }
}
