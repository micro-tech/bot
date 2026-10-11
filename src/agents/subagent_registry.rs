//! Sub-agent registry (task 230): process-global live view of spawned sub-agents.
//!
//! Why process-global: cron/heartbeat/one-shot subagents are stateless and
//! `dispatch_spawn` is the only spawn path, so a single registry sees every
//! sub-agent. The web UI renders `agent_status` snapshots from this plus
//! per-backend activity, and receives `subagent_event` bus messages on every
//! spawn/finish (same publish contract as the task-219 `tool_call` event).

use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

/// Lifecycle states for one sub-agent entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentStatus {
    Working,
    Done,
    Failed,
    TimedOut,
}

impl SubagentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SubagentStatus::Working => "working",
            SubagentStatus::Done => "done",
            SubagentStatus::Failed => "failed",
            SubagentStatus::TimedOut => "timed_out",
        }
    }
}

/// One row in the UI's sub-agent list.
#[derive(Debug, Clone)]
pub struct SubagentRecord {
    pub id: String,
    pub backend: String,
    pub role_hint: String,
    pub task_summary: String,
    pub status: SubagentStatus,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
}

impl SubagentRecord {
    /// `task` is truncated to 120 chars — the UI shows a one-liner, not the
    /// full marching orders.
    pub fn new(id: String, backend: &str, role_hint: &str, task: &str) -> Self {
        let hint = role_hint.trim();
        Self {
            id,
            backend: backend.to_string(),
            role_hint: if hint.is_empty() {
                "(any)".to_string()
            } else {
                hint.to_string()
            },
            task_summary: task.chars().take(120).collect(),
            status: SubagentStatus::Working,
            started_at_ms: crate::utils::now_ms(),
            finished_at_ms: None,
        }
    }
}

/// Finished entries kept after completion — bounded so a long session can't
/// grow the registry without limit. Working entries are never pruned.
const MAX_FINISHED_KEPT: usize = 10;

fn registry() -> &'static Arc<RwLock<HashMap<String, SubagentRecord>>> {
    static REGISTRY: OnceLock<Arc<RwLock<HashMap<String, SubagentRecord>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Arc::new(RwLock::new(HashMap::new())))
}

// ── Change listener ───────────────────────────────────────────────────────────
// The web server installs this at startup; the registry calls it (lock-free)
// after every spawn/finish so the server can push a fresh `agent_status`
// snapshot. Registry stays decoupled from web_server — no import cycle.

static STATUS_LISTENER: OnceLock<Arc<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Install the on-change callback. First call wins; the web server sets this
/// once at startup.
pub fn set_status_listener(f: Arc<dyn Fn() + Send + Sync>) {
    let _ = STATUS_LISTENER.set(f);
}

fn notify_status_changed() {
    if let Some(f) = STATUS_LISTENER.get() {
        f();
    }
}

// ── Bus events ────────────────────────────────────────────────────────────────

pub fn record_to_json(r: &SubagentRecord) -> Value {
    serde_json::json!({
        "id": r.id,
        "backend": r.backend,
        "role_hint": r.role_hint,
        "task_summary": r.task_summary,
        "status": r.status.as_str(),
        "started_at_ms": r.started_at_ms,
        "finished_at_ms": r.finished_at_ms,
    })
}

/// Publish a `subagent_event` to `web_interface` — the same contract as the
/// task-219 `tool_call` event, so the existing bridge forwards it untouched.
fn publish_event(event: &str, subagent: &Value) {
    if let Some(bus) = crate::bus::global_bus() {
        let _ = bus.publish(crate::bus::Message {
            to: "web_interface".to_string(),
            from: "subagent_registry".to_string(),
            data: serde_json::json!({
                "type": "subagent_event",
                "event": event,
                "subagent": subagent,
            })
            .to_string(),
            timestamp: crate::utils::now_ms(),
        });
    }
    // No global bus (unit tests, bus-less binaries): the registry still works,
    // there is just nobody listening. Loud failure here would break every
    // spawn_subagent caller for zero benefit.
}

// ── Public API ────────────────────────────────────────────────────────────────

fn prune_locked(map: &mut HashMap<String, SubagentRecord>) {
    let mut finished: Vec<(u64, String)> = map
        .iter()
        .filter(|(_, r)| r.status != SubagentStatus::Working)
        .map(|(id, r)| (r.finished_at_ms.unwrap_or(0), id.clone()))
        .collect();
    if finished.len() <= MAX_FINISHED_KEPT {
        return;
    }
    finished.sort_by(|a, b| b.0.cmp(&a.0)); // newest first
    for (_, id) in finished.into_iter().skip(MAX_FINISHED_KEPT) {
        map.remove(&id);
    }
}

/// Register a freshly spawned sub-agent. Emits `subagent_event` "spawned".
pub fn register(record: SubagentRecord) {
    let payload = {
        let mut map = registry().write().unwrap();
        map.insert(record.id.clone(), record.clone());
        prune_locked(&mut map);
        record_to_json(&record)
    };
    // Lock is dropped before publish/notify: the status listener re-locks the
    // registry for its snapshot, so notifying under the write lock would
    // deadlock the moment the web server is listening.
    publish_event("spawned", &payload);
    notify_status_changed();
}

/// Mark a sub-agent finished. Unknown ids are ignored (defensive: a finish
/// must never panic the chat loop). Emits `subagent_event` "finished".
pub fn finish(id: &str, status: SubagentStatus) {
    let payload = {
        let mut map = registry().write().unwrap();
        let payload = match map.get_mut(id) {
            Some(rec) => {
                rec.status = status;
                rec.finished_at_ms = Some(crate::utils::now_ms());
                Some(record_to_json(rec))
            }
            None => None,
        };
        prune_locked(&mut map);
        payload
    };
    if let Some(p) = payload {
        publish_event("finished", &p);
        notify_status_changed();
    }
}

/// Current entries, oldest first. The UI re-renders from this wholesale —
/// no incremental-sync state to drift.
pub fn snapshot() -> Vec<SubagentRecord> {
    let map = registry().read().unwrap();
    let mut v: Vec<SubagentRecord> = map.values().cloned().collect();
    v.sort_by_key(|r| r.started_at_ms);
    v
}

/// Test-only: empty the registry between tests. The process-global map is
/// shared by every test in the binary, so tests must clean up after
/// themselves (and use unique id prefixes to avoid cross-test collisions).
#[cfg(test)]
pub fn reset_for_tests() {
    registry().write().unwrap().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    // Serialize registry tests: they share the process-global map.
    static TLOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    fn tlock() -> std::sync::MutexGuard<'static, ()> {
        TLOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn rec(id: &str) -> SubagentRecord {
        SubagentRecord::new(id.to_string(), "local", "planner", "do the thing")
    }

    #[test]
    fn register_then_finish_lifecycle_updates_status() {
        let _t = tlock();
        // GIVEN an empty registry:
        reset_for_tests();

        // WHEN a sub-agent is registered and then finishes,
        // THEN the snapshot shows it Working first and Done after:
        register(rec("test-reg-lifecycle-1"));
        let snap = snapshot();
        let r = snap.iter().find(|r| r.id == "test-reg-lifecycle-1").unwrap();
        assert_eq!(r.status, SubagentStatus::Working);
        assert_eq!(r.role_hint, "planner");
        assert!(r.finished_at_ms.is_none());

        finish("test-reg-lifecycle-1", SubagentStatus::Done);
        let snap = snapshot();
        let r = snap.iter().find(|r| r.id == "test-reg-lifecycle-1").unwrap();
        assert_eq!(r.status, SubagentStatus::Done);
        assert!(r.finished_at_ms.is_some());

        // AND finishing an unknown id is a silent no-op, not a panic:
        finish("test-reg-nope-missing", SubagentStatus::Failed);
        reset_for_tests();
    }

    #[test]
    fn prune_keeps_all_working_plus_last_10_finished() {
        let _t = tlock();
        // GIVEN 3 working and 14 finished sub-agents:
        reset_for_tests();
        for i in 0..3 {
            register(rec(&format!("test-reg-prune-w{}", i)));
        }
        for i in 0..14 {
            let id = format!("test-reg-prune-f{}", i);
            register(rec(&id));
            // Ensure distinct finish timestamps (ms clock may not tick).
            std::thread::sleep(std::time::Duration::from_millis(2));
            finish(&id, SubagentStatus::Done);
        }

        // WHEN we look at the snapshot,
        // THEN all 3 working survive and only the 10 newest finished do:
        let snap = snapshot();
        let working: Vec<_> = snap
            .iter()
            .filter(|r| r.id.starts_with("test-reg-prune-w"))
            .collect();
        let finished: Vec<_> = snap
            .iter()
            .filter(|r| r.id.starts_with("test-reg-prune-f"))
            .collect();
        assert_eq!(working.len(), 3, "working entries must never be pruned");
        assert_eq!(finished.len(), 10, "only the last 10 finished are kept");
        // The 4 oldest finished (f0..f3) were evicted; f4..f13 remain:
        let ids: Vec<&str> = finished.iter().map(|r| r.id.as_str()).collect();
        for old in ["test-reg-prune-f0", "test-reg-prune-f1", "test-reg-prune-f2", "test-reg-prune-f3"] {
            assert!(!ids.contains(&old), "{} should have been pruned", old);
        }
        assert!(ids.contains(&"test-reg-prune-f13"), "newest finished must survive");
        reset_for_tests();
    }

    #[test]
    fn concurrent_register_and_finish_does_not_deadlock() {
        let _t = tlock();
        // GIVEN an empty registry:
        reset_for_tests();

        // WHEN 8 threads register+finish 25 sub-agents each,
        // THEN everything completes without deadlock or panic:
        let handles: Vec<_> = (0..8)
            .map(|t| {
                std::thread::spawn(move || {
                    for i in 0..25 {
                        let id = format!("test-reg-conc-{}-{}", t, i);
                        register(rec(&id));
                        finish(&id, SubagentStatus::Done);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("registry thread panicked or deadlocked");
        }
        // AND the finished cap still holds for our entries:
        let mine: Vec<_> = snapshot()
            .into_iter()
            .filter(|r| r.id.starts_with("test-reg-conc-"))
            .filter(|r| r.status != SubagentStatus::Working)
            .collect();
        assert!(mine.len() <= 10, "finished cap violated: {}", mine.len());
        reset_for_tests();
    }

    #[test]
    fn record_json_shape_matches_ui_contract() {
        let _t = tlock();
        // GIVEN a record with an empty role hint and a long task,
        // WHEN serialized,
        // THEN the UI gets every field it renders, with sane fallbacks:
        let r = SubagentRecord::new(
            "test-reg-json-1".to_string(),
            "gemini",
            "",
            &"x".repeat(200),
        );
        let j = record_to_json(&r);
        assert_eq!(j["id"], "test-reg-json-1");
        assert_eq!(j["backend"], "gemini");
        assert_eq!(j["role_hint"], "(any)");
        assert_eq!(j["status"], "working");
        assert_eq!(j["task_summary"].as_str().unwrap().chars().count(), 120);
        assert!(j["finished_at_ms"].is_null());
        reset_for_tests();
    }
}
