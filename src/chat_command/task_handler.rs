//! LLM task handler (task 310, spec §6).
//!
//! Takes a validated `how: llm` envelope and runs it as an agent job
//! on the named backend, async, scope-enforced, with a per-backend
//! FIFO queue. It spawns via Helix's EXISTING agent-run machinery
//! (`tools::subagent_tool::dispatch_spawn` — the same path the
//! commander pattern uses); no new LLM client is invented here.
//!
//! Flow:
//! 1. `submit_llm_task` (called from the tick, sync): ensures the
//!    scope-role persona file exists under `<runtime_dir>/agents/`,
//!    then either runs the task immediately (backend idle) or queues
//!    it (backend busy). It ALWAYS posts exactly one reply — ACK,
//!    queued/busy, or rejection — and returns; it never blocks the
//!    tick on an agent run.
//! 2. The runner (`run_llm_task`, detached tokio task): verifies the
//!    spawned toolset against the scope BEFORE calling
//!    `dispatch_spawn` (fail closed — see `verify_toolset`), awaits
//!    the run (whose own timeout enforces the budget), posts the
//!    RESULT / failure / timeout report citing the task's hub id,
//!    then advances that backend's queue (next task gets a "started"
//!    notice + a runner).
//!
//! Scope enforcement (the load-bearing part):
//! - A scope maps to a persona role file (`chat-task-read`,
//!   `chat-task-write-<domain>`) whose frontmatter `tools:` list is
//!   the allowlist. `dispatch_spawn` passes `role_hint` through to
//!   `persona::subagent_tool_definitions`, which filters the full
//!   toolset to that list.
//! - Persona files alone are NOT fail-closed (a missing file or an
//!   all-unknown allowlist yields the FULL toolset), so the runner
//!   verifies the filtered set before spawning: non-empty, a subset
//!   of the scope's allowlist, and — for a write scope — containing
//!   at least one write tool for that domain. Any failure → the task
//!   is refused with a report, never spawned.
//! - Read tasks therefore cannot write: the write tools are absent
//!   from their toolset, so an attempted write fails closed (the
//!   tool is unavailable) and the report states no writes were made.
//! - Gmail write tool names (`mcp__google__gmail_modify` etc.) are
//!   BEST-EFFORT — they are not confirmed against the live google-mcp
//!   tool list anywhere in this repo. If the MCP server advertises
//!   different names, the filtered set contains no Gmail write tool
//!   and `verify_toolset` refuses the spawn (safe, but the task does
//!   not run). Confirming the real names against the live tool list
//!   is a deploy-time follow-up, not a silent grant.
//!
//! Known v1 limitations (documented, not hidden):
//! - Queue + running state are PERSISTED (M21): the queue file
//!   `<runtime_dir>/chat_command/queue.json` is saved on every
//!   mutation and reloaded once at startup; a task that was running
//!   when Helix died comes back queued at the front of its backend
//!   (never silently done, never re-run without a report — the
//!   resume re-spawns it and it reports normally). A corrupt or
//!   missing queue file starts empty with a warning, never a
//!   startup failure.
//! - Write-task reports state the scope limit but do NOT enumerate
//!   the writes made — `dispatch_spawn`'s report carries no tool-call
//!   trace. Enumerating writes needs tracing from the spawn
//!   machinery (follow-up).
//! - Timeout reports carry no partial results — `dispatch_spawn`'s
//!   timeout cancels the run and returns no partial text. The report
//!   says timed out, honestly, without invented partials.
//!
//! Persona-dir note: `main.rs` sets the persona dir to the runtime
//! dir at startup, and the tick's `runtime_dir` IS that same dir
//! (parent of the loaded config). Role files therefore go to
//! `<runtime_dir>/agents/<role>.md`, which is exactly where
//! `persona_path` reads them in production — and inside the test
//! tmpdir in tick tests (no worktree pollution).

use super::envelope::{LlmBackend, Scope};
use super::reply;
use super::{ChatCommandConfig, ResolvedChatTools};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Default LLM time budget in seconds (spec §3: llm 300).
pub const DEFAULT_LLM_TIME_SECS: u64 = 300;

// ── Scope → tool allowlists ─────────────────────────────────────────

/// Static read-only tools (from `tools::execute` dispatch). Everything
/// else static is a write and is excluded from read tasks:
/// `write_note`, `send_email`, `set_belief`, `bayes_reset`,
/// `run_shell` (a shell can write), `generate_mermaid_project_map`
/// (writes files). `spawn_subagent` is always excluded by the base
/// filter and is never allowlisted.
pub const STATIC_READ_TOOLS: &[&str] = &[
    "read_log",
    "read_note",
    "list_notes",
    "check_inbox",
    "read_email",
    "system_status",
    "list_tools",
    "get_beliefs",
    "repo_glob",
    "repo_read",
    "repo_grep",
    "scan_directory",
];

/// Gmail read tools (confirmed names in-repo: `mcp_client` manager +
/// security allowlist). Included best-effort: if google-mcp is not
/// enabled they are simply not advertised and the filter drops them.
pub const GMAIL_READ_TOOLS: &[&str] = &["mcp__google__gmail_search", "mcp__google__gmail_read"];

/// Wire read tools (mcp_wire, packet capture / pcap analysis /
/// interfaces). Confirmed names in-repo: `mcp__wire__interfaces` is
/// called directly by `tools::file_tools` (`create_network_map`,
/// `own_lan_ips`), and the upstream tool names (`capture`,
/// `analyze_pcap`, `interfaces`) are the task-332 `how: "mcp"`
/// allowlist in `mcp_handler::read_tools_for`. All three are
/// read-only: they observe traffic or files and modify no state.
/// Included best-effort, like the Gmail tools: if the wire server is
/// not configured they are simply not advertised and the filter drops
/// them, so a read task still verifies and runs (without wire
/// access). This is what lets the chat-command loop answer network
/// questions at all — before this, a loop task had no network tool
/// and could only refuse (task 375: the nightly network report).
pub const WIRE_READ_TOOLS: &[&str] = &[
    "mcp__wire__capture",
    "mcp__wire__interfaces",
    "mcp__wire__analyze_pcap",
];

/// Write tools per domain. `notes` is a KNOWN static tool. `gmail`
/// names are best-effort (see module docs) — the verification guard
/// refuses a `write:gmail` spawn when none of them is advertised.
fn write_tools_for(domain: &str) -> Vec<&'static str> {
    match domain {
        "notes" => vec!["write_note"],
        "gmail" => vec![
            "mcp__google__gmail_modify",
            "mcp__google__gmail_label",
            "mcp__google__gmail_archive",
            "mcp__google__gmail_send",
        ],
        _ => vec![],
    }
}

/// Full allowlist for a scope: read tools, plus the domain's write
/// tools for a write scope.
pub fn allowlist_for(scope: &Scope) -> Vec<String> {
    let mut tools: Vec<String> = STATIC_READ_TOOLS
        .iter()
        .chain(GMAIL_READ_TOOLS.iter())
        .chain(WIRE_READ_TOOLS.iter())
        .map(|s| s.to_string())
        .collect();
    if let Scope::Write(domain) = scope {
        tools.extend(write_tools_for(domain).iter().map(|s| s.to_string()));
    }
    tools
}

/// Persona role name for a scope (sanitized: lowercase + hyphens, so
/// `persona_path` accepts it).
pub fn role_for(scope: &Scope) -> String {
    match scope {
        Scope::Read => "chat-task-read".to_string(),
        Scope::Write(domain) => format!("chat-task-write-{domain}"),
    }
}

/// Persona file content for a role: frontmatter `tools:` allowlist
/// plus a short body (the body joins the sub-agent's system prompt;
/// it states the role, it does not grant anything — the allowlist
/// does that).
pub fn role_file_content(role: &str, tools: &[String]) -> String {
    format!(
        "---\ntools: [{}]\n---\nTask worker role `{role}` (managed by the chat-command handler; rewritten on task submit).\nComplete the assigned task and report concisely.\n",
        tools.join(", ")
    )
}

/// Write the role file at `path` (creating parent dirs). Skips the
/// write when the content is already identical. Returns whether it
/// wrote. Errors are fatal to the submit (fail closed).
pub fn ensure_role_file(path: &Path, content: &str) -> Result<bool, String> {
    if let Ok(existing) = std::fs::read_to_string(path) {
        if existing == content {
            return Ok(false);
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("could not create role dir: {e}"))?;
    }
    std::fs::write(path, content).map_err(|e| format!("could not write role file: {e}"))?;
    Ok(true)
}

/// Tool names from a `tool_definitions()` value (array of
/// `{"function": {"name": ...}}` entries).
pub fn tool_names_from_definitions(defs: &serde_json::Value) -> Vec<String> {
    defs.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|d| d.get("function")?.get("name")?.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Verify a filtered toolset against a scope BEFORE spawning. This
/// is what makes the persona-file mechanism fail-closed: a missing
/// file (or an all-unknown allowlist) yields the FULL toolset, which
/// is NOT a subset of the scope allowlist and is refused here.
/// Also refuses a write scope whose write tools are absent (e.g.
/// `write:gmail` when google-mcp advertises no Gmail write tools) —
/// running it read-only would silently under-deliver.
pub fn verify_toolset(filtered: &[String], scope: &Scope) -> Result<(), String> {
    if filtered.is_empty() {
        return Err("scope verification failed: allowlist produced no tools".to_string());
    }
    let allowed = allowlist_for(scope);
    for name in filtered {
        if !allowed.iter().any(|a| a == name) {
            return Err(format!(
                "scope verification failed: unexpected tool '{name}' for scope {}",
                scope.display()
            ));
        }
    }
    if let Scope::Write(domain) = scope {
        let writes = write_tools_for(domain);
        if !filtered.iter().any(|n| writes.iter().any(|w| w == n)) {
            return Err(format!(
                "scope verification failed: no write tool for domain '{domain}' available"
            ));
        }
    }
    Ok(())
}

// ── Queue state machine (pure, unit-tested) ─────────────────────────

/// One accepted LLM task. `hub_id` is the task's hub message id —
/// the report cites it, so the sender can correlate task → report.
/// Serializable: the queue file (M21) stores these verbatim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedTask {
    pub hub_id: i64,
    pub sender: String,
    pub backend: LlmBackend,
    pub task: String,
    pub time_secs: u64,
    pub scope: Scope,
}

/// Per-backend running task + FIFO queue. `running` holds the FULL
/// task (not just its hub id) so a restart can re-queue it with its
/// scope, budget, and text intact — a hub id alone could not
/// reconstruct the task.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct HandlerState {
    #[serde(default)]
    pub running: HashMap<LlmBackend, QueuedTask>,
    #[serde(default)]
    pub queues: HashMap<LlmBackend, VecDeque<QueuedTask>>,
}

/// What `submit_task` decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitDecision {
    /// Backend was idle — run now.
    RunNow,
    /// Backend busy — queued at this 1-based position, behind this
    /// running hub id.
    Queued { position: usize, running_id: i64 },
}

/// Submit a task to the state machine. Idle backend → running;
/// busy backend → FIFO queue (position reported to the sender).
pub fn submit_task(state: &mut HandlerState, task: QueuedTask) -> SubmitDecision {
    if let Some(running) = state.running.get(&task.backend) {
        let running_id = running.hub_id;
        let queue = state.queues.entry(task.backend).or_default();
        queue.push_back(task);
        SubmitDecision::Queued {
            position: queue.len(),
            running_id,
        }
    } else {
        state.running.insert(task.backend, task);
        SubmitDecision::RunNow
    }
}

/// Mark a backend's running task complete (only when the hub id
/// matches — a stale completion touches nothing) and pop the next
/// queued task for that backend, marking it running. Returns the
/// next task, if any, for the caller to spawn + announce.
pub fn complete_task(
    state: &mut HandlerState,
    backend: LlmBackend,
    hub_id: i64,
) -> Option<QueuedTask> {
    if state.running.get(&backend).map(|t| t.hub_id) != Some(hub_id) {
        return None;
    }
    state.running.remove(&backend);
    let next = state.queues.get_mut(&backend)?.pop_front()?;
    state.running.insert(backend, next.clone());
    Some(next)
}

// ── Process-global state ────────────────────────────────────────────

fn handler_state() -> &'static Mutex<HandlerState> {
    static STATE: OnceLock<Mutex<HandlerState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(HandlerState::default()))
}

fn pending_spawns() -> &'static Mutex<Vec<QueuedTask>> {
    static PENDING: OnceLock<Mutex<Vec<QueuedTask>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(Vec::new()))
}

/// Test reset: clears running/queued/pending state. Tick tests call
/// this first (under `test_env_lock`) so one test's running task
/// cannot make the next test's task queue instead of run.
#[cfg(test)]
pub fn reset_for_tests() {
    if let Ok(mut s) = handler_state().lock() {
        *s = HandlerState::default();
    }
    if let Ok(mut p) = pending_spawns().lock() {
        p.clear();
    }
}

// ── Queue persistence (M21) ─────────────────────────────────────────
// The queue file lives next to the cursor state:
// `<runtime_dir>/chat_command/queue.json`. Saved on every mutation
// (submit, advance, resume) so a crash can only lose the mutation
// in flight, never the whole queue. Loaded once per process by the
// tick; a corrupt or missing file starts empty with a warning.
// The runtime dir is threaded explicitly (never a process global):
// the tick tests each use a fresh tmpdir, and a global would pin
// saves to whichever test ran first.

/// Persisted queue file format version.
pub const QUEUE_FILE_VERSION: u32 = 1;

/// The queue file: a version tag plus the handler state verbatim.
/// `running` is stored in full (not just hub ids) so a restart can
/// re-queue a running task with its scope, budget, and text intact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueFile {
    pub version: u32,
    #[serde(flatten)]
    pub state: HandlerState,
}

/// How a queue load went. Missing and corrupt are distinguished for
/// logging, but the caller treats both the same way: start empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueLoad {
    /// No queue file yet (first run, or nothing was ever queued).
    Missing,
    /// File exists but does not parse.
    Corrupt,
    /// Parsed queue.
    Loaded,
}

pub fn queue_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("chat_command").join("queue.json")
}

/// Serialize a state to the queue file (pure: takes the state, so
/// tests can round-trip without touching the process global).
pub fn save_queue_to(runtime_dir: &Path, state: &HandlerState) -> std::io::Result<()> {
    let path = queue_path(runtime_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = QueueFile {
        version: QUEUE_FILE_VERSION,
        state: state.clone(),
    };
    std::fs::write(path, serde_json::to_string_pretty(&file)?)
}

/// Read + parse the queue file. Missing → default + `Missing`;
/// unparseable → default + `Corrupt`; otherwise the file + `Loaded`.
pub fn load_queue_from(runtime_dir: &Path) -> (QueueFile, QueueLoad) {
    let empty = || QueueFile {
        version: QUEUE_FILE_VERSION,
        state: HandlerState::default(),
    };
    match std::fs::read_to_string(queue_path(runtime_dir)) {
        Err(_) => (empty(), QueueLoad::Missing),
        Ok(s) => match serde_json::from_str::<QueueFile>(&s) {
            Ok(file) => (file, QueueLoad::Loaded),
            Err(_) => (empty(), QueueLoad::Corrupt),
        },
    }
}

/// Merge a loaded queue file into a state (startup-only: the state
/// must be empty — the tick loads once, before any submit). A task
/// that was RUNNING when Helix died goes to the FRONT of its
/// backend's queue (it never completed, so it must run again —
/// never silently marked done, and no report is invented for work
/// that did not run); the persisted queued tasks follow in FIFO
/// order. `running` is cleared: nothing is actually running after
/// a restart. Dedups by hub id, so a file that lists the same task
/// twice can never run it twice. Returns (restored, demoted).
pub fn merge_restored(state: &mut HandlerState, file: &QueueFile) -> (usize, usize) {
    let mut restored = 0usize;
    let mut demoted = 0usize;
    let mut merged: HashMap<LlmBackend, VecDeque<QueuedTask>> = HashMap::new();
    let mut backends: Vec<LlmBackend> = file.state.running.keys().copied().collect();
    for b in file.state.queues.keys() {
        if !backends.contains(b) {
            backends.push(*b);
        }
    }
    for backend in backends {
        let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
        let mut q: VecDeque<QueuedTask> = VecDeque::new();
        if let Some(rt) = file.state.running.get(&backend) {
            if seen.insert(rt.hub_id) {
                q.push_back(rt.clone());
                demoted += 1;
                restored += 1;
            }
        }
        if let Some(queued) = file.state.queues.get(&backend) {
            for t in queued {
                if seen.insert(t.hub_id) {
                    q.push_back(t.clone());
                    restored += 1;
                }
            }
        }
        if !q.is_empty() {
            merged.insert(backend, q);
        }
    }
    state.running.clear();
    state.queues = merged;
    (restored, demoted)
}

/// Save the process-global handler state to the runtime dir.
/// Best-effort: the caller logs a warning on Err — a failed save
/// must never reject or drop an accepted task.
pub fn save_queue(runtime_dir: &Path) -> std::io::Result<()> {
    match handler_state().lock() {
        Ok(state) => save_queue_to(runtime_dir, &state),
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "handler state poisoned",
        )),
    }
}

/// Load the persisted queue into the process-global state (startup
/// only). Returns (kind, restored, demoted).
pub fn load_queue(runtime_dir: &Path) -> (QueueLoad, usize, usize) {
    let (file, kind) = load_queue_from(runtime_dir);
    if kind != QueueLoad::Loaded {
        return (kind, 0, 0);
    }
    let (restored, demoted) = match handler_state().lock() {
        Ok(mut state) => merge_restored(&mut state, &file),
        Err(_) => (0, 0),
    };
    (kind, restored, demoted)
}

/// Pop the front task of each idle backend and mark it running.
/// Pure state transition; the caller spawns + announces.
pub fn pop_next_to_start(state: &mut HandlerState) -> Vec<QueuedTask> {
    let mut started = Vec::new();
    for backend in [LlmBackend::Server, LlmBackend::Desktop, LlmBackend::Gemini] {
        if state.running.contains_key(&backend) {
            continue;
        }
        if let Some(q) = state.queues.get_mut(&backend) {
            if let Some(task) = q.pop_front() {
                state.running.insert(backend, task.clone());
                started.push(task);
            }
        }
    }
    started
}

/// Resume queued tasks after a restart: start the front task of each
/// backend that has a queue, announce the resume, and persist the
/// new running state. A task whose spawn is deferred (no runtime)
/// goes to `pending_spawns`, like a fresh RunNow submit — the tick's
/// `drain_pending` retries it.
pub fn resume_queued(tools: &ResolvedChatTools, cfg: &ChatCommandConfig, runtime_dir: &Path) {
    let started = match handler_state().lock() {
        Ok(mut state) => pop_next_to_start(&mut state),
        Err(_) => Vec::new(),
    };
    if started.is_empty() {
        return;
    }
    if let Err(e) = save_queue(runtime_dir) {
        log::warn!("chat-command: failed to persist resumed queue: {e}");
    }
    for task in started {
        crate::utils::log_to_file(&format!(
            "chat-command: task resumed after restart hub_id={} backend={}",
            task.hub_id,
            task.backend.display()
        ));
        post_body(
            tools,
            &cfg.room,
            cfg.chunk_bytes,
            &format!(
                "task #{} resumed after restart on {}",
                task.hub_id,
                task.backend.display()
            ),
        );
        if !try_spawn(tools, &cfg.room, cfg.chunk_bytes, &task, runtime_dir) {
            if let Ok(mut pending) = pending_spawns().lock() {
                pending.push(task);
            }
        }
    }
}

static QUEUE_RESTORED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Test reset for the restore-once flag (the flag is process-global;
/// tick tests each use a fresh tmpdir and must be able to restore).
#[cfg(test)]
pub fn reset_restore_for_tests() {
    QUEUE_RESTORED.store(false, std::sync::atomic::Ordering::SeqCst);
}

/// Load the persisted queue once per process and resume it. Called
/// from the tick (after tools resolve, so resume announcements can
/// be posted). A tick that runs before tools are available never
/// reaches this call, so the flag stays unset and a later tick
/// retries the restore.
pub fn restore_queue_once(runtime_dir: &Path, tools: &ResolvedChatTools, cfg: &ChatCommandConfig) {
    if QUEUE_RESTORED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let (kind, restored, demoted) = load_queue(runtime_dir);
    match kind {
        QueueLoad::Missing => log::debug!("chat-command: no persisted queue yet"),
        QueueLoad::Corrupt => {
            log::warn!("chat-command: persisted queue corrupt, starting empty")
        }
        QueueLoad::Loaded => {
            log::info!(
                "chat-command: restored {restored} queued tasks ({demoted} were running at shutdown)"
            );
            if restored > 0 {
                resume_queued(tools, cfg, runtime_dir);
            }
        }
    }
}

// ── Reply posting ───────────────────────────────────────────────────

/// Chunk + send one report body. Failures are logged (same rule as
/// command replies: a failed reply never re-executes anything).
fn post_body(tools: &ResolvedChatTools, room: &str, chunk_bytes: usize, body: &str) {
    let chunks = reply::chunk_reply(body, chunk_bytes);
    if !reply::send_reply(tools, room, &chunks) {
        log::warn!("chat-command: task report send abandoned (room={room})");
    }
}

// ── Submit (tick side, sync, never blocks on a run) ─────────────────

/// Accept an LLM task from the tick: ensure the scope-role file,
/// then run now or queue, posting exactly one reply (ACK, queued /
/// busy, or rejection). Never blocks on the agent run.
#[allow(clippy::too_many_arguments)]
pub fn submit_llm_task(
    tools: &ResolvedChatTools,
    cfg: &ChatCommandConfig,
    runtime_dir: &Path,
    sender: &str,
    hub_id: i64,
    backend: LlmBackend,
    time_secs: u64,
    scope: Scope,
    task: &str,
) {
    let role = role_for(&scope);
    let allowlist = allowlist_for(&scope);
    let role_path = runtime_dir.join("agents").join(format!("{role}.md"));
    if let Err(e) = ensure_role_file(&role_path, &role_file_content(&role, &allowlist)) {
        crate::utils::log_to_file(&format!(
            "chat-command: task rejected hub_id={hub_id} sender={sender} backend={} reason={e}",
            backend.display()
        ));
        post_body(
            tools,
            &cfg.room,
            cfg.chunk_bytes,
            &format!("task #{hub_id} rejected: {e}"),
        );
        return;
    }
    let queued = QueuedTask {
        hub_id,
        sender: sender.to_string(),
        backend,
        task: task.to_string(),
        time_secs,
        scope,
    };
    let decision = match handler_state().lock() {
        Ok(mut state) => submit_task(&mut state, queued.clone()),
        Err(_) => {
            // Poisoned lock: fail closed (do not run an untracked task).
            crate::utils::log_to_file(&format!(
                "chat-command: task rejected hub_id={hub_id} sender={sender} reason=handler state poisoned"
            ));
            post_body(
                tools,
                &cfg.room,
                cfg.chunk_bytes,
                &format!("task #{hub_id} rejected: handler state unavailable"),
            );
            return;
        }
    };
    match decision {
        SubmitDecision::RunNow => {
            if !try_spawn(tools, &cfg.room, cfg.chunk_bytes, &queued, runtime_dir) {
                // No runtime (sync tests, or an exotic sync context):
                // the task is accepted + running, and the spawn is
                // retried by `drain_pending` on a later tick.
                if let Ok(mut pending) = pending_spawns().lock() {
                    pending.push(queued);
                }
            }
            crate::utils::log_to_file(&format!(
                "chat-command: task accepted hub_id={hub_id} sender={sender} backend={} budget={time_secs}s",
                backend.display()
            ));
            post_body(
                tools,
                &cfg.room,
                cfg.chunk_bytes,
                &format!(
                    "task #{hub_id} accepted, running on {}, budget {time_secs}s",
                    backend.display()
                ),
            );
        }
        SubmitDecision::Queued {
            position,
            running_id,
        } => {
            crate::utils::log_to_file(&format!(
                "chat-command: task queued hub_id={hub_id} sender={sender} backend={} behind={running_id} position={position}",
                backend.display()
            ));
            post_body(
                tools,
                &cfg.room,
                cfg.chunk_bytes,
                &format!(
                    "task #{hub_id} queued: {} is busy running task #{running_id}; position {position} in queue",
                    backend.display()
                ),
            );
        }
    }
    // Persist the new state (accepted task, running or queued). A
    // failed save must not reject the task — it is already accepted
    // and running/queued; the worst case is the pre-M21 behaviour
    // (a crash before the next successful save loses it).
    if let Err(e) = save_queue(runtime_dir) {
        log::warn!("chat-command: failed to persist queue after submit: {e}");
    }
}

/// Try to spawn a detached runner for a task. Returns false when no
/// tokio runtime is current (caller queues it for `drain_pending`).
fn try_spawn(
    tools: &ResolvedChatTools,
    room: &str,
    chunk_bytes: usize,
    task: &QueuedTask,
    runtime_dir: &Path,
) -> bool {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return false;
    };
    let tools = tools.clone();
    let room = room.to_string();
    let task = task.clone();
    let runtime_dir = runtime_dir.to_path_buf();
    handle.spawn(async move {
        run_llm_task(tools, room, chunk_bytes, task, &runtime_dir).await;
    });
    true
}

/// Retry spawns that were deferred for lack of a runtime. Called at
/// the end of each tick (after the message loop). In production the
/// submit already spawned, so this is normally a no-op.
pub fn drain_pending(tools: &ResolvedChatTools, cfg: &ChatCommandConfig, runtime_dir: &Path) {
    let pending: Vec<QueuedTask> = match pending_spawns().lock() {
        Ok(mut p) => std::mem::take(&mut *p),
        Err(_) => return,
    };
    if pending.is_empty() {
        return;
    }
    let mut still_pending = Vec::new();
    for task in pending {
        if !try_spawn(tools, &cfg.room, cfg.chunk_bytes, &task, runtime_dir) {
            still_pending.push(task);
        }
    }
    if let Ok(mut p) = pending_spawns().lock() {
        p.extend(still_pending);
    }
}

// ── Runner (detached task, async) ───────────────────────────────────

/// Classify a `dispatch_spawn` report for the header line.
fn report_header(hub_id: i64, backend: LlmBackend, report: &str) -> String {
    if report.starts_with("Sub-agent timed out") {
        format!("task #{hub_id} timed out (backend {})", backend.display())
    } else if report.starts_with("Sub-agent failed") || report.starts_with("Error:") {
        format!("task #{hub_id} failed (backend {})", backend.display())
    } else {
        format!("task #{hub_id} result (backend {})", backend.display())
    }
}

/// Scope note appended to every task report. Read reports state
/// explicitly that no writes were made (the write tools were absent
/// from the run's toolset, so the agent could not have written).
/// Write reports state the domain limit; they do NOT enumerate the
/// writes (no tool-call trace exists in the spawn report — see
/// module docs).
fn scope_note(scope: &Scope) -> String {
    match scope {
        Scope::Read => {
            "scope: read — write tools were not available to this task, so no writes were made."
                .to_string()
        }
        Scope::Write(domain) => {
            format!("scope: write:{domain} — writes were limited to {domain} tools.")
        }
    }
}

/// Run one task to completion: verify scope, spawn via the existing
/// machinery, post the report, advance the queue. Never starts with
/// `!` (headers start with `task #...`), and the sender gate
/// (helix excluded) means even a chunk starting with `!` could never
/// retrigger.
async fn run_llm_task(
    tools: ResolvedChatTools,
    room: String,
    chunk_bytes: usize,
    task: QueuedTask,
    runtime_dir: &Path,
) {
    let role = role_for(&task.scope);
    // Verify BEFORE spawning: a missing/wrong role file must refuse,
    // never silently run with the full toolset.
    let filtered = tool_names_from_definitions(&crate::persona::subagent_tool_definitions(&role));
    if let Err(e) = verify_toolset(&filtered, &task.scope) {
        crate::utils::log_to_file(&format!(
            "chat-command: task rejected hub_id={} backend={} reason={e}",
            task.hub_id,
            task.backend.display()
        ));
        post_body(
            &tools,
            &room,
            chunk_bytes,
            &format!("task #{} rejected: {e}", task.hub_id),
        );
        advance_queue(
            &tools,
            &room,
            chunk_bytes,
            task.backend,
            task.hub_id,
            runtime_dir,
        );
        return;
    }
    let spec = crate::tools::subagent_tool::SpawnSpec {
        task: task.task.clone(),
        backend: task.backend.spawn_backend().to_string(),
        role_hint: role,
        timeout_secs: task.time_secs,
    };
    crate::utils::log_to_file(&format!(
        "chat-command: task started hub_id={} backend={} budget={}s",
        task.hub_id,
        task.backend.display(),
        task.time_secs
    ));
    let report = crate::tools::subagent_tool::dispatch_spawn(&spec).await;
    let timed_out = report.starts_with("Sub-agent timed out");
    let header = report_header(task.hub_id, task.backend, &report);
    let body = format!("{header}\n{report}\n{}", scope_note(&task.scope));
    post_body(&tools, &room, chunk_bytes, &body);
    crate::utils::log_to_file(&format!(
        "chat-command: task {} hub_id={} backend={}",
        if timed_out { "timeout" } else { "completed" },
        task.hub_id,
        task.backend.display()
    ));
    advance_queue(
        &tools,
        &room,
        chunk_bytes,
        task.backend,
        task.hub_id,
        runtime_dir,
    );
}

/// Complete one backend's running task and, if a task was queued
/// behind it, announce + spawn it.
fn advance_queue(
    tools: &ResolvedChatTools,
    room: &str,
    chunk_bytes: usize,
    backend: LlmBackend,
    hub_id: i64,
    runtime_dir: &Path,
) {
    let next = match handler_state().lock() {
        Ok(mut state) => complete_task(&mut state, backend, hub_id),
        Err(_) => None,
    };
    // Persist the new state (running task gone, next promoted) — a
    // crash before this save would re-queue the completed task and
    // run it twice, so the save is part of the advance, not an
    // afterthought.
    if let Err(e) = save_queue(runtime_dir) {
        log::warn!("chat-command: failed to persist queue after advance: {e}");
    }
    let Some(next) = next else { return };
    crate::utils::log_to_file(&format!(
        "chat-command: task started hub_id={} backend={} (from queue)",
        next.hub_id,
        backend.display()
    ));
    post_body(
        tools,
        room,
        chunk_bytes,
        &format!("task #{} started on {}", next.hub_id, backend.display()),
    );
    if !try_spawn(tools, room, chunk_bytes, &next, runtime_dir) {
        if let Ok(mut pending) = pending_spawns().lock() {
            pending.push(next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(s: &str) -> Scope {
        super::super::envelope::parse_scope(s).expect("scope must parse")
    }

    #[test]
    fn read_allowlist_has_no_write_tools() {
        let allow = allowlist_for(&Scope::Read);
        for write in [
            "write_note",
            "send_email",
            "set_belief",
            "bayes_reset",
            "run_shell",
            "generate_mermaid_project_map",
            "spawn_subagent",
        ] {
            assert!(
                !allow.contains(&write.to_string()),
                "read must exclude {write}"
            );
        }
        for read in [
            "read_note",
            "read_email",
            "mcp__google__gmail_search",
            "mcp__google__gmail_read",
            "mcp__wire__capture",
            "mcp__wire__interfaces",
            "mcp__wire__analyze_pcap",
        ] {
            assert!(
                allow.contains(&read.to_string()),
                "read must include {read}"
            );
        }
    }

    #[test]
    fn read_allowlist_wire_tools_are_read_only_and_verify() {
        // Task 375: the loop's read scope must carry the wire tools,
        // and the verification guard must accept a filtered set that
        // contains them (otherwise the spawn is refused and the
        // nightly network report can only fail).
        for wire in [
            "mcp__wire__capture",
            "mcp__wire__interfaces",
            "mcp__wire__analyze_pcap",
        ] {
            assert!(
                allowlist_for(&Scope::Read).contains(&wire.to_string()),
                "read must include {wire}"
            );
            // Wire tools are reads: no write scope may claim them.
            assert!(
                !write_tools_for("notes").contains(&wire),
                "{wire} must not be a notes write tool"
            );
            assert!(
                !write_tools_for("gmail").contains(&wire),
                "{wire} must not be a gmail write tool"
            );
        }
        // A filtered set with wire tools present verifies clean.
        let mut filtered = allowlist_for(&Scope::Read);
        assert!(verify_toolset(&filtered, &Scope::Read).is_ok());
        // ... and a filtered set WITHOUT them still verifies (wire
        // server unconfigured: best-effort, like Gmail).
        filtered.retain(|n| !n.starts_with("mcp__wire__"));
        assert!(verify_toolset(&filtered, &Scope::Read).is_ok());
    }

    #[test]
    fn write_notes_allows_only_notes_writes() {
        let allow = allowlist_for(&scope("write:notes"));
        assert!(allow.contains(&"write_note".to_string()));
        assert!(!allow.contains(&"send_email".to_string()));
        assert!(!allow.contains(&"mcp__google__gmail_modify".to_string()));
    }

    #[test]
    fn role_names_are_sanitized() {
        assert_eq!(role_for(&Scope::Read), "chat-task-read");
        assert_eq!(role_for(&scope("write:gmail")), "chat-task-write-gmail");
    }

    #[test]
    fn role_file_content_carries_the_allowlist() {
        let content = role_file_content(
            "chat-task-read",
            &["read_note".to_string(), "read_email".to_string()],
        );
        assert!(content.starts_with("---\ntools: [read_note, read_email]\n---\n"));
        assert!(content.contains("chat-task-read"));
    }

    #[test]
    fn ensure_role_file_skips_identical_and_rewrites_changed() {
        let dir = std::env::temp_dir().join(format!(
            "helix-chat-task-role-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("agents").join("chat-task-read.md");
        assert!(ensure_role_file(&path, "v1").unwrap(), "first write");
        assert!(!ensure_role_file(&path, "v1").unwrap(), "identical skips");
        assert!(ensure_role_file(&path, "v2").unwrap(), "changed rewrites");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "v2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_toolset_fail_closed_cases() {
        let read = allowlist_for(&Scope::Read);
        assert!(verify_toolset(&read, &Scope::Read).is_ok());
        assert!(
            verify_toolset(&[], &Scope::Read).is_err(),
            "empty must fail"
        );
        let mut with_write = read.clone();
        with_write.push("write_note".to_string());
        assert!(
            verify_toolset(&with_write, &Scope::Read).is_err(),
            "read + write_note must fail"
        );
        // Full toolset (missing persona file) must fail: it contains
        // tools outside the scope allowlist.
        let mut full = read.clone();
        full.push("send_email".to_string());
        full.push("run_shell".to_string());
        assert!(verify_toolset(&full, &Scope::Read).is_err());
        // Write scopes: write tool present passes; absent fails.
        let notes = allowlist_for(&scope("write:notes"));
        assert!(verify_toolset(&notes, &scope("write:notes")).is_ok());
        assert!(
            verify_toolset(&read, &scope("write:notes")).is_err(),
            "read-only set must not satisfy a write scope"
        );
        let gmail = allowlist_for(&scope("write:gmail"));
        assert!(verify_toolset(&gmail, &scope("write:gmail")).is_ok());
        assert!(
            verify_toolset(&read, &scope("write:gmail")).is_err(),
            "no Gmail write tool advertised → refuse, never run read-only"
        );
    }

    #[test]
    fn queue_fifo_per_backend_and_idle_runs() {
        let mut state = HandlerState::default();
        let mk = |id: i64, backend: LlmBackend| QueuedTask {
            hub_id: id,
            sender: "cobble-jr".to_string(),
            backend,
            task: "t".to_string(),
            time_secs: 300,
            scope: Scope::Read,
        };
        assert_eq!(
            submit_task(&mut state, mk(1, LlmBackend::Server)),
            SubmitDecision::RunNow
        );
        assert_eq!(
            submit_task(&mut state, mk(2, LlmBackend::Server)),
            SubmitDecision::Queued {
                position: 1,
                running_id: 1
            }
        );
        assert_eq!(
            submit_task(&mut state, mk(3, LlmBackend::Server)),
            SubmitDecision::Queued {
                position: 2,
                running_id: 1
            }
        );
        // A different backend is idle even while server is busy.
        assert_eq!(
            submit_task(&mut state, mk(4, LlmBackend::Desktop)),
            SubmitDecision::RunNow
        );
        // Completing the running task pops the next in FIFO order.
        assert_eq!(
            complete_task(&mut state, LlmBackend::Server, 1)
                .unwrap()
                .hub_id,
            2
        );
        assert_eq!(
            complete_task(&mut state, LlmBackend::Server, 2)
                .unwrap()
                .hub_id,
            3
        );
        assert!(complete_task(&mut state, LlmBackend::Server, 3).is_none());
        // Stale completion touches nothing.
        assert_eq!(
            submit_task(&mut state, mk(5, LlmBackend::Server)),
            SubmitDecision::RunNow
        );
        assert!(complete_task(&mut state, LlmBackend::Server, 999).is_none());
        assert_eq!(
            state.running.get(&LlmBackend::Server).map(|t| t.hub_id),
            Some(5)
        );
    }

    #[test]
    fn report_headers_classify_and_never_start_with_bang() {
        for (report, needle) in [
            ("Sub-agent complete\ntask_id: x\nfinal:\ny", "result"),
            ("Sub-agent failed\ntask_id: x\nerror: boom", "failed"),
            (
                "Sub-agent timed out after 300s and was cancelled.",
                "timed out",
            ),
            ("Error: no healthy backend", "failed"),
        ] {
            let header = report_header(7, LlmBackend::Server, report);
            assert!(header.contains(needle), "{header} (want {needle})");
            assert!(header.starts_with("task #7 "), "{header}");
            assert!(!header.starts_with('!'), "{header}");
        }
        assert!(scope_note(&Scope::Read).contains("no writes were made"));
        assert!(scope_note(&scope("write:gmail")).contains("write:gmail"));
    }

    #[test]
    fn tool_names_extracted_from_definitions() {
        let defs = serde_json::json!([
            {"type": "function", "function": {"name": "read_note"}},
            {"type": "function", "function": {"name": "mcp__google__gmail_search"}},
            {"type": "other"}
        ]);
        assert_eq!(
            tool_names_from_definitions(&defs),
            vec![
                "read_note".to_string(),
                "mcp__google__gmail_search".to_string()
            ]
        );
    }

    // ── M21 queue persistence ────────────────────────────────────

    fn qtmp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "helix-chat-queue-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn qt(id: i64, backend: LlmBackend) -> QueuedTask {
        QueuedTask {
            hub_id: id,
            sender: "cobble-jr".to_string(),
            backend,
            task: format!("task {id}"),
            time_secs: 300,
            scope: Scope::Read,
        }
    }

    #[test]
    fn queue_save_load_round_trip_restores_order_running_demoted_to_front() {
        // GIVEN a state with a running task + queued tasks on two backends:
        let mut state = HandlerState::default();
        assert_eq!(
            submit_task(&mut state, qt(1, LlmBackend::Server)),
            SubmitDecision::RunNow
        );
        assert_eq!(
            submit_task(&mut state, qt(2, LlmBackend::Server)),
            SubmitDecision::Queued {
                position: 1,
                running_id: 1
            }
        );
        assert_eq!(
            submit_task(&mut state, qt(3, LlmBackend::Server)),
            SubmitDecision::Queued {
                position: 2,
                running_id: 1
            }
        );
        assert_eq!(
            submit_task(&mut state, qt(4, LlmBackend::Desktop)),
            SubmitDecision::RunNow
        );

        // WHEN saved to disk and loaded into a fresh state,
        // THEN the running tasks are demoted to the FRONT of their backend
        // queue (they never completed — never silently done), queued
        // order is preserved, and nothing is marked running:
        let dir = qtmp("roundtrip");
        save_queue_to(&dir, &state).unwrap();
        let (file, kind) = load_queue_from(&dir);
        assert_eq!(kind, QueueLoad::Loaded);
        let mut back = HandlerState::default();
        let (restored, demoted) = merge_restored(&mut back, &file);
        assert_eq!((restored, demoted), (4, 2));
        assert!(back.running.is_empty(), "nothing runs after a restart");
        let server: Vec<i64> = back.queues[&LlmBackend::Server]
            .iter()
            .map(|t| t.hub_id)
            .collect();
        assert_eq!(
            server,
            vec![1, 2, 3],
            "running task 1 goes first, then FIFO"
        );
        let desktop: Vec<i64> = back.queues[&LlmBackend::Desktop]
            .iter()
            .map(|t| t.hub_id)
            .collect();
        assert_eq!(desktop, vec![4]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn queue_load_missing_and_corrupt_are_fail_soft() {
        // Missing file: no state, no crash.
        let dir = qtmp("missing");
        let (file, kind) = load_queue_from(&dir);
        assert_eq!(kind, QueueLoad::Missing);
        let mut back = HandlerState::default();
        assert_eq!(merge_restored(&mut back, &file), (0, 0));
        assert!(back.queues.is_empty());
        let _ = std::fs::remove_dir_all(&dir);

        // Corrupt file: reported, not a crash, empty state.
        let dir = qtmp("corrupt");
        let path = queue_path(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{not json").unwrap();
        let (file, kind) = load_queue_from(&dir);
        assert_eq!(kind, QueueLoad::Corrupt);
        let mut back = HandlerState::default();
        assert_eq!(merge_restored(&mut back, &file), (0, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn queue_merge_dedups_by_hub_id() {
        // A file that lists the same hub id as both running and queued
        // must restore it exactly once (never run a task twice).
        let file = QueueFile {
            version: QUEUE_FILE_VERSION,
            state: HandlerState {
                running: HashMap::from([(LlmBackend::Server, qt(1, LlmBackend::Server))]),
                queues: HashMap::from([(
                    LlmBackend::Server,
                    VecDeque::from([qt(1, LlmBackend::Server), qt(2, LlmBackend::Server)]),
                )]),
            },
        };
        let mut back = HandlerState::default();
        let (restored, demoted) = merge_restored(&mut back, &file);
        assert_eq!((restored, demoted), (2, 1));
        let server: Vec<i64> = back.queues[&LlmBackend::Server]
            .iter()
            .map(|t| t.hub_id)
            .collect();
        assert_eq!(server, vec![1, 2]);
    }

    #[test]
    fn queue_pop_next_to_start_marks_running_per_idle_backend() {
        // GIVEN queues on two backends and a third idle:
        let mut state = HandlerState::default();
        assert_eq!(
            submit_task(&mut state, qt(1, LlmBackend::Server)),
            SubmitDecision::RunNow
        );
        assert_eq!(
            submit_task(&mut state, qt(2, LlmBackend::Server)),
            SubmitDecision::Queued {
                position: 1,
                running_id: 1
            }
        );
        assert_eq!(
            submit_task(&mut state, qt(3, LlmBackend::Desktop)),
            SubmitDecision::RunNow
        );
        // Simulate a restart: running demoted to the queue front.
        let file = QueueFile {
            version: QUEUE_FILE_VERSION,
            state: HandlerState {
                running: std::mem::take(&mut state.running),
                queues: std::mem::take(&mut state.queues),
            },
        };
        let mut back = HandlerState::default();
        merge_restored(&mut back, &file);

        // WHEN popping the next task per backend,
        // THEN the front of each non-empty queue starts, the rest wait:
        let started = pop_next_to_start(&mut back);
        let ids: Vec<i64> = started.iter().map(|t| t.hub_id).collect();
        assert_eq!(ids, vec![1, 3], "front of server + desktop start");
        assert_eq!(
            back.running.get(&LlmBackend::Server).map(|t| t.hub_id),
            Some(1)
        );
        assert_eq!(
            back.running.get(&LlmBackend::Desktop).map(|t| t.hub_id),
            Some(3)
        );
        assert!(back.running.get(&LlmBackend::Gemini).is_none());
        let server: Vec<i64> = back.queues[&LlmBackend::Server]
            .iter()
            .map(|t| t.hub_id)
            .collect();
        assert_eq!(server, vec![2], "task 2 still queued behind 1");
    }
}
