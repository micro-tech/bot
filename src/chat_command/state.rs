//! Persisted chat-command cursor (tasks 296).
//!
//! Lives at `<runtime_dir>/chat_command/state.json` so restarts don't
//! re-execute commands. The cursor is the id of the last PROCESSED
//! message (executed or ignored) — persisted per message, never per
//! tick, so a crash mid-tick never re-executes an earlier command.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Cursor state. `room` is stored so a room change in config is
/// detected (a cursor from another room is meaningless).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandState {
    #[serde(default)]
    pub room: Option<String>,
    #[serde(default)]
    pub last_seen_id: Option<i64>,
}

/// How a state load went. Missing and corrupt are distinguished for
/// logging, but the caller treats both the same way: seed the cursor
/// to the room's latest id (never replay old commands).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateLoad {
    /// No state file yet (first run).
    Missing,
    /// File exists but does not parse.
    Corrupt,
    /// Parsed state.
    Loaded,
}

impl CommandState {
    pub fn state_path(runtime_dir: &Path) -> PathBuf {
        runtime_dir.join("chat_command").join("state.json")
    }

    /// Load from disk. Missing file → `Missing`; unparseable file →
    /// `Corrupt` (with a default state); otherwise `Loaded`.
    pub fn load(runtime_dir: &Path) -> (Self, StateLoad) {
        match std::fs::read_to_string(Self::state_path(runtime_dir)) {
            Err(_) => (Self::default(), StateLoad::Missing),
            Ok(s) => match serde_json::from_str(&s) {
                Ok(state) => (state, StateLoad::Loaded),
                Err(_) => (Self::default(), StateLoad::Corrupt),
            },
        }
    }

    /// Persist. Errors are returned for the caller to log, never
    /// panicked. The tick treats a failed save as a stop signal (rest
    /// of the page waits for the next tick) rather than processing
    /// messages whose cursor cannot advance.
    pub fn save(&self, runtime_dir: &Path) -> std::io::Result<()> {
        let path = Self::state_path(runtime_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "helix-chat-command-state-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_load_round_trip() {
        let dir = tmpdir("roundtrip");
        let state = CommandState {
            room: Some("command".to_string()),
            last_seen_id: Some(42),
        };
        state.save(&dir).unwrap();
        let (back, kind) = CommandState::load(&dir);
        assert_eq!(kind, StateLoad::Loaded);
        assert_eq!(back.room.as_deref(), Some("command"));
        assert_eq!(back.last_seen_id, Some(42));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_reports_missing() {
        let dir = tmpdir("missing");
        let (_, kind) = CommandState::load(&dir);
        assert_eq!(kind, StateLoad::Missing);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_reports_corrupt_not_crash() {
        let dir = tmpdir("corrupt");
        let path = CommandState::state_path(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{not json").unwrap();
        let (state, kind) = CommandState::load(&dir);
        assert_eq!(kind, StateLoad::Corrupt);
        assert_eq!(state.last_seen_id, None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
