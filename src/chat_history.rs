//! Bounded in-memory conversation history for the web chat path.
//!
//! Every web-chat turn used to be stateless: the backends built a fresh
//! message list (system prompt + current user message) on each request, so
//! follow-ups like "try again" arrived with no memory of the previous turn.
//! This module keeps the last few turns in a process-global ring buffer.
//!
//! Design notes:
//! - In-memory only, single-user box: no disk schema, no bus changes.
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
pub fn record_turn(user: &str, assistant: &str) {
    if user.trim().is_empty() {
        return;
    }
    let assistant = if assistant.trim().is_empty() {
        FAILED_TURN_MARKER.to_string()
    } else {
        truncate_chars(assistant)
    };
    let mut h = lock();
    h.push_back(ChatTurn {
        user: truncate_chars(user),
        assistant,
    });
    while h.len() > MAX_TURNS {
        h.pop_front();
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
        clear();
        f();
        clear();
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
}
