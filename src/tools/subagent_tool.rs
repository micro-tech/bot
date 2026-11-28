//! `spawn_subagent` tool — commander delegation (task 214).
//!
//! Lets any backend John chats with (the "commander") spin up a sub-agent on
//! a chosen backend. The tool blocks (with a configurable timeout) and hands
//! the sub-agent's final message back to the commander.
//!
//! Guardrails:
//! - Max spawn depth 1: a sub-agent's own toolset excludes `spawn_subagent`
//!   AND a task-local depth counter refuses nested calls. No delegation loops.
//! - API backends (Gemini/Grok) go through the spend counter
//!   (`crate::router::spend`); refused at the daily cap.
//! - Timeouts: the sub-agent run is wrapped in `tokio::time::timeout`; on
//!   expiry the worker is cancelled and the commander gets a timeout report.

use serde_json::Value;
use std::cell::Cell;
use std::sync::OnceLock;
use std::time::Duration;

use crate::io::ollama::llm_trait::OllamaLlm;
use crate::router::{LLMBackend, RouterConfig, RoutingContext};

// ── Spawn depth guard ─────────────────────────────────────────────────────────

/// Maximum nesting: the commander (depth 0) may spawn; a sub-agent (depth 1)
/// may not spawn further.
pub const MAX_SPAWN_DEPTH: u32 = 1;

tokio::task_local! {
    static SPAWN_DEPTH: Cell<u32>;
}

// ── Commander system prompt (task 215) ────────────────────────────────────────────

/// Short commander instruction block prepended as the system message on every
/// backend chat path. Kept tight: local models have small context windows.
pub const COMMANDER_SYSTEM_PROMPT: &str = "You are the commander. When the user asks for work, \
break it into pieces and delegate each piece with the spawn_subagent tool, picking the backend \
per piece (local_ollama: fast and free; lan_ollama: overflow; gemini: heavy reasoning, costs \
API budget). Do the orchestration yourself; do not do the pieces inline. Do not ask clarifying \
questions when the request is actionable -- act, then report a concise summary when sub-agents finish.";

/// Sub-agent job block (task 233): the standing orders every worker gets.
/// Extracted from run_subagent()'s old inline format string so the persona
/// module can compose it after the persona block. Byte-identical to the
/// pre-233 text — do not "improve" the wording without checking the
/// byte-identical fallback test in persona.rs.
pub const SUBAGENT_JOB_PROMPT: &str = "Complete the task with your tools. Do not ask clarifying questions — act. \
Finish with a concise summary of what you did.";

static COMMANDER_ENABLED_OVERRIDE: OnceLock<bool> = OnceLock::new();

/// Set at startup from `[helix.commander] enabled` in config.toml (main.rs).
pub fn set_commander_enabled(enabled: bool) {
    let _ = COMMANDER_ENABLED_OVERRIDE.set(enabled);
}

/// Build the system message for chat paths, or None when commander mode is off.
/// Chat loops prepend this to the message list (Ollama) or map it to the
/// backend's native system-instruction field (Gemini).
/// Task 233: the text is composed by the persona module (persona block +
/// job block); with no agent.md it is byte-identical to COMMANDER_SYSTEM_PROMPT.
pub fn commander_system_message() -> Option<Value> {
    crate::persona::composed_commander_text().map(|text| {
        serde_json::json!({
            "role": "system",
            "content": text
        })
    })
}

/// Whether the commander prompt block is active. Default true (task 214
/// landed); override via `[helix.commander] enabled` or HELIX_COMMANDER_ENABLED=0/1.
pub fn commander_enabled() -> bool {
    if let Some(v) = COMMANDER_ENABLED_OVERRIDE.get() {
        return *v;
    }
    match std::env::var("HELIX_COMMANDER_ENABLED").as_deref() {
        Ok("0") | Ok("false") | Ok("no") => false,
        Ok("1") | Ok("true") | Ok("yes") => true,
        _ => true,
    }
}

// ── Backend endpoint registry ─────────────────────────────────────────────────

/// URL + model for one Ollama backend.
#[derive(Debug, Clone)]
pub struct OllamaEndpoint {
    pub url: String,
    pub model: String,
}

#[derive(Debug, Clone, Default)]
struct BackendEndpoints {
    local: Option<OllamaEndpoint>,
    lan: Option<OllamaEndpoint>,
}

static ENDPOINTS: OnceLock<BackendEndpoints> = OnceLock::new();

/// Called once at startup (main.rs) from the `[[ollama]]` config entries.
pub fn set_ollama_endpoints(local: Option<OllamaEndpoint>, lan: Option<OllamaEndpoint>) {
    let _ = ENDPOINTS.set(BackendEndpoints { local, lan });
}

pub(crate) fn resolve_ollama_endpoint(backend: &LLMBackend) -> Result<OllamaEndpoint, String> {
    if let Some(reg) = ENDPOINTS.get() {
        let hit = match backend {
            LLMBackend::LocalOllama => reg.local.as_ref(),
            LLMBackend::LanOllama => reg.lan.as_ref(),
            _ => None,
        };
        if let Some(ep) = hit {
            return Ok(ep.clone());
        }
    }
    // Env fallback (also what unit tests use).
    let (url_var, model_var, def_url) = match backend {
        LLMBackend::LocalOllama => (
            "HELIX_SUBAGENT_LOCAL_URL",
            "HELIX_SUBAGENT_LOCAL_MODEL",
            "http://localhost:11434",
        ),
        LLMBackend::LanOllama => (
            "HELIX_SUBAGENT_LAN_URL",
            "HELIX_SUBAGENT_LAN_MODEL",
            "http://192.168.1.149:11434",
        ),
        _ => return Err(format!("{:?} is not an Ollama backend", backend)),
    };
    Ok(OllamaEndpoint {
        url: std::env::var(url_var).unwrap_or_else(|_| def_url.to_string()),
        model: std::env::var(model_var).unwrap_or_else(|_| "gpt-oss:20b".to_string()),
    })
}

// ── Arg parsing ───────────────────────────────────────────────────────────────

const DEFAULT_TIMEOUT_SECS: u64 = 300;
const MAX_TIMEOUT_SECS: u64 = 1800;

fn parse_backend(s: &str) -> Result<LLMBackend, String> {
    match s.trim().to_lowercase().as_str() {
        "" | "auto" => Err("__auto__".to_string()), // sentinel: resolve via router
        "local_ollama" | "local" => Ok(LLMBackend::LocalOllama),
        "lan_ollama" | "lan" | "server" => Ok(LLMBackend::LanOllama),
        "gemini" => Ok(LLMBackend::Gemini),
        "grok" => Ok(LLMBackend::Grok),
        other => Err(format!(
            "Unknown backend '{}'. Use one of: local_ollama | lan_ollama | gemini | grok | auto.",
            other
        )),
    }
}

fn looks_like_code(task: &str) -> bool {
    let t = task.to_lowercase();
    [
        "rust", "code", "function", "compile", "cargo", "python", "bug", "refactor", "api",
    ]
    .iter()
    .any(|w| t.contains(w))
}

fn resolve_backend_auto(task: &str) -> LLMBackend {
    let ctx = RoutingContext {
        prompt: task.to_string(),
        token_estimate: task.len() / 4,
        has_code: looks_like_code(task),
        complexity_score: 0.0, // let route() compute it
        timestamp: chrono::Utc::now(),
        user_override: None,
        telemetry: None,
        health: None,
    };
    match crate::router::route(&ctx, &RouterConfig::default()) {
        LLMBackend::Grok => {
            // Grok has no client implementation yet — fall back to Gemini.
            log::warn!(
                "spawn_subagent: router picked Grok (no client yet); falling back to Gemini"
            );
            LLMBackend::Gemini
        }
        LLMBackend::Fallback => LLMBackend::LocalOllama,
        b => b,
    }
}

pub(crate) fn role_prefix(role_hint: &str) -> String {
    let hint = role_hint.trim();
    if hint.is_empty() {
        return String::new();
    }
    let known = match hint.to_lowercase().as_str() {
        "planner" => "Act as a planner: break the work down, no code changes unless asked. ",
        "builder" => "Act as a builder: implement and verify with tests. ",
        "reviewer" => "Act as a code reviewer: read the code, report issues by severity. ",
        "ui" => "Act as a frontend builder: HTML/CSS/JS work. ",
        _ => "",
    };
    if known.is_empty() {
        format!("Role: {}. ", hint)
    } else {
        format!("Role: {}. {} ", hint, known)
    }
}

// ── Tool entry point (called from tools::execute) ─────────────────────────────

/// Parsed `spawn_subagent` arguments. Shared by the sync tool entry and the
/// cron job-pump (task 216) so both dispatch through one path.
#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub task: String,
    pub backend: String,
    pub role_hint: String,
    pub timeout_secs: u64,
}

/// Parse tool args into a SpawnSpec. Used by `spawn_subagent` and by
/// `Routine` -> spec conversion for cron payloads.
pub fn parse_spawn_args(args: &Value) -> Result<SpawnSpec, String> {
    let task = args
        .get("task")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if task.is_empty() {
        return Err(
            "spawn_subagent requires a non-empty 'task' string describing the marching orders."
                .to_string(),
        );
    }
    Ok(SpawnSpec {
        task: task.to_string(),
        backend: args
            .get("backend")
            .and_then(|v| v.as_str())
            .unwrap_or("auto")
            .to_string(),
        role_hint: args
            .get("role_hint")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        timeout_secs: args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .clamp(1, MAX_TIMEOUT_SECS),
    })
}

fn depth_refused() -> Option<String> {
    let depth = SPAWN_DEPTH.try_with(|c| c.get()).unwrap_or(0);
    if depth >= MAX_SPAWN_DEPTH {
        Some(format!(
            "spawn_subagent refused: sub-agents may not spawn further sub-agents \
             (max spawn depth is {}). Complete your task with the tools you have \
             and report back to your commander.",
            MAX_SPAWN_DEPTH
        ))
    } else {
        None
    }
}

/// Resolve + validate the backend choice (explicit wins, auto -> router)
/// and enforce the API spend gate. Shared by the sync tool entry (so arg
/// errors surface before any runtime is needed) and `dispatch_spawn`.
fn prepare_backend(spec: &SpawnSpec) -> Result<LLMBackend, String> {
    let backend = match parse_backend(&spec.backend) {
        Ok(b) => b,
        Err(s) if s == "__auto__" => resolve_backend_auto(&spec.task),
        Err(e) => return Err(format!("Error: {}", e)),
    };
    if backend == LLMBackend::Grok {
        return Err("Error: the Grok backend has no client implementation yet. \
                Pick local_ollama, lan_ollama, gemini, or auto."
            .to_string());
    }
    // Spend gate for paid backends (hook for task 217's policy).
    if let Err(msg) = crate::router::check_api_allowed(&backend) {
        return Err(format!("spawn_subagent refused: {}", msg));
    }
    Ok(backend)
}

/// Test seam: when set, dispatch returns a canned report instead of calling
/// a live LLM. Checked in both the sync entry and `dispatch_spawn`.
fn test_stub_report(spec: &SpawnSpec) -> Option<String> {
    if std::env::var("HELIX_SUBAGENT_TEST_STUB").as_deref() == Ok("1") {
        Some(format!(
            "Sub-agent complete\ntask_id: subagent-test-stub\nbackend: test_stub\nrole: {}\nfinal:\n[stub] {}",
            role_hint_or_any(&spec.role_hint),
            spec.task.chars().take(200).collect::<String>()
        ))
    } else {
        None
    }
}

/// Short backend label for the sub-agent registry / web UI. The commander
/// report keeps the historical `{:?}`-lowercased form; the UI gets this.
fn backend_short_name(backend: &LLMBackend) -> &'static str {
    match backend {
        LLMBackend::LocalOllama => "local",
        LLMBackend::LanOllama => "lan",
        LLMBackend::Gemini => "gemini",
        LLMBackend::Grok => "grok",
        LLMBackend::Fallback => "fallback",
    }
}

/// Backend ladder, fastest/cheapest first. Health-aware fallback walks DOWN
/// this list from the requested backend (task 217.3). Grok is parked at the
/// bottom as not-implemented — the walker never selects it.
const BACKEND_LADDER: [LLMBackend; 3] = [
    LLMBackend::LocalOllama,
    LLMBackend::LanOllama,
    LLMBackend::Gemini,
];

/// Walk the ladder from `requested` downward, returning the first backend that
/// is both reachable (Ollama `/api/tags` probe, 60s cache) and allowed (API
/// spend gate for Gemini). Also returns a note when a fallback happened, so
/// the commander report always says what actually ran.
async fn pick_healthy_backend(
    requested: LLMBackend,
) -> Result<(LLMBackend, Option<String>), String> {
    let start = BACKEND_LADDER
        .iter()
        .position(|b| *b == requested)
        .unwrap_or(0);
    let mut last_err = String::from("unknown");
    for b in &BACKEND_LADDER[start..] {
        match b {
            LLMBackend::LocalOllama | LLMBackend::LanOllama => match resolve_ollama_endpoint(b) {
                Ok(ep) if crate::io::ollama::ollama_reachable(&ep.url).await => {
                    let note = if *b != requested {
                        Some(format!("{:?} unreachable, fell back to {:?}", requested, b))
                    } else {
                        None
                    };
                    return Ok((b.clone(), note));
                }
                Ok(ep) => last_err = format!("{:?} at {} unreachable", b, ep.url),
                Err(e) => last_err = e,
            },
            _ => match crate::router::check_api_allowed(b) {
                Ok(()) => {
                    let note = if *b != requested {
                        Some(format!("{:?} unreachable, fell back to {:?}", requested, b))
                    } else {
                        None
                    };
                    return Ok((b.clone(), note));
                }
                Err(e) => last_err = e,
            },
        }
    }
    Err(format!("no backend usable down the ladder: {}", last_err))
}

/// Shared async dispatch core (task 216): the sync tool entry and the cron
/// job-pump both funnel through here. Returns the formatted commander report.
pub async fn dispatch_spawn(spec: &SpawnSpec) -> String {
    // Depth guard — enforced BEFORE any blocking, so a nested spawn can never
    // reach block_in_place (which would panic).
    if let Some(refused) = depth_refused() {
        return refused;
    }
    if let Some(stub) = test_stub_report(spec) {
        // The stub path still runs the registry lifecycle: integration tests
        // (and the UI) observe spawn→finish without needing a live LLM.
        let task_id = format!("subagent-{}", uuid::Uuid::new_v4().simple());
        let reg = crate::agents::subagent_registry::SubagentRecord::new(
            task_id.clone(),
            "test_stub",
            &spec.role_hint,
            &spec.task,
        );
        crate::agents::subagent_registry::register(reg);
        crate::agents::subagent_registry::finish(
            &task_id,
            crate::agents::subagent_registry::SubagentStatus::Done,
        );
        return stub;
    }
    let depth = SPAWN_DEPTH.try_with(|c| c.get()).unwrap_or(0);

    let requested = match prepare_backend(spec) {
        Ok(b) => b,
        Err(e) => return e,
    };

    // Task 217.3: health-aware fallback down the ladder.
    let (backend, fallback_note) = match pick_healthy_backend(requested).await {
        Ok(ok) => ok,
        Err(e) => return format!("Error: {}", e),
    };

    let task_id = format!("subagent-{}", uuid::Uuid::new_v4().simple());
    let backend_name = format!("{:?}", backend).to_lowercase();
    // Task 230: register the moment the id exists, so the UI can show a live
    // sub-agent instead of only learning about it after it finishes.
    crate::agents::subagent_registry::register(
        crate::agents::subagent_registry::SubagentRecord::new(
            task_id.clone(),
            backend_short_name(&backend),
            &spec.role_hint,
            &spec.task,
        ),
    );
    log::info!(
        "spawn_subagent {} -> backend={} role='{}' timeout={}s",
        task_id,
        backend_name,
        spec.role_hint,
        spec.timeout_secs
    );

    let timeout = Duration::from_secs(spec.timeout_secs);
    // Depth is tracked via task-local so it follows the future across threads.
    let run = SPAWN_DEPTH.scope(
        Cell::new(depth + 1),
        run_subagent(&task_id, &spec.task, &backend, &spec.role_hint),
    );
    // Task 217.3: surface any health fallback in the report so the commander
    // always knows what actually ran.
    let fb_line = fallback_note
        .map(|n| format!("note: {}\n", n))
        .unwrap_or_default();
    match tokio::time::timeout(timeout, run).await {
        Ok(Ok(final_text)) => {
            crate::agents::subagent_registry::finish(
                &task_id,
                crate::agents::subagent_registry::SubagentStatus::Done,
            );
            format!(
                "Sub-agent complete\ntask_id: {}\nbackend: {}\nrole: {}\n{}final:\n{}",
                task_id,
                backend_name,
                role_hint_or_any(&spec.role_hint),
                fb_line,
                final_text
            )
        }
        Ok(Err(err)) => {
            crate::agents::subagent_registry::finish(
                &task_id,
                crate::agents::subagent_registry::SubagentStatus::Failed,
            );
            // Task 233: revision loop — a failed worker may teach its role
            // file a lesson. Best-effort, .pending only, rate-limited; a
            // refused proposal must never break the failure report.
            if crate::persona::self_author_enabled() && !spec.role_hint.trim().is_empty() {
                let lesson = format!(
                    "task failed on backend {}: {}",
                    backend_name,
                    err.chars().take(200).collect::<String>()
                );
                if let Err(e) = crate::persona::propose_lesson(&spec.role_hint, &lesson) {
                    log::debug!("persona: lesson proposal skipped: {}", e);
                }
            }
            format!(
                "Sub-agent failed\ntask_id: {}\nbackend: {}\nerror: {}",
                task_id, backend_name, err
            )
        }
        Err(_) => {
            crate::agents::subagent_registry::finish(
                &task_id,
                crate::agents::subagent_registry::SubagentStatus::TimedOut,
            );
            // Task 233: same lesson trigger for timeouts.
            if crate::persona::self_author_enabled() && !spec.role_hint.trim().is_empty() {
                let lesson = format!(
                    "task timed out after {}s on backend {}; consider smaller tasks or larger timeouts",
                    spec.timeout_secs, backend_name
                );
                if let Err(e) = crate::persona::propose_lesson(&spec.role_hint, &lesson) {
                    log::debug!("persona: lesson proposal skipped: {}", e);
                }
            }
            format!(
                "Sub-agent timed out after {}s and was cancelled.\n\
             task_id: {}\nbackend: {}\n\
             The task may have been too big — retry with a smaller task or a larger timeout_secs.",
                spec.timeout_secs, task_id, backend_name
            )
        }
    }
}

/// Synchronous entry point for `tools::execute()`.
pub fn spawn_subagent(args: &Value) -> String {
    let spec = match parse_spawn_args(args) {
        Ok(s) => s,
        Err(e) => return format!("Error: {}", e),
    };

    // Depth guard before blocking (see dispatch_spawn for the authoritative check).
    if let Some(refused) = depth_refused() {
        return refused;
    }

    // Test seam: integration tests set this to avoid needing live LLMs.
    if let Some(stub) = test_stub_report(&spec) {
        return stub;
    }

    // Validate the backend choice before demanding a runtime, so arg errors
    // surface cleanly from any calling context.
    if let Err(e) = prepare_backend(&spec) {
        return e;
    }

    let handle = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => {
            return "Error: spawn_subagent needs a Tokio runtime (it is only valid \
                    from async chat loops, not from sync threads)."
                .to_string();
        }
    };

    // Block this worker thread (multi-thread runtime) until the sub-agent
    // finishes or the timeout fires.
    tokio::task::block_in_place(|| handle.block_on(dispatch_spawn(&spec)))
}

fn role_hint_or_any(role_hint: &str) -> &str {
    if role_hint.trim().is_empty() {
        "(any)"
    } else {
        role_hint
    }
}

// ── Sub-agent runner ──────────────────────────────────────────────────────────

/// Run the sub-agent's prompt to completion on the chosen backend.
async fn run_subagent(
    task_id: &str,
    task: &str,
    backend: &LLMBackend,
    role_hint: &str,
) -> Result<String, String> {
    // Task 233: pre-spawn draft — when the role has no persona file and
    // self-authoring is on, the commander drafts one now (lands as .pending,
    // never live). Best-effort: a failed draft must not block the spawn.
    if !role_hint.trim().is_empty()
        && let Err(e) = crate::persona::ensure_persona_file(Some(role_hint)).await
    {
        log::debug!(
            "persona: pre-spawn draft skipped for role '{}': {}",
            role_hint,
            e
        );
    }
    let composed = crate::persona::compose_subagent_prompt(task_id, task, role_hint);
    // Task 233: the role's frontmatter allowlist filters the toolset
    // (spawn_subagent is always excluded — the depth guard is absolute).
    // Task 282: the loop worker's filter FAILS CLOSED on a zero match
    // (unlike the general mechanism's full-toolset fallback) — a typo'd
    // persona must never silently grant the loop worker the full set.
    // Task 362: the loop drafter gets the SAME fail-closed filter and
    // the SAME 8 read-only tools — the allowlist is NOT widened for
    // drafting.
    let tools = if role_hint == "loop-worker" {
        match crate::self_improve::worker::loop_worker_tool_definitions() {
            Ok(t) => t,
            Err(e) => return Err(format!("loop-worker spawn refused: {e}")),
        }
    } else if role_hint == "loop-drafter" {
        match crate::self_improve::draft::loop_drafter_tool_definitions() {
            Ok(t) => t,
            Err(e) => return Err(format!("loop-drafter spawn refused: {e}")),
        }
    } else {
        crate::persona::subagent_tool_definitions(role_hint)
    };

    // Task 390: fail-closed guard + diagnostic logging. The Nov 26/27
    // nightly reports failed because the sub-agent claimed no task was
    // provided; composition is verified to carry the task, but if it
    // ever does not (a future refactor, a bad role file), refuse
    // loudly instead of spawning a confused sub-agent. Log the prompt
    // shape so a recurrence is diagnosable from the Helix log.
    if !composed.user.contains(task) {
        log::error!(
            "sub-agent spawn refused (task {}): composed user message lacks the task text (task_chars={}, user_chars={}, system_chars={})",
            task_id,
            task.chars().count(),
            composed.user.chars().count(),
            composed
                .system
                .as_ref()
                .map(|s| s.chars().count())
                .unwrap_or(0)
        );
        return Err(format!(
            "sub-agent spawn refused: composed prompt missing task text (task {task_id})"
        ));
    }
    let tool_count = tools.as_array().map(|a| a.len()).unwrap_or(0);
    log::info!(
        "sub-agent spawn (task {}): backend={:?} role={} task_chars={} user_chars={} system_chars={} tools={}",
        task_id,
        backend,
        role_hint,
        task.chars().count(),
        composed.user.chars().count(),
        composed
            .system
            .as_ref()
            .map(|s| s.chars().count())
            .unwrap_or(0),
        tool_count
    );

    match backend {
        LLMBackend::LocalOllama | LLMBackend::LanOllama => {
            let ep = resolve_ollama_endpoint(backend)?;
            let client =
                crate::io::ollama::llm_impl::OllamaLlmImpl::new(ep.url.clone(), ep.model.clone());
            // Task 233: sub-agents get a REAL system message when a persona
            // file exists; otherwise the legacy single user message
            // (byte-identical to pre-233).
            let messages: Vec<serde_json::Value> = match composed.system {
                Some(sys) => vec![
                    serde_json::json!({"role": "system", "content": sys}),
                    serde_json::json!({"role": "user", "content": composed.user}),
                ],
                None => vec![serde_json::json!({"role": "user", "content": composed.user})],
            };
            client
                .chat_with_tools(&messages, tools)
                .await
                .map_err(|e| format!("Ollama sub-agent on {:?} failed: {}", backend, e))
        }
        LLMBackend::Gemini => {
            crate::io::llm_gemini::call_gemini_direct(&composed.user, composed.system.as_deref())
                .await
                .map_err(|e| format!("Gemini sub-agent failed: {}", e))
        }
        LLMBackend::Grok => Err("Grok backend is not implemented yet.".to_string()),
        LLMBackend::Fallback => Err("Fallback is not a runnable backend.".to_string()),
    }
}

// Serialize env-mutating tests across modules: they share process-global
// env vars (HELIX_SUBAGENT_TEST_STUB, HELIX_SPEND_FILE, ...).
//
// M23 (task 411) — THE env-mutation rule: this is the ONE lock for all
// env-mutating tests in the crate. Any test that calls `std::env::set_var`
// or `std::env::remove_var` must hold this lock for its whole duration,
// because env mutation is process-global and `unsafe` under edition 2024:
// a parallel test reading any var races with it. Do NOT create a second
// module-private lock — `spend.rs` used to have its own `TEST_LOCK`, and
// it could not serialize against mutations in other modules. If a test
// only needs the lock through a helper, use one of the locked helpers
// (`with_clean_history`, `with_test_file`, `with_env`) — they all take
// this lock. Enforced by the `no_unlocked_env_mutation_in_tests` scan
// test in the `tests` module below.
#[cfg(test)]
static TEST_LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
#[cfg(test)]
pub fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    TEST_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_task_is_rejected() {
        let out = spawn_subagent(&json!({"task": "   "}));
        assert!(out.contains("requires a non-empty 'task'"), "got: {}", out);
    }

    #[test]
    fn unknown_backend_is_rejected() {
        // Task 294: locked — spawn_subagent checks HELIX_SUBAGENT_TEST_STUB
        // before backend validation, so a concurrent stub-setting test would
        // make this see stub output instead of "Unknown backend".
        let _tl = super::test_env_lock();
        let out = spawn_subagent(&json!({"task": "do a thing", "backend": "skynet"}));
        assert!(out.contains("Unknown backend"), "got: {}", out);
    }

    #[test]
    fn grok_backend_reports_not_implemented() {
        // Task 294: locked — same stub-env race as unknown_backend_is_rejected.
        let _tl = super::test_env_lock();
        let out = spawn_subagent(&json!({"task": "do a thing", "backend": "grok"}));
        assert!(out.contains("no client implementation"), "got: {}", out);
        assert!(out.to_lowercase().contains("grok"), "got: {}", out);
    }

    #[tokio::test]
    async fn nested_spawn_is_refused_at_depth_limit() {
        // Simulate being inside a sub-agent: depth already at max.
        let out = SPAWN_DEPTH
            .scope(Cell::new(MAX_SPAWN_DEPTH), async {
                spawn_subagent(&json!({"task": "go deeper"}))
            })
            .await;
        assert!(
            out.contains("may not spawn further"),
            "nested spawn must be refused, got: {}",
            out
        );
    }

    #[test]
    fn stubbed_spawn_returns_final_message() {
        let _tl = super::test_env_lock();
        // SAFETY: single-threaded test env manipulation; stub only affects this process.
        unsafe { std::env::set_var("HELIX_SUBAGENT_TEST_STUB", "1") };
        let out = spawn_subagent(&json!({
            "task": "plan out a bot app in rust",
            "backend": "local_ollama",
            "role_hint": "planner"
        }));
        unsafe { std::env::remove_var("HELIX_SUBAGENT_TEST_STUB") };
        assert!(out.contains("task_id: subagent-test-stub"), "got: {}", out);
        assert!(out.contains("backend: test_stub"), "got: {}", out);
        assert!(out.contains("plan out a bot app in rust"), "got: {}", out);
        assert!(out.contains("final:"), "got: {}", out);
    }

    #[test]
    fn schema_registered_and_execute_dispatched() {
        // Task 294: locked — calls tool_definitions() and excluding().
        let _tl = super::test_env_lock();
        // Schema must be advertised to the models...
        let defs = crate::tools::tool_definitions();
        let arr = defs.as_array().expect("tool_definitions must be an array");
        let schema = arr
            .iter()
            .find(|d| d["function"]["name"] == "spawn_subagent")
            .expect("spawn_subagent schema missing from tool_definitions()");
        let required = schema["function"]["parameters"]["required"]
            .as_array()
            .expect("schema must list required params");
        assert!(required.iter().any(|r| r == "task"));
        // ...and the execute() dispatch arm must reach our implementation.
        let out = crate::tools::execute("spawn_subagent", &json!({}));
        assert!(out.contains("non-empty 'task'"), "got: {}", out);
        // The sub-agent's own toolset must NOT contain spawn_subagent (depth guard).
        let filtered = crate::tools::tool_definitions_excluding(&["spawn_subagent"]);
        let farr = filtered.as_array().unwrap();
        assert!(
            !farr
                .iter()
                .any(|d| d["function"]["name"] == "spawn_subagent")
        );
    }

    #[test]
    fn commander_prompt_is_short_and_mentions_tool() {
        assert!(COMMANDER_SYSTEM_PROMPT.contains("spawn_subagent"));
        assert!(
            COMMANDER_SYSTEM_PROMPT.len() < 1200,
            "keep it tight for small local models ({} chars)",
            COMMANDER_SYSTEM_PROMPT.len()
        );
    }

    #[test]
    fn commander_system_message_respects_gate() {
        let _tl = super::test_env_lock();
        use std::sync::Mutex as StdMutex;
        static G: OnceLock<StdMutex<()>> = OnceLock::new();
        let _g = G
            .get_or_init(|| StdMutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // SAFETY: serialized by G; default env (unset) means enabled.
        unsafe { std::env::remove_var("HELIX_COMMANDER_ENABLED") };
        // NOTE: if some other test already called set_commander_enabled(), the
        // override wins — no test in this binary does.
        let msg = commander_system_message();
        assert!(msg.is_some(), "commander mode defaults to enabled");
        let binding = msg.unwrap();
        let content = binding["content"].as_str().unwrap_or("");
        assert!(content.contains("spawn_subagent"));

        unsafe { std::env::set_var("HELIX_COMMANDER_ENABLED", "0") };
        assert!(
            commander_system_message().is_none(),
            "HELIX_COMMANDER_ENABLED=0 must disable it"
        );
        unsafe { std::env::remove_var("HELIX_COMMANDER_ENABLED") };
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn dispatch_spawn_uses_stub_and_shared_helper() {
        let _tl = super::test_env_lock();
        // Task 216: the cron job-pump calls dispatch_spawn directly — verify the
        // shared helper works end-to-end (stubbed, no live LLM needed).
        // SAFETY: single-threaded test env manipulation.
        unsafe { std::env::set_var("HELIX_SUBAGENT_TEST_STUB", "1") };
        let spec = SpawnSpec {
            task: "check the inbox".to_string(),
            backend: "auto".to_string(),
            role_hint: "quartermaster".to_string(),
            timeout_secs: 30,
        };
        let out = dispatch_spawn(&spec).await;
        unsafe { std::env::remove_var("HELIX_SUBAGENT_TEST_STUB") };
        // NOTE: the stub lives in the sync wrapper only; dispatch_spawn runs the
        // real path. With no Ollama running, auto-resolution may fail — but the
        // depth guard, backend parsing, and spend gate must all pass first.
        // For a deterministic assertion we check it either completed via stub-like
        // refusal-free flow or reported a backend error (not a guardrail trip).
        assert!(
            !out.contains("may not spawn further"),
            "depth guard must not trip at depth 0: {}",
            out
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn fallback_skips_unreachable_ollama() {
        // Task 217.3: dead Ollama boxes fall through to the next ladder rung.
        let _tl = super::test_env_lock();
        crate::io::ollama::ensure_crypto_provider();
        // SAFETY: single-threaded test env manipulation (see TEST_LOCK).
        unsafe {
            std::env::set_var("HELIX_SUBAGENT_LOCAL_URL", "http://127.0.0.1:9");
            std::env::set_var("HELIX_SUBAGENT_LAN_URL", "http://127.0.0.1:9");
            std::env::set_var(
                "HELIX_SPEND_FILE",
                format!("/tmp/helix_spend_fallback_{}.json", std::process::id()),
            );
        }
        crate::router::spend::reset_for_tests(); // clean spend -> Gemini allowed
        let (backend, note) = pick_healthy_backend(LLMBackend::LocalOllama)
            .await
            .expect("ladder should land on Gemini");
        assert_eq!(backend, LLMBackend::Gemini);
        assert!(note.is_some(), "fallback must be reported");
        unsafe {
            std::env::remove_var("HELIX_SUBAGENT_LOCAL_URL");
            std::env::remove_var("HELIX_SUBAGENT_LAN_URL");
            std::env::remove_var("HELIX_SPEND_FILE");
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn fallback_fails_cleanly_when_everything_is_down() {
        // Task 217.3: dead Ollama + tripped spend cap -> honest error, no panic.
        let _tl = super::test_env_lock();
        crate::io::ollama::ensure_crypto_provider();
        unsafe {
            std::env::set_var("HELIX_SUBAGENT_LOCAL_URL", "http://127.0.0.1:9");
            std::env::set_var("HELIX_SUBAGENT_LAN_URL", "http://127.0.0.1:9");
            std::env::set_var(
                "HELIX_SPEND_FILE",
                format!("/tmp/helix_spend_fallback2_{}.json", std::process::id()),
            );
        }
        crate::router::spend::reset_for_tests();
        crate::router::spend::set_daily_api_cap_usd(0.01);
        // Task 236: cap blocks only in block mode (default is warn+continue).
        crate::router::spend::set_cap_block_on_exceed(true);
        crate::router::spend::note_api_call(&LLMBackend::Gemini); // $0.02 > $0.01
        let err = pick_healthy_backend(LLMBackend::LocalOllama)
            .await
            .expect_err("nothing usable: should error");
        assert!(err.contains("no backend usable"), "got: {}", err);
        crate::router::spend::reset_for_tests();
        unsafe {
            std::env::remove_var("HELIX_SUBAGENT_LOCAL_URL");
            std::env::remove_var("HELIX_SUBAGENT_LAN_URL");
            std::env::remove_var("HELIX_SPEND_FILE");
        }
    }

    #[test]
    fn parse_backend_accepts_aliases() {
        assert_eq!(parse_backend("auto").unwrap_err(), "__auto__");
        assert_eq!(parse_backend("").unwrap_err(), "__auto__");
        assert_eq!(
            parse_backend("local_ollama").unwrap(),
            LLMBackend::LocalOllama
        );
        assert_eq!(parse_backend("lan").unwrap(), LLMBackend::LanOllama);
        assert_eq!(parse_backend("GEMINI").unwrap(), LLMBackend::Gemini);
        assert!(parse_backend("skynet").is_err());
    }

    #[test]
    fn gemini_sits_last_on_health_ladder() {
        // Task 258: the support drafter pins backend="gemini". dispatch_spawn
        // walks BACKEND_LADDER *downward* from the requested backend, so an
        // explicit Gemini request either runs on Gemini or fails loudly — it
        // can never silently fall back to a local model. Pin that property:
        // Gemini must stay last on the ladder.
        assert_eq!(
            BACKEND_LADDER.last(),
            Some(&LLMBackend::Gemini),
            "Gemini must remain last on the health ladder (task 258 invariant)"
        );
        assert!(!BACKEND_LADDER.contains(&LLMBackend::Grok));
    }

    #[test]
    fn endpoint_env_fallback_works() {
        let _tl = super::test_env_lock();
        // SAFETY: single-threaded test env manipulation.
        unsafe {
            std::env::set_var("HELIX_SUBAGENT_LOCAL_URL", "http://test:11434");
            std::env::set_var("HELIX_SUBAGENT_LOCAL_MODEL", "test-model");
        }
        // Only used when the startup registry is empty (it is in tests).
        let ep = resolve_ollama_endpoint(&LLMBackend::LocalOllama).unwrap();
        unsafe {
            std::env::remove_var("HELIX_SUBAGENT_LOCAL_URL");
            std::env::remove_var("HELIX_SUBAGENT_LOCAL_MODEL");
        }
        assert_eq!(ep.url, "http://test:11434");
        assert_eq!(ep.model, "test-model");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn stubbed_dispatch_emits_spawned_then_finished_bus_events() {
        let _tl = super::test_env_lock();
        // GIVEN a global bus with a web_interface subscriber and the test stub armed:
        let bus = std::sync::Arc::new(crate::bus::Bus::new());
        let mut rx = bus.subscribe("web_interface");
        crate::bus::set_global_bus(bus.clone());
        crate::agents::subagent_registry::reset_for_tests();
        // SAFETY: single-threaded test env manipulation; stub only affects this process.
        unsafe { std::env::set_var("HELIX_SUBAGENT_TEST_STUB", "1") };

        // WHEN dispatch_spawn runs through the stub path,
        let spec = SpawnSpec {
            task: "test the event order".to_string(),
            backend: "local_ollama".to_string(),
            role_hint: "tester".to_string(),
            timeout_secs: 30,
        };
        let out = dispatch_spawn(&spec).await;
        unsafe { std::env::remove_var("HELIX_SUBAGENT_TEST_STUB") };

        // THEN the report is the stub,
        assert!(out.contains("subagent-test-stub"), "got: {}", out);

        // AND spawned+finished subagent_events arrived in order on the bus.
        // (H6/task 383: the bus is now tokio::sync::mpsc, so recv() is
        // awaited with a timeout instead of the old blocking recv_timeout.
        // Publish hands off to the bus router thread, so delivery is not
        // synchronous with the publish call.)
        let mut events = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while events.len() < 2 && std::time::Instant::now() < deadline {
            if let Ok(Some(msg)) =
                tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await
            {
                let v: serde_json::Value = serde_json::from_str(&msg.data).unwrap_or_default();
                if v["type"] == "subagent_event" {
                    events.push(v["event"].as_str().unwrap_or("").to_string());
                }
            }
        }
        assert_eq!(
            events,
            vec!["spawned".to_string(), "finished".to_string()],
            "expected spawned->finished in order, got: {:?}",
            events
        );

        // AND the registry shows the finished entry (prune keeps it):
        let snap = crate::agents::subagent_registry::snapshot();
        assert!(
            snap.iter().any(|r| r.backend == "test_stub"
                && r.status == crate::agents::subagent_registry::SubagentStatus::Done),
            "stub run should leave a Done record"
        );
        crate::agents::subagent_registry::reset_for_tests();
    }

    // ── M23: env-mutation lock enforcement ──────────────────────────────

    /// M23 (task 411): no env mutation in test code without the shared lock.
    ///
    /// Scans every `src/**/*.rs` file for `set_var(`/`remove_var(` inside
    /// test code (`mod tests` blocks, a whole file named `tests.rs`, or a
    /// top-level `#[test]` / `#[tokio::test]` fn), and asserts the enclosing
    /// test fn holds `test_env_lock` — directly, via a locked helper
    /// (`with_clean_history`, `with_test_file`, `with_env`), or by receiving
    /// the guard as a `&MutexGuard` parameter. A new env-mutating test that
    /// forgets the lock fails here instead of flaking under parallel test
    /// threads.
    #[test]
    fn no_unlocked_env_mutation_in_tests() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        collect_rs_files(&root, &mut files);
        assert!(!files.is_empty(), "no .rs files found under {:?}", root);
        let mut violations: Vec<String> = Vec::new();
        for file in &files {
            let text = std::fs::read_to_string(file)
                .unwrap_or_else(|e| panic!("cannot read {:?}: {}", file, e));
            scan_file_for_unlocked_env(file, &text, &mut violations);
        }
        assert!(
            violations.is_empty(),
            "env-mutating tests without test_env_lock (M23):\n{}",
            violations.join("\n")
        );
    }

    /// M23 follow-up (Reviewer should-fix): the scanner was blind to
    /// top-level `#[test]` fns — it only looked inside `mod tests` blocks
    /// and `tests.rs` files, so the (now deleted)
    /// `src/router/config_tests.rs` top-level tests with unlocked
    /// `set_var`/`remove_var` went unflagged. Prove a top-level `#[test]`
    /// fn with an unlocked mutation is flagged, while locked test code
    /// and production code still pass.
    #[test]
    fn scan_flags_top_level_test_fn_env_mutation() {
        let file = std::path::Path::new("src/fake_scan_target.rs");

        // Unlocked set_var / remove_var in a top-level #[test] fn: flagged.
        let unlocked = "use std::env;\n\n#[test]\nfn top_level_test_mutates_env() {\n    env::set_var(\"FOO\", \"1\");\n    env::remove_var(\"FOO\");\n}\n";
        let mut v = Vec::new();
        scan_file_for_unlocked_env(file, unlocked, &mut v);
        assert_eq!(
            v.len(),
            2,
            "expected both top-level env mutations flagged, got: {:?}",
            v
        );
        assert!(v[0].contains("top_level_test_mutates_env"));

        // Same shape with #[tokio::test]: flagged too.
        let unlocked_tokio = "#[tokio::test]\nasync fn top_level_tokio_test_mutates_env() {\n    std::env::set_var(\"FOO\", \"1\");\n}\n";
        let mut v = Vec::new();
        scan_file_for_unlocked_env(file, unlocked_tokio, &mut v);
        assert_eq!(
            v.len(),
            1,
            "expected top-level tokio set_var flagged, got: {:?}",
            v
        );

        // Locked top-level #[test] fn (direct lock): clean.
        let locked = "#[test]\nfn top_level_test_locked() {\n    let _g = test_env_lock();\n    std::env::set_var(\"FOO\", \"1\");\n}\n";
        let mut v = Vec::new();
        scan_file_for_unlocked_env(file, locked, &mut v);
        assert!(
            v.is_empty(),
            "locked top-level test must pass, got: {:?}",
            v
        );

        // Locked via the with_env helper: clean.
        let locked_helper = "#[test]\nfn top_level_test_with_env() {\n    with_env(\"FOO\", \"1\", || {\n        std::env::set_var(\"FOO\", \"1\");\n    });\n}\n";
        let mut v = Vec::new();
        scan_file_for_unlocked_env(file, locked_helper, &mut v);
        assert!(
            v.is_empty(),
            "with_env top-level test must pass, got: {:?}",
            v
        );

        // Production fn (no #[test]) with set_var: clean — the scanner
        // only polices test code.
        let prod = "use std::env;\n\npub fn configure() {\n    env::set_var(\"FOO\", \"1\");\n}\n";
        let mut v = Vec::new();
        scan_file_for_unlocked_env(file, prod, &mut v);
        assert!(v.is_empty(), "production set_var must pass, got: {:?}", v);

        // A #[test] fn inside `mod tests` without the lock is still
        // flagged (existing behaviour, regression guard).
        let mod_tests = "#[cfg(test)]\nmod tests {\n    use super::*;\n    #[test]\n    fn in_mod_tests_unlocked() {\n        std::env::set_var(\"FOO\", \"1\");\n    }\n}\n";
        let mut v = Vec::new();
        scan_file_for_unlocked_env(file, mod_tests, &mut v);
        assert_eq!(
            v.len(),
            1,
            "expected in-mod-tests set_var still flagged, got: {:?}",
            v
        );
    }

    fn collect_rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if !name.starts_with('.') && name != "target" {
                    collect_rs_files(&path, out);
                }
            } else if name.ends_with(".rs") {
                out.push(path);
            }
        }
    }

    /// Strip string literals, char literals and comments (replaced by
    /// spaces, newlines kept) so brace counting and keyword matching see
    /// only real code. Line numbers are preserved.
    fn strip_strings_and_comments(src: &str) -> String {
        #[derive(PartialEq)]
        enum St {
            Normal,
            Str,
            Char,
            LineComment,
            BlockComment,
        }
        let mut out = String::with_capacity(src.len());
        let mut st = St::Normal;
        let mut chars = src.chars().peekable();
        while let Some(c) = chars.next() {
            match st {
                St::Normal => match c {
                    '"' => {
                        st = St::Str;
                        out.push(' ');
                    }
                    '/' => match chars.peek() {
                        Some('/') => {
                            chars.next();
                            st = St::LineComment;
                            out.push(' ');
                            out.push(' ');
                        }
                        Some('*') => {
                            chars.next();
                            st = St::BlockComment;
                            out.push(' ');
                            out.push(' ');
                        }
                        _ => out.push(c),
                    },
                    '\'' => {
                        // Char literal only if it looks like 'x' / '\n' /
                        // '\'' — otherwise it is a lifetime, left alone.
                        let mut probe = chars.clone();
                        let ok = match probe.next() {
                            Some('\\') => {
                                let _ = probe.next();
                                probe.next() == Some('\'')
                            }
                            Some(_) => probe.next() == Some('\''),
                            None => false,
                        };
                        if ok {
                            st = St::Char;
                            out.push(' ');
                        } else {
                            out.push(c);
                        }
                    }
                    _ => out.push(c),
                },
                St::Str => {
                    if c == '\\' {
                        chars.next();
                        out.push(' ');
                        out.push(' ');
                    } else if c == '"' {
                        st = St::Normal;
                        out.push(' ');
                    } else {
                        out.push(if c == '\n' { '\n' } else { ' ' });
                    }
                }
                St::Char => {
                    if c == '\\' {
                        chars.next();
                        out.push(' ');
                        out.push(' ');
                    } else if c == '\'' {
                        st = St::Normal;
                        out.push(' ');
                    } else {
                        out.push(if c == '\n' { '\n' } else { ' ' });
                    }
                }
                St::LineComment => {
                    if c == '\n' {
                        st = St::Normal;
                        out.push('\n');
                    } else {
                        out.push(' ');
                    }
                }
                St::BlockComment => {
                    if c == '*' && chars.peek() == Some(&'/') {
                        chars.next();
                        st = St::Normal;
                        out.push(' ');
                        out.push(' ');
                    } else {
                        out.push(if c == '\n' { '\n' } else { ' ' });
                    }
                }
            }
        }
        out
    }

    /// The `fn` name on a definition line, if any (`fn`, `async fn`,
    /// `pub fn`, `pub(crate) fn`).
    fn fn_name_on_line(line: &str) -> Option<String> {
        let mut t = line.trim_start();
        // Strip qualifiers in any order: pub, pub(...), async, const, unsafe.
        loop {
            if let Some(rest) = t.strip_prefix("pub") {
                t = rest.trim_start();
                if t.starts_with('(') {
                    // Skip the (...) group.
                    let mut depth = 0;
                    let mut cut = None;
                    for (i, c) in t.char_indices() {
                        match c {
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    cut = Some(i + 1);
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    t = t[cut?..].trim_start();
                }
                continue;
            }
            if let Some(rest) = t.strip_prefix("async") {
                t = rest.trim_start();
                continue;
            }
            if let Some(rest) = t.strip_prefix("const") {
                t = rest.trim_start();
                continue;
            }
            if let Some(rest) = t.strip_prefix("unsafe") {
                t = rest.trim_start();
                continue;
            }
            break;
        }
        let rest = t.strip_prefix("fn ")?;
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() { None } else { Some(name) }
    }

    /// Enclosing fns of `idx` (innermost first): walk upward tracking the
    /// brace balance; a `fn` line encloses `idx` when the braces between
    /// it and `idx` are net-open (its body has not closed yet).
    fn enclosing_fns(lines: &[&str], idx: usize) -> Vec<(String, usize)> {
        let mut out = Vec::new();
        let mut net: i32 = 0;
        let mut j = idx as i32;
        while j >= 0 {
            let line = lines[j as usize];
            let opens = line.matches('{').count() as i32;
            let closes = line.matches('}').count() as i32;
            net += closes - opens;
            if let Some(name) = fn_name_on_line(line)
                && net < 0
            {
                out.push((name, j as usize));
            }
            j -= 1;
        }
        out
    }

    /// Top-level test fns: (start line, end line, inclusive) for fns
    /// annotated `#[test]` / `#[tokio::test]` at the top level of a file
    /// (brace depth 0). These are test code even though they sit outside
    /// any `mod tests` block — the deleted `src/router/config_tests.rs`
    /// had top-level `#[test]` fns with unlocked env mutations that this
    /// scanner was blind to. Other attributes between the test attribute
    /// and the fn (e.g. `#[ignore]`) do not clear the marker.
    fn top_level_test_fn_spans(lines: &[&str], depth_before: &[i32]) -> Vec<(usize, usize)> {
        let mut spans = Vec::new();
        let mut pending_test_attr = false;
        for (idx, line) in lines.iter().enumerate() {
            let t = line.trim();
            if t.starts_with("#[test]") || t.starts_with("#[tokio::test]") {
                pending_test_attr = true;
                continue;
            }
            if t.starts_with("#[") || t.is_empty() {
                continue;
            }
            if pending_test_attr && depth_before[idx] == 0 && fn_name_on_line(line).is_some() {
                // Find the end of the fn body: the first line after the
                // opening brace that brings the depth back to the fn's
                // start depth. A `;` before any `{` is a body-less
                // declaration — no span.
                let start_depth = depth_before[idx];
                let mut j = idx;
                let mut opened = false;
                let mut end = None;
                while j < lines.len() {
                    let opens = lines[j].matches('{').count() as i32;
                    let closes = lines[j].matches('}').count() as i32;
                    if !opened && opens == 0 && lines[j].contains(';') {
                        break;
                    }
                    if opens > 0 {
                        opened = true;
                    }
                    if opened && depth_before[j] + opens - closes <= start_depth {
                        end = Some(j);
                        break;
                    }
                    j += 1;
                }
                if let Some(end) = end {
                    spans.push((idx, end));
                }
            }
            pending_test_attr = false;
        }
        spans
    }

    fn scan_file_for_unlocked_env(
        file: &std::path::Path,
        text: &str,
        violations: &mut Vec<String>,
    ) {
        let cleaned = strip_strings_and_comments(text);
        let lines: Vec<&str> = cleaned.lines().collect();
        let whole_file_is_tests = file.file_name().map(|n| n == "tests.rs").unwrap_or(false);
        // Brace depth before each line.
        let mut depth_before: Vec<i32> = Vec::with_capacity(lines.len());
        let mut depth: i32 = 0;
        for line in &lines {
            depth_before.push(depth);
            depth += line.matches('{').count() as i32;
            depth -= line.matches('}').count() as i32;
        }
        // Top-level `#[test]` / `#[tokio::test]` fns (outside any mod tests).
        let top_level_tests = top_level_test_fn_spans(&lines, &depth_before);
        // Active `mod tests {` scope: (entry depth, start line).
        let mut tests_scope: Option<(i32, usize)> = None;
        for (idx, line) in lines.iter().enumerate() {
            if let Some((entry, _)) = tests_scope
                && !whole_file_is_tests
                && depth_before[idx] <= entry
            {
                tests_scope = None;
            }
            if line.contains("mod tests") && line.contains('{') {
                tests_scope = Some((depth_before[idx], idx));
            }
            let in_tests = whole_file_is_tests
                || tests_scope.is_some()
                || top_level_tests.iter().any(|(s, e)| idx >= *s && idx <= *e);
            if !in_tests {
                continue;
            }
            if !(line.contains("set_var(") || line.contains("remove_var(")) {
                continue;
            }
            let chain = enclosing_fns(&lines, idx);
            let locked = chain.iter().any(|(_, start)| {
                let segment = lines[*start..=idx].join("\n");
                segment.contains("test_env_lock")
                    || segment.contains("with_clean_history")
                    || segment.contains("with_test_file")
                    || segment.contains("with_env(")
                    || lines[*start].contains("MutexGuard")
            });
            if !locked {
                let fname = chain
                    .first()
                    .map(|(n, _)| n.clone())
                    .unwrap_or_else(|| "<top level>".to_string());
                violations.push(format!(
                    "{}:{} in fn `{}`: {}",
                    file.display(),
                    idx + 1,
                    fname,
                    line.trim()
                ));
            }
        }
    }
}
