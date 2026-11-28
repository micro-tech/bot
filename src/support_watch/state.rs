//! Persisted poll-loop state (task 256).
//!
//! Lives at `<runtime_dir>/support_watch/state.json` so restarts don't lose
//! the cadence position or the set of already-seen message ids. Included in
//! `scripts/backup-state.sh` (task 256.4).
//!
//! Corruption handling (task 402, M12): a state file that exists but does
//! not parse is CORRUPT, not empty. `load_status` reports it; the watcher
//! quarantines the file (`state.json.corrupt-<ms>`), logs loudly, and marks
//! the fetched batch seen WITHOUT drafting (`mark_seen_quietly`) — a corrupt
//! file must never read as "nothing seen", which is the reply-all shape
//! (the whole fetched batch looks new and gets a reply drafted to each).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Poll-loop mode. Serialized lowercase for the JSON file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WatchMode {
    #[default]
    NormalDay,
    NormalNight,
    Burst,
}

impl WatchMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            WatchMode::NormalDay => "NORMAL-DAY",
            WatchMode::NormalNight => "NORMAL-NIGHT",
            WatchMode::Burst => "BURST",
        }
    }
}

/// Maximum ids remembered. Bounds the state file; the watcher only needs
/// to recognize "already seen" for recent mail.
pub const MAX_SEEN_IDS: usize = 200;

/// Draft attempts before a message stops being re-driven (review B2).
/// A poison message (one that always fails drafting) must not burn a
/// Gemini call every minute forever — after this many attempts it stays
/// unhandled on the status line for John's manual handling.
pub const MAX_DRAFT_ATTEMPTS: u32 = 5;

/// Full poll-loop state.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WatchState {
    /// When the last real mailbox check ran (ms epoch). None = never.
    pub last_check_ms: Option<u64>,
    pub mode: WatchMode,
    /// Consecutive clean (no-new-mail) checks while in Burst.
    pub quiet_streak: u32,
    /// Id of the newest message seen on the last check.
    pub last_seen_uid: Option<String>,
    /// Recently seen message ids (capped at MAX_SEEN_IDS).
    #[serde(default)]
    pub seen_ids: Vec<String>,
    /// Seen ids with no staged draft, sent reply, or dismiss yet (task 257
    /// removes entries via `mark_handled`).
    #[serde(default)]
    pub unhandled_ids: Vec<String>,
    /// Draft attempts per message id (review B2). Failed drafts are
    /// re-driven on subsequent checks; the counter caps the retries so a
    /// poison message can't burn a Gemini call every minute forever.
    /// Cleared when the message becomes handled.
    #[serde(default)]
    pub draft_attempts: HashMap<String, u32>,
}

/// Outcome of loading the state file. `Missing` (first run) and
/// `Corrupt` (unparseable) both yield a fresh default — a bad state
/// file must never wedge the watcher — but only `Corrupt` needs the
/// quarantine + mark-seen treatment (task 402).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateLoad {
    Ok,
    Missing,
    Corrupt,
}

impl WatchState {
    pub fn unhandled_count(&self) -> usize {
        self.unhandled_ids.len()
    }

    /// Record freshly fetched ids. Returns the ids that are new (not
    /// previously seen). New ids join both `seen_ids` and `unhandled_ids`.
    pub fn note_seen(&mut self, ids: &[String]) -> Vec<String> {
        let mut fresh = Vec::new();
        for id in ids {
            if !self.seen_ids.contains(id) {
                fresh.push(id.clone());
            }
        }
        for id in &fresh {
            self.seen_ids.push(id.clone());
            self.unhandled_ids.push(id.clone());
        }
        // Cap: drop oldest. (Vec::contains is O(n) but n <= 200 — fine.)
        while self.seen_ids.len() > MAX_SEEN_IDS {
            self.seen_ids.remove(0);
        }
        while self.unhandled_ids.len() > MAX_SEEN_IDS {
            self.unhandled_ids.remove(0);
        }
        if let Some(last) = ids.last() {
            self.last_seen_uid = Some(last.clone());
        }
        fresh
    }

    /// A message became handled (draft staged, reply sent, or dismissed).
    /// Also clears its draft-attempt counter (review B2).
    pub fn mark_handled(&mut self, ids: &[String]) {
        self.unhandled_ids.retain(|id| !ids.contains(id));
        for id in ids {
            self.draft_attempts.remove(id);
        }
    }

    /// Record ids as seen WITHOUT making them unhandled — no drafts will
    /// ever be driven for them. Used when the state file was corrupt (task
    /// 402): the batch is acknowledged so it is never treated as new, but
    /// nothing is drafted from it. (A corrupt file must never turn into a
    /// reply-all: `note_seen` would put the whole batch in `unhandled_ids`,
    /// and the watcher would draft a reply to every one of them.)
    pub fn mark_seen_quietly(&mut self, ids: &[String]) {
        for id in ids {
            if !self.seen_ids.contains(id) {
                self.seen_ids.push(id.clone());
            }
        }
        while self.seen_ids.len() > MAX_SEEN_IDS {
            self.seen_ids.remove(0);
        }
        if let Some(last) = ids.last() {
            self.last_seen_uid = Some(last.clone());
        }
    }

    pub fn state_path(runtime_dir: &Path) -> PathBuf {
        runtime_dir.join("support_watch").join("state.json")
    }

    /// Load from disk, reporting whether the file was fine, missing, or
    /// corrupt. Missing/corrupt both yield a fresh default.
    pub fn load_status(runtime_dir: &Path) -> (Self, StateLoad) {
        let path = Self::state_path(runtime_dir);
        match std::fs::read_to_string(&path) {
            Err(_) => (Self::default(), StateLoad::Missing),
            Ok(s) => match serde_json::from_str::<Self>(&s) {
                Ok(st) => (st, StateLoad::Ok),
                Err(_) => (Self::default(), StateLoad::Corrupt),
            },
        }
    }

    /// Quarantine a corrupt state file: rename `state.json` to
    /// `state.json.corrupt-<epoch-ms>` so the corruption is preserved for
    /// inspection and can never be re-read as an empty `seen_ids`.
    /// Returns the quarantine path, or `None` if there was no file.
    pub fn quarantine(runtime_dir: &Path) -> Option<PathBuf> {
        let path = Self::state_path(runtime_dir);
        if !path.exists() {
            return None;
        }
        let dest = path.with_extension(format!("json.corrupt-{}", crate::utils::now_ms()));
        match std::fs::rename(&path, &dest) {
            Ok(()) => Some(dest),
            Err(_) => None,
        }
    }

    /// Load from disk; missing/corrupt file yields a fresh default (a bad
    /// state file must never wedge the watcher — it just re-polls).
    /// Prefer `load_status` when the caller needs to know the file was
    /// corrupt (task 402).
    pub fn load(runtime_dir: &Path) -> Self {
        Self::load_status(runtime_dir).0
    }

    /// Persist. Errors are returned for the caller to log, never panicked.
    pub fn save(&self, runtime_dir: &Path) -> std::io::Result<()> {
        let path = Self::state_path(runtime_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self).unwrap())?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_seen_returns_only_fresh_ids() {
        let mut st = WatchState::default();
        let fresh = st.note_seen(&["a".into(), "b".into()]);
        assert_eq!(fresh, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(st.unhandled_count(), 2);
        // Second sighting of "a" is not fresh; "c" is.
        let fresh2 = st.note_seen(&["a".into(), "c".into()]);
        assert_eq!(fresh2, vec!["c".to_string()]);
        assert_eq!(st.unhandled_count(), 3);
        assert_eq!(st.last_seen_uid.as_deref(), Some("c"));
    }

    #[test]
    fn mark_handled_removes_from_unhandled_but_keeps_seen() {
        let mut st = WatchState::default();
        st.note_seen(&["a".into(), "b".into()]);
        st.mark_handled(&["a".into()]);
        assert_eq!(st.unhandled_count(), 1);
        assert!(st.seen_ids.contains(&"a".to_string()));
        // Re-seeing "a" later is not "new" even though it was handled.
        assert!(st.note_seen(&["a".into()]).is_empty());
    }

    #[test]
    fn corrupt_state_file_yields_default() {
        let dir = std::env::temp_dir().join("helix-support-watch-test-corrupt");
        std::fs::create_dir_all(dir.join("support_watch")).unwrap();
        std::fs::write(WatchState::state_path(&dir), "not json{{{").unwrap();
        let st = WatchState::load(&dir);
        assert_eq!(st.unhandled_count(), 0);
        assert_eq!(st.mode, WatchMode::NormalDay);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_status_distinguishes_missing_corrupt_and_ok() {
        let dir = std::env::temp_dir().join("helix-support-watch-test-load-status");
        let _ = std::fs::remove_dir_all(&dir);
        // Missing file -> Missing.
        let (_, status) = WatchState::load_status(&dir);
        assert_eq!(status, StateLoad::Missing);
        // Corrupt file -> Corrupt.
        std::fs::create_dir_all(dir.join("support_watch")).unwrap();
        std::fs::write(WatchState::state_path(&dir), "not json{{{").unwrap();
        let (_, status) = WatchState::load_status(&dir);
        assert_eq!(status, StateLoad::Corrupt);
        // Valid file -> Ok.
        let mut st = WatchState::default();
        st.note_seen(&["a".into()]);
        st.save(&dir).unwrap();
        let (back, status) = WatchState::load_status(&dir);
        assert_eq!(status, StateLoad::Ok);
        assert_eq!(back.unhandled_count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quarantine_moves_corrupt_file_aside() {
        let dir = std::env::temp_dir().join("helix-support-watch-test-quarantine");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("support_watch")).unwrap();
        std::fs::write(WatchState::state_path(&dir), "not json{{{").unwrap();
        // WHEN quarantined,
        let dest = WatchState::quarantine(&dir).expect("quarantine path");
        // THEN the original is gone, the corrupt bytes are preserved aside,
        // and a fresh load sees a missing file (not a corrupt one):
        assert!(!WatchState::state_path(&dir).exists());
        assert!(dest.exists());
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "not json{{{");
        let (_, status) = WatchState::load_status(&dir);
        assert_eq!(status, StateLoad::Missing);
        // AND quarantining again (nothing there) is a no-op:
        assert_eq!(WatchState::quarantine(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mark_seen_quietly_never_creates_unhandled() {
        let mut st = WatchState::default();
        // GIVEN a batch marked seen quietly (corrupt-state path):
        st.mark_seen_quietly(&["a".into(), "b".into()]);
        // THEN the ids are seen (a re-sighting is not "new") but nothing
        // is unhandled, so no draft will ever be driven for them:
        assert_eq!(st.unhandled_count(), 0);
        assert!(st.note_seen(&["a".into(), "b".into()]).is_empty());
        assert_eq!(st.last_seen_uid.as_deref(), Some("b"));
        // AND the seen cap still applies:
        for i in 0..(MAX_SEEN_IDS + 10) {
            st.mark_seen_quietly(&[format!("x{}", i)]);
        }
        assert_eq!(st.seen_ids.len(), MAX_SEEN_IDS);
    }

    #[test]
    fn save_load_round_trip() {
        let dir = std::env::temp_dir().join("helix-support-watch-test-rt");
        let _ = std::fs::remove_dir_all(&dir);
        let mut st = WatchState {
            mode: WatchMode::Burst,
            quiet_streak: 1,
            last_check_ms: Some(12345),
            ..Default::default()
        };
        st.note_seen(&["x".into()]);
        st.save(&dir).expect("save");
        let back = WatchState::load(&dir);
        assert_eq!(back.mode, WatchMode::Burst);
        assert_eq!(back.quiet_streak, 1);
        assert_eq!(back.last_check_ms, Some(12345));
        assert_eq!(back.unhandled_count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
