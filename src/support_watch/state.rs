//! Persisted poll-loop state (task 256).
//!
//! Lives at `<runtime_dir>/support_watch/state.json` so restarts don't lose
//! the cadence position or the set of already-seen message ids. Included in
//! `scripts/backup-state.sh` (task 256.4).

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
    pub fn mark_handled(&mut self, ids: &[String]) {
        self.unhandled_ids.retain(|id| !ids.contains(id));
    }

    pub fn state_path(runtime_dir: &Path) -> PathBuf {
        runtime_dir.join("support_watch").join("state.json")
    }

    /// Load from disk; missing/corrupt file yields a fresh default (a bad
    /// state file must never wedge the watcher — it just re-polls).
    pub fn load(runtime_dir: &Path) -> Self {
        std::fs::read_to_string(Self::state_path(runtime_dir))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
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
    fn save_load_round_trip() {
        let dir = std::env::temp_dir().join("helix-support-watch-test-rt");
        let _ = std::fs::remove_dir_all(&dir);
        let mut st = WatchState::default();
        st.mode = WatchMode::Burst;
        st.quiet_streak = 1;
        st.last_check_ms = Some(12345);
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
