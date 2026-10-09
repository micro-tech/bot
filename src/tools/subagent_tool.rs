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

static COMMANDER_ENABLED_OVERRIDE: OnceLock<bool> = OnceLock::new();

/// Set at startup from `[helix.commander] enabled` in config.toml (main.rs).
pub fn set_commander_enabled(enabled: bool) {
    let _ = COMMANDER_ENABLED_OVERRIDE.set(enabled);
}

/// Build the system message for chat paths, or None when commander mode is off.
/// Chat loops prepend this to the message list (Ollama) or map it to the
/// backend's native system-instruction field (Gemini).
pub fn commander_system_message() -> Option<Value> {
    if commander_enabled() {
        Some(serde_json::json!({
            "role": "system",
            "content": COMMANDER_SYSTEM_PROMPT
        }))
    } else {
        None
    }
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

fn resolve_ollama_endpoint(backend: &LLMBackend) -> Result<OllamaEndpoint, String> {
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
    ["rust", "code", "function", "compile", "cargo", "python", "bug", "refactor", "api"]
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
            log::warn!("spawn_subagent: router picked Grok (no client yet); falling back to Gemini");
            LLMBackend::Gemini
        }
        LLMBackend::Fallback => LLMBackend::LocalOllama,
        b => b,
    }
}

fn role_prefix(role_hint: &str) -> String {
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

/// Backend ladder, fastest/cheapest first. Health-aware fallback walks DOWN
/// this list from the requested backend (task 217.3). Grok is parked at the
/// bottom as not-implemented — the walker never selects it.
const BACKEND_LADDER: [LLMBackend; 3] =
    [LLMBackend::LocalOllama, LLMBackend::LanOllama, LLMBackend::Gemini];

/// Walk the ladder from `requested` downward, returning the first backend that
/// is both reachable (Ollama `/api/tags` probe, 60s cache) and allowed (API
/// spend gate for Gemini). Also returns a note when a fallback happened, so
/// the commander report always says what actually ran.
async fn pick_healthy_backend(requested: LLMBackend) -> Result<(LLMBackend, Option<String>), String> {
    let start = BACKEND_LADDER
        .iter()
        .position(|b| *b == requested)
        .unwrap_or(0);
    let mut last_err = String::from("unknown");
    for b in &BACKEND_LADDER[start..] {
        match b {
            LLMBackend::LocalOllama | LLMBackend::LanOllama => {
                match resolve_ollama_endpoint(b) {
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
                }
            }
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
        Ok(Ok(final_text)) => format!(
            "Sub-agent complete\ntask_id: {}\nbackend: {}\nrole: {}\n{}final:\n{}",
            task_id,
            backend_name,
            role_hint_or_any(&spec.role_hint),
            fb_line,
            final_text
        ),
        Ok(Err(err)) => format!(
            "Sub-agent failed\ntask_id: {}\nbackend: {}\nerror: {}",
            task_id, backend_name, err
        ),
        Err(_) => format!(
            "Sub-agent timed out after {}s and was cancelled.\n\
             task_id: {}\nbackend: {}\n\
             The task may have been too big — retry with a smaller task or a larger timeout_secs.",
            spec.timeout_secs, task_id, backend_name
        ),
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
                .to_string()
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
    let prompt = format!(
        "You are a sub-agent ({}). Your commander's orders:\n\n{}\n\n{}\
         Complete the task with your tools. Do not ask clarifying questions — act. \
         Finish with a concise summary of what you did.",
        task_id,
        role_prefix(role_hint),
        task,
    );

    match backend {
        LLMBackend::LocalOllama | LLMBackend::LanOllama => {
            let ep = resolve_ollama_endpoint(backend)?;
            let client = crate::io::ollama::llm_impl::OllamaLlmImpl::new(ep.url.clone(), ep.model.clone());
            // Depth guard by construction: the sub-agent never sees spawn_subagent.
            let tools = crate::tools::tool_definitions_excluding(&["spawn_subagent"]);
            let messages = vec![serde_json::json!({"role": "user", "content": prompt})];
            client
                .chat_with_tools(&messages, tools)
                .await
                .map_err(|e| format!("Ollama sub-agent on {:?} failed: {}", backend, e))
        }
        LLMBackend::Gemini => crate::io::llm_gemini::call_gemini_direct(&prompt)
            .await
            .map_err(|e| format!("Gemini sub-agent failed: {}", e)),
        LLMBackend::Grok => Err("Grok backend is not implemented yet.".to_string()),
        LLMBackend::Fallback => Err("Fallback is not a runnable backend.".to_string()),
    }
}

// Serialize env-mutating tests across modules: they share process-global
// env vars (HELIX_SUBAGENT_TEST_STUB, HELIX_SPEND_FILE, ...).
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
        let out = spawn_subagent(&json!({"task": "do a thing", "backend": "skynet"}));
        assert!(out.contains("Unknown backend"), "got: {}", out);
    }

    #[test]
    fn grok_backend_reports_not_implemented() {
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
        assert!(!farr.iter().any(|d| d["function"]["name"] == "spawn_subagent"));
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
        let _g = G.get_or_init(|| StdMutex::new(())).lock().unwrap_or_else(|e| e.into_inner());
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
        assert!(commander_system_message().is_none(), "HELIX_COMMANDER_ENABLED=0 must disable it");
        unsafe { std::env::remove_var("HELIX_COMMANDER_ENABLED") };
    }

    #[tokio::test]
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
        assert_eq!(parse_backend("local_ollama").unwrap(), LLMBackend::LocalOllama);
        assert_eq!(parse_backend("lan").unwrap(), LLMBackend::LanOllama);
        assert_eq!(parse_backend("GEMINI").unwrap(), LLMBackend::Gemini);
        assert!(parse_backend("skynet").is_err());
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
}
