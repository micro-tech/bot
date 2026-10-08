// src/memory/episodic.rs — timestamped event log with JSONL disk persistence (task 204).
//
// Every recorded episode is appended to `episodes.jsonl` (next to
// `beliefs.json`; configurable via `HELIX_EPISODES_FILE`) and fsynced, so the
// event history survives restarts. The in-memory `Vec` is a bounded recent
// window; the log file is the full archive. Corrupt lines are skipped with a
// warning, never fatal.

use crate::utils::now_ms;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const EPISODES_FILE: &str = "episodes.jsonl";

fn episodes_file() -> PathBuf {
    PathBuf::from(
        std::env::var("HELIX_EPISODES_FILE").unwrap_or_else(|_| EPISODES_FILE.to_string()),
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Episode {
    pub id: u64,
    pub event: String,
    pub timestamp: u64,
}

pub struct EpisodicMemory {
    episodes: Vec<Episode>,
    next_id: u64,
    max_len: usize,
    /// Persistence target; `None` = memory-only (tests, transient use).
    file: Option<PathBuf>,
}

impl EpisodicMemory {
    /// In-memory only. Prefer `load_or_new` in the bot.
    pub fn new(max_len: usize) -> Self {
        Self {
            episodes: Vec::new(),
            next_id: 0,
            max_len,
            file: None,
        }
    }

    /// Load the archive from `path`, replaying valid lines and skipping
    /// corrupt ones. The in-memory window is bounded to `max_len`, but the
    /// log file keeps everything ever recorded.
    pub fn load_from(path: &Path, max_len: usize) -> Self {
        let mut mem = Self {
            episodes: Vec::new(),
            next_id: 0,
            max_len,
            file: Some(path.to_path_buf()),
        };
        match fs::read_to_string(path) {
            Ok(content) if !content.trim().is_empty() => {
                for (n, line) in content.lines().enumerate() {
                    match serde_json::from_str::<Episode>(line) {
                        Ok(ep) => {
                            mem.next_id = mem.next_id.max(ep.id + 1);
                            mem.episodes.push(ep);
                        }
                        Err(e) => log::warn!(
                            "skipping corrupt episode line {} in {}: {}",
                            n + 1,
                            path.display(),
                            e
                        ),
                    }
                }
                // Bound the in-memory window; the file remains the archive.
                let len = mem.episodes.len();
                if len > max_len {
                    mem.episodes.drain(0..len - max_len);
                }
            }
            _ => {}
        }
        mem
    }

    /// Production constructor: disk-backed at the configured path.
    pub fn load_or_new(max_len: usize) -> Self {
        Self::load_from(&episodes_file(), max_len)
    }

    pub fn record(&mut self, event: String) -> u64 {
        let id = self.next_id;
        self.next_id += 1;

        let episode = Episode {
            id,
            event,
            timestamp: now_ms(),
        };

        // Append to the durable log first: the archive outlives the window.
        // Episodes are low-frequency writes, so fsync-per-record is the
        // documented durability choice (no debounced batching to lose).
        if let Some(path) = &self.file {
            match serde_json::to_string(&episode) {
                Ok(mut line) => {
                    line.push('\n');
                    if let Err(e) = append_and_sync(path, line.as_bytes()) {
                        log::error!(
                            "failed to persist episode {} to {}: {}",
                            id,
                            path.display(),
                            e
                        );
                    }
                }
                Err(e) => log::error!("failed to serialize episode {}: {}", id, e),
            }
        }

        self.episodes.push(episode);

        // bound the in-memory window
        if self.episodes.len() > self.max_len {
            let overflow = self.episodes.len() - self.max_len;
            self.episodes.drain(0..overflow);
        }

        id
    }

    pub fn recent(&self, n: usize) -> Vec<Episode> {
        let len = self.episodes.len();
        let start = len.saturating_sub(n);
        self.episodes[start..].to_vec()
    }

    /// Alias used by memory_continuity for ergonomic access.
    pub fn get_recent(&self, n: usize) -> Vec<String> {
        self.recent(n).into_iter().map(|e| e.event).collect()
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.episodes.len()
    }
}

/// Append bytes to `path` (creating it) and fsync, so a crash right after
/// `record` returns cannot lose the episode.
fn append_and_sync(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("helix-episodic-test-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn test_new() {
        let mem = EpisodicMemory::new(10);
        assert!(mem.episodes.is_empty());
        assert_eq!(mem.next_id, 0);
        assert_eq!(mem.max_len, 10);
    }

    #[test]
    fn test_record() {
        let mut mem = EpisodicMemory::new(10);
        let id = mem.record("test event".to_string());
        assert_eq!(id, 0);
        assert_eq!(mem.episodes.len(), 1);
        assert_eq!(mem.episodes[0].event, "test event");
        assert_eq!(mem.next_id, 1);
    }

    #[test]
    fn test_recent() {
        let mut mem = EpisodicMemory::new(10);
        mem.record("event1".to_string());
        mem.record("event2".to_string());
        mem.record("event3".to_string());
        let recent = mem.recent(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].event, "event2");
        assert_eq!(recent[1].event, "event3");
    }

    #[test]
    fn test_max_len() {
        let mut mem = EpisodicMemory::new(2);
        mem.record("event1".to_string());
        mem.record("event2".to_string());
        mem.record("event3".to_string());
        assert_eq!(mem.episodes.len(), 2);
        assert_eq!(mem.episodes[0].event, "event2");
        assert_eq!(mem.episodes[1].event, "event3");
    }

    #[test]
    fn record_appends_jsonl_and_reload_restores_archive() {
        // GIVEN a disk-backed episodic memory:
        let path = scratch("archive.jsonl");
        let mut mem = EpisodicMemory::load_from(&path, 2);

        // WHEN three episodes are recorded (window holds 2),
        // THEN all three land in the log file:
        mem.record("event1".to_string());
        mem.record("event2".to_string());
        mem.record("event3".to_string());
        assert_eq!(mem.len(), 2, "in-memory window stays bounded");
        let lines: Vec<_> = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|l| l.to_string())
            .collect();
        assert_eq!(lines.len(), 3, "log file keeps the full archive");
        assert!(lines[0].contains("event1"));

        // WHEN reloaded (process-equivalent restart),
        // THEN the archive replays, ids continue, and the window re-bounds:
        drop(mem);
        let reloaded = EpisodicMemory::load_from(&path, 2);
        assert_eq!(reloaded.len(), 2);
        assert_eq!(reloaded.next_id, 3, "ids must not restart at zero");
        let recent = reloaded.recent(5);
        assert_eq!(recent[0].event, "event2");
        assert_eq!(recent[1].event, "event3");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn corrupt_lines_are_skipped_not_fatal() {
        // GIVEN a log with one corrupt line among good ones:
        let path = scratch("corrupt.jsonl");
        std::fs::write(
            &path,
            "{\"id\":0,\"event\":\"good1\",\"timestamp\":1}\nNOT JSON\n{\"id\":1,\"event\":\"good2\",\"timestamp\":2}\n",
        )
        .unwrap();

        // WHEN loaded, THEN good episodes survive and the bad line is skipped:
        let mem = EpisodicMemory::load_from(&path, 10);
        assert_eq!(mem.len(), 2);
        assert_eq!(mem.next_id, 2);

        let _ = std::fs::remove_file(&path);
    }
}
