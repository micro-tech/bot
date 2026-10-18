//! Bounded in-memory conversation history for the web chat path.
//!
//! Every web-chat turn used to be stateless: the backends built a fresh
//! message list (system prompt + current user message) on each request, so
//! follow-ups like "try again" arrived with no memory of the previous turn.
//! This module keeps the last few turns in a process-global ring buffer.
//!
//! Task 243: history now survives restarts. Each completed turn is appended
//! to `logs/chat_history.jsonl` (same `logs/` directory as the other logs;
//! override with `HELIX_CHAT_HISTORY_FILE`); on boot, `load_from_disk()`
//! reloads the last [`MAX_TURNS`] turns. Corrupt lines are skipped
//! (logged, never fatal).
//!
//! Design notes:
//! - A turn is recorded once the backend finishes it (success or failure).
//!   Failed turns are kept as user-only so a retry still resolves.
//! - History is injected as proper user/assistant message pairs, never as a
//!   text blob, and the in-flight turn is never part of it (it is recorded
//!   after the reply is produced).
//! - The lock is held only for short, non-async critical sections.

use std::collections::VecDeque;
use std::sync::{LazyLock, Mutex};

/// Maximum number of turns retained (a turn = user message + assistant reply).
pub const MAX_TURNS: usize = 10;
/// Per-message character cap, so one giant paste cannot blow the context
/// budget. Applied with char-boundary safety.
pub const MAX_TURN_CHARS: usize = 2000;
/// Assistant-side text stored when a turn produced no reply. Keeps history
/// as clean user/assistant pairs (Gemini requires role alternation) and
/// tells the model the previous attempt failed, so "try again" resolves.
pub const FAILED_TURN_MARKER: &str = "(no reply — request failed)";

/// One completed chat turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatTurn {
    /// What the user sent. Always non-empty for recorded turns.
    pub user: String,
    /// What the assistant replied. Empty when the turn failed.
    pub assistant: String,
}

static HISTORY: LazyLock<Mutex<VecDeque<ChatTurn>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));

fn lock() -> std::sync::MutexGuard<'static, VecDeque<ChatTurn>> {
    HISTORY.lock().unwrap_or_else(|e| e.into_inner())
}

fn truncate_chars(s: &str) -> String {
    if s.chars().count() > MAX_TURN_CHARS {
        s.chars().take(MAX_TURN_CHARS).collect()
    } else {
        s.to_string()
    }
}

/// Record one finished turn. Turns with an empty user message are ignored.
/// An empty `assistant` means the turn failed: it is stored with
/// [`FAILED_TURN_MARKER`] so the original request stays resolvable
/// ("try again") and role alternation is preserved.
///
/// Task 243: also appends the turn to the JSONL file (best-effort — a disk
/// failure must never break the in-memory path).
pub fn record_turn(user: &str, assistant: &str) {
    if user.trim().is_empty() {
        return;
    }
    let assistant = if assistant.trim().is_empty() {
        FAILED_TURN_MARKER.to_string()
    } else {
        truncate_chars(assistant)
    };
    let turn = ChatTurn {
        user: truncate_chars(user),
        assistant,
    };
    {
        let mut h = lock();
        h.push_back(turn.clone());
        while h.len() > MAX_TURNS {
            h.pop_front();
        }
    }
    append_to_disk(&turn);
}

/// Path of the JSONL history file. Override with `HELIX_CHAT_HISTORY_FILE`
/// (tests use a temp file); default co-locates with the other logs.
fn history_file_path() -> String {
    std::env::var("HELIX_CHAT_HISTORY_FILE")
        .unwrap_or_else(|_| "logs/chat_history.jsonl".to_string())
}

/// Append one turn as a JSONL line. Best-effort: logs and continues on error.
fn append_to_disk(turn: &ChatTurn) {
    let line =
        serde_json::json!({ "user": turn.user, "assistant": turn.assistant }).to_string();
    let path = history_file_path();
    if let Some(parent) = std::path::Path::new(&path).parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    use std::io::Write;
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(mut f) => {
            if let Err(e) = writeln!(f, "{}", line) {
                eprintln!("[chat_history] failed to append to {}: {}", path, e);
            }
        }
        Err(e) => eprintln!("[chat_history] failed to open {}: {}", path, e),
    }
}

/// Reload history from disk (call once at boot). Keeps the last
/// [`MAX_TURNS`] turns; corrupt lines are logged and skipped, never fatal.
/// Compacts the file to the retained turns so it can't grow without bound.
pub fn load_from_disk() {
    let path = history_file_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return, // no file yet — nothing to load
    };
    let mut turns: VecDeque<ChatTurn> = VecDeque::new();
    for (i, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(v) => {
                let user = v["user"].as_str().unwrap_or("").to_string();
                let assistant = v["assistant"].as_str().unwrap_or("").to_string();
                if user.trim().is_empty() {
                    eprintln!("[chat_history] line {}: empty user, skipping", i + 1);
                    continue;
                }
                turns.push_back(ChatTurn { user, assistant });
                while turns.len() > MAX_TURNS {
                    turns.pop_front();
                }
            }
            Err(e) => {
                eprintln!("[chat_history] line {}: corrupt, skipping ({})", i + 1, e);
            }
        }
    }
    {
        let mut h = lock();
        *h = turns.clone();
    }
    // Compact: rewrite with only the retained turns.
    if let Ok(mut f) = std::fs::File::create(&path) {
        use std::io::Write;
        // Task 265: a failed compaction write used to be swallowed per
        // line, silently losing history on the next restart. Report once.
        let mut first_err: Option<std::io::Error> = None;
        for t in &turns {
            let line =
                serde_json::json!({ "user": t.user, "assistant": t.assistant }).to_string();
            if let Err(e) = writeln!(f, "{}", line) {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
        if let Some(e) = first_err {
            eprintln!(
                "[chat_history] compaction write failed ({e}) — history may be lost on restart"
            );
        }
    }
}

/// Oldest-first snapshot of the retained turns. The in-flight turn is never
/// included: callers record only after producing the reply.
pub fn recent_turns() -> Vec<ChatTurn> {
    lock().iter().cloned().collect()
}

/// Drop all retained history. Used by tests; also a hook for a future
/// `/clear` style command.
pub fn clear() {
    lock().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_clean_history(f: impl FnOnce()) {
        // Task 243: isolate disk writes — each test gets a temp JSONL file.
        let _tl = crate::tools::subagent_tool::test_env_lock();
        let path = std::env::temp_dir().join(format!(
            "helix-chat-history-test-{}.jsonl",
            std::process::id()
        ));
        let path_str = path.to_str().unwrap().to_string();
        unsafe {
            std::env::set_var("HELIX_CHAT_HISTORY_FILE", &path_str);
        }
        let _ = std::fs::remove_file(&path);
        clear();
        f();
        clear();
        let _ = std::fs::remove_file(&path);
        unsafe {
            std::env::remove_var("HELIX_CHAT_HISTORY_FILE");
        }
    }

    #[test]
    fn records_and_returns_turns_oldest_first() {
        with_clean_history(|| {
            record_turn("first", "reply one");
            record_turn("second", "reply two");
            let turns = recent_turns();
            assert_eq!(turns.len(), 2);
            assert_eq!(turns[0].user, "first");
            assert_eq!(turns[0].assistant, "reply one");
            assert_eq!(turns[1].user, "second");
        });
    }

    #[test]
    fn evicts_oldest_beyond_cap() {
        with_clean_history(|| {
            for i in 0..(MAX_TURNS + 3) {
                record_turn(&format!("q{i}"), &format!("a{i}"));
            }
            let turns = recent_turns();
            assert_eq!(turns.len(), MAX_TURNS);
            assert_eq!(turns[0].user, "q3");
            assert_eq!(turns[MAX_TURNS - 1].user, format!("q{}", MAX_TURNS + 2));
        });
    }

    #[test]
    fn failed_turns_keep_user_message_with_marker() {
        with_clean_history(|| {
            record_turn("search my gmail for starlink", "");
            let turns = recent_turns();
            assert_eq!(turns.len(), 1);
            assert_eq!(turns[0].user, "search my gmail for starlink");
            assert_eq!(turns[0].assistant, FAILED_TURN_MARKER);
        });
    }

    #[test]
    fn ignores_empty_user_messages() {
        with_clean_history(|| {
            record_turn("   ", "hello?");
            record_turn("", "");
            assert!(recent_turns().is_empty());
        });
    }

    #[test]
    fn truncates_long_messages_at_char_boundary() {
        with_clean_history(|| {
            // 'é' is multi-byte: take() on chars keeps the boundary safe.
            let long = "é".repeat(MAX_TURN_CHARS + 100);
            record_turn(&long, "short");
            let turns = recent_turns();
            assert_eq!(turns[0].user.chars().count(), MAX_TURN_CHARS);
            assert_eq!(turns[0].assistant, "short");
        });
    }

    #[test]
    fn clear_empties_history() {
        record_turn("x", "y");
        clear();
        assert!(recent_turns().is_empty());
    }

    #[test]
    fn persists_and_reloads_round_trip() {
        // Task 243: GIVEN two recorded turns,
        with_clean_history(|| {
            record_turn("first question", "first answer");
            record_turn("second question", "second answer");
            // WHEN the process "restarts" (memory cleared, reload from disk),
            clear();
            assert!(recent_turns().is_empty());
            load_from_disk();
            // THEN both turns come back in order:
            let turns = recent_turns();
            assert_eq!(turns.len(), 2);
            assert_eq!(turns[0].user, "first question");
            assert_eq!(turns[0].assistant, "first answer");
            assert_eq!(turns[1].user, "second question");
            assert_eq!(turns[1].assistant, "second answer");
        });
    }

    #[test]
    fn corrupt_lines_dont_break_reload() {
        // Task 243: GIVEN a JSONL file with corrupt lines mixed in,
        with_clean_history(|| {
            let path = std::env::var("HELIX_CHAT_HISTORY_FILE").unwrap();
            let mut content = String::new();
            content.push_str(&serde_json::json!({"user": "good one", "assistant": "good answer"}).to_string());
            content.push('\n');
            content.push_str("this is not json at all\n");
            content.push_str(&serde_json::json!({"user": "", "assistant": "empty user"}).to_string());
            content.push('\n');
            content.push_str(&serde_json::json!({"broken": "no user field"}).to_string());
            content.push('\n');
            content.push('\n');
            content.push_str(&serde_json::json!({"user": "good two", "assistant": "good answer two"}).to_string());
            content.push('\n');
            std::fs::write(&path, content).unwrap();
            // WHEN reloading,
            load_from_disk();
            // THEN the good turns load, corrupt lines are skipped, no crash:
            let turns = recent_turns();
            assert_eq!(turns.len(), 2);
            assert_eq!(turns[0].user, "good one");
            assert_eq!(turns[1].user, "good two");
        });
    }

    #[test]
    fn reload_enforces_turn_cap() {
        // Task 243: GIVEN a file with more than MAX_TURNS turns,
        with_clean_history(|| {
            let path = std::env::var("HELIX_CHAT_HISTORY_FILE").unwrap();
            let mut content = String::new();
            for i in 0..25 {
                let line = serde_json::json!({"user": format!("q{}", i), "assistant": format!("a{}", i)}).to_string();
                content.push_str(&line);
                content.push('\n');
            }
            std::fs::write(&path, content).unwrap();
            // WHEN reloading,
            load_from_disk();
            // THEN only the last MAX_TURNS survive (oldest evicted):
            let turns = recent_turns();
            assert_eq!(turns.len(), MAX_TURNS);
            assert_eq!(turns[0].user, "q15");
            assert_eq!(turns[MAX_TURNS - 1].user, "q24");
            // AND the file was compacted to the retained turns:
            let line_count = std::fs::read_to_string(&path)
                .unwrap()
                .lines()
                .count();
            assert_eq!(line_count, MAX_TURNS);
        });
    }
}
