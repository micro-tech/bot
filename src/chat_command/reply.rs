//! Reply chunking + send (task 299).
//!
//! Replies echo the command, then the output. The hub caps a message
//! body at 8 KB, so replies are split into `chunk_bytes` byte chunks
//! on UTF-8 char boundaries, prefixed `[n/m] ` (the prefix counts
//! against the chunk budget), sent in order to the SAME room, paced
//! for the hub's burst rate limit, with retry-once on failure.

use crate::chat_command::ResolvedChatTools;

/// True when a `chat_send` output means the send FAILED. Real failure
/// modes are all marked: the MCP layer prefixes tool errors with
/// "MCP tool reported an error:", the manager prefixes its own
/// failures with "mcp:", and chat-mcp prefixes hub failures with
/// "hub error:". Anything else is treated as success (a successful
/// send returns the envelope JSON).
pub fn is_send_error(output: &str) -> bool {
    let t = output.trim_start();
    t.starts_with("MCP tool reported an error:")
        || t.starts_with("mcp:")
        || t.starts_with("hub error:")
}

/// Split `text` into chunks of at most `chunk_bytes` BYTES, each
/// prefixed `[n/m] ` (prefix included in the budget). Splits never
/// break a UTF-8 character. Chunks reassemble to the original text
/// (strip the `[n/m] ` prefix from each chunk and concatenate).
/// The chunk count is solved iteratively: the prefix length depends
/// on the total, and the total depends on the prefix length.
pub fn chunk_reply(text: &str, chunk_bytes: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    // Iterate the total until stable (prefix digits only change at
    // 10/100/... boundaries, so this converges in ≤2 rounds).
    let mut total = 1usize;
    loop {
        let chunks = split_with_total(text, chunk_bytes, total);
        if chunks.len() == total {
            return chunks;
        }
        total = chunks.len();
    }
}

fn split_with_total(text: &str, chunk_bytes: usize, total: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut rest = text;
    let mut index = 1usize;
    while !rest.is_empty() {
        let prefix = format!("[{index}/{total}] ");
        let avail = chunk_bytes.saturating_sub(prefix.len()).max(1);
        // Take whole chars while the next char still fits.
        let mut take = 0usize;
        for (byte_idx, ch) in rest.char_indices() {
            if byte_idx + ch.len_utf8() > avail {
                break;
            }
            take = byte_idx + ch.len_utf8();
        }
        if take == 0 {
            // One char wider than the budget (pathological chunk_bytes):
            // take it whole rather than splitting a character.
            take = rest.chars().next().map(|c| c.len_utf8()).unwrap_or(0);
        }
        chunks.push(format!("{prefix}{}", &rest[..take]));
        rest = &rest[take..];
        index += 1;
    }
    chunks
}

/// Send every chunk to `room` via `chat_send`, in order, with ~150 ms
/// pacing between chunks (hub burst rate limit) and retry-once per
/// chunk on failure (after a short sleep). Returns false if any chunk
/// was abandoned after its retry — remaining chunks are NOT sent
/// (a partial reply is better than a reordered one, and the failure
/// is logged by the caller).
pub fn send_reply(tools: &ResolvedChatTools, room: &str, chunks: &[String]) -> bool {
    for (i, chunk) in chunks.iter().enumerate() {
        if i > 0 {
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        if !send_one(tools, room, chunk) {
            std::thread::sleep(std::time::Duration::from_millis(500));
            if !send_one(tools, room, chunk) {
                log::warn!("chat-command: send failed twice, abandoning reply (room={room})");
                return false;
            }
        }
    }
    true
}

fn send_one(tools: &ResolvedChatTools, room: &str, body: &str) -> bool {
    match crate::mcp_client::try_execute(
        &tools.send,
        &serde_json::json!({"room": room, "body": body}),
    ) {
        Some(out) => !is_send_error(&out),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reassemble(chunks: &[String]) -> String {
        chunks
            .iter()
            .map(|c| {
                let body = c.split_once("] ").map(|(_, b)| b).unwrap_or(c.as_str());
                body
            })
            .collect()
    }

    #[test]
    fn chunking_exact_under_over() {
        // Exact: 10 bytes + "[1/1] " (6) = 16 budget.
        let one = chunk_reply("0123456789", 16);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0], "[1/1] 0123456789");
        // Under: short text, one chunk.
        assert_eq!(chunk_reply("hi", 7500).len(), 1);
        // Over: 20 bytes at budget 16 → "[1/3] " (6) leaves 10 bytes
        // per chunk → 2 chunks of 10... plus prefix math: 20 bytes
        // needs 2 chunks at 10 bytes each.
        let two = chunk_reply("01234567890123456789", 16);
        assert_eq!(two.len(), 2);
        assert_eq!(reassemble(&two), "01234567890123456789");
        for c in &two {
            assert!(c.len() <= 16, "chunk {c:?} exceeds budget");
        }
    }

    #[test]
    fn chunking_never_splits_multibyte() {
        let text = "héllo 🌍 ".repeat(200);
        let chunks = chunk_reply(&text, 100);
        for c in &chunks {
            assert!(c.len() <= 100, "chunk exceeds budget: {} bytes", c.len());
        }
        assert_eq!(reassemble(&chunks), text, "chunks must reassemble exactly");
        // Numbering is sequential from 1.
        for (i, c) in chunks.iter().enumerate() {
            assert!(c.starts_with(&format!("[{}/{}] ", i + 1, chunks.len())), "{c:?}");
        }
    }

    #[test]
    fn chunking_prefix_counts_against_budget() {
        // 7500-byte budget: every chunk incl. prefix must fit.
        let text = "x".repeat(20000);
        let chunks = chunk_reply(&text, 7500);
        for c in &chunks {
            assert!(c.len() <= 7500);
        }
        assert_eq!(reassemble(&chunks), text);
    }

    #[test]
    fn send_error_markers_detected() {
        assert!(is_send_error("MCP tool reported an error:\nhub error: 429"));
        assert!(is_send_error("mcp: tool call failed: timeout"));
        assert!(is_send_error("hub error: 413 body too large"));
        assert!(!is_send_error("{\"id\": 5, \"room\": \"command\", \"sender\": \"helix\"}"));
        assert!(!is_send_error("ok"));
    }
}
