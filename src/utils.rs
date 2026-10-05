// Utility functions for Agent OS

use chrono::Local;
use log::error;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Returns a cross-platform path for the error log.
/// Windows: %APPDATA%\helix\logs\error_log.md
/// Linux/macOS: ~/.helix/logs/error_log.md
fn error_log_path() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
    } else {
        std::env::var("HOME")
            .map(|h| PathBuf::from(h).join(".helix"))
            .unwrap_or_else(|_| PathBuf::from(".helix"))
    };

    let log_dir = if cfg!(windows) {
        base.join("helix").join("logs")
    } else {
        base.join("logs")
    };

    // Ensure directory exists
    if let Err(e) = fs::create_dir_all(&log_dir) {
        // Can't log this yet, but we tried
        eprintln!("Failed to create log dir {:?}: {}", log_dir, e);
    }

    log_dir.join("error_log.md")
}

/// Logs a message to the error log file with a timestamp.
/// The log is human-readable markdown format.
pub fn log_to_file(message: &str) {
    let timestamp = Local::now().format("%Y-%m-%d %I:%M:%S %p").to_string();
    let log_entry = format!(
        "[{}] {}
",
        timestamp, message
    );

    let path = error_log_path();

    match OpenOptions::new().append(true).create(true).open(&path) {
        Ok(mut file) => {
            if let Err(e) = file.write_all(log_entry.as_bytes()) {
                error!("Failed to write to error log file {:?}: {}", path, e);
            }
        }
        Err(e) => {
            error!("Failed to open error log file {:?}: {}", path, e);
        }
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Return at most `max` bytes of `s`, never splitting a UTF-8 char boundary.
/// Unlike `&s[..n]`, this cannot panic on non-ASCII input.
pub fn truncate_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut boundary = max;
    while !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &s[..boundary]
}

/// Return at most the last `max` bytes of `s`, on a UTF-8 char boundary.
/// Unlike `&s[s.len() - max..]`, this cannot panic on non-ASCII input.
pub fn tail_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

/// Format the heartbeat.md body the tick loop writes each beat (task 200).
/// Kept here (not inline in main.rs) so the writer and the web server's
/// reader stay in lockstep, with a round-trip test below.
pub fn format_heartbeat_md(tick: u64, timestamp: &str, uptime_secs: u64, errors: u64) -> String {
    format!(
        "# Helix Heartbeat\ntick: {}\ntimestamp: {}\nuptime_secs: {}\nmode: autonomous\nerrors: {}\n",
        tick, timestamp, uptime_secs, errors
    )
}

/// Parse a heartbeat.md body back into (tick, timestamp, uptime_secs, errors).
/// Returns None when the body doesn't look like a beat file.
pub fn parse_heartbeat_md(content: &str) -> Option<(u64, String, u64, u64)> {
    let mut tick = None;
    let mut timestamp = None;
    let mut uptime_secs = None;
    let mut errors = None;
    for line in content.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue; // header/comment lines carry no fields
        };
        match k.trim() {
            "tick" => tick = v.trim().parse().ok(),
            "timestamp" => timestamp = Some(v.trim().to_string()),
            "uptime_secs" => uptime_secs = v.trim().parse().ok(),
            "errors" => errors = v.trim().parse().ok(),
            _ => {}
        }
    }
    Some((tick?, timestamp?, uptime_secs?, errors?))
}

/// Seconds since the beat file was last modified, measured against `now`.
/// None when the file is missing or its mtime is unreadable (treated as
/// infinitely stale by the watchdog).
pub fn heartbeat_stale_secs(
    path: &std::path::Path,
    now: std::time::SystemTime,
) -> Option<u64> {
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    now.duration_since(mtime).ok().map(|d| d.as_secs())
}

/// Missed-beat predicate (task 200): a beat is missed when the file is older
/// than 3x the tick interval — or when there is no beat file at all.
pub fn is_heartbeat_missed(stale_secs: Option<u64>, interval_secs: u64) -> bool {
    match stale_secs {
        Some(s) => s > 3 * interval_secs,
        None => true,
    }
}

#[cfg(test)]
mod heartbeat_utils_tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    #[test]
    fn heartbeat_md_round_trips() {
        // GIVEN a formatted beat file body:
        let body = format_heartbeat_md(42, "2026-10-05T01:30:00Z", 3600, 2);
        // WHEN parsed back:
        let parsed = parse_heartbeat_md(&body).expect("must parse");
        // THEN every field survives the round trip:
        assert_eq!(parsed.0, 42);
        assert_eq!(parsed.1, "2026-10-05T01:30:00Z");
        assert_eq!(parsed.2, 3600);
        assert_eq!(parsed.3, 2);
    }

    #[test]
    fn heartbeat_md_garbage_does_not_parse() {
        assert!(parse_heartbeat_md("not a beat file").is_none());
        assert!(parse_heartbeat_md("").is_none());
    }

    #[test]
    fn staleness_predicate_fresh_beat_is_not_missed() {
        // GIVEN a beat file written just now:
        let dir = std::env::temp_dir().join("helix-hb-test-fresh");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("heartbeat.md");
        std::fs::write(&path, format_heartbeat_md(1, "t", 10, 0)).unwrap();

        // WHEN checked against now with a 300s interval:
        let stale = heartbeat_stale_secs(&path, SystemTime::now());

        // THEN it is fresh and not missed:
        assert!(stale.unwrap_or(u64::MAX) < 5);
        assert!(!is_heartbeat_missed(stale, 300));
    }

    #[test]
    fn staleness_predicate_old_beat_is_missed() {
        // GIVEN a beat timestamp 10x the interval in the past:
        let now = SystemTime::now();
        let long_ago = now - Duration::from_secs(3000);

        // WHEN the predicate runs with a 300s interval:
        let stale = now
            .duration_since(long_ago)
            .ok()
            .map(|d| d.as_secs());

        // THEN the beat is reported missed:
        assert_eq!(stale, Some(3000));
        assert!(is_heartbeat_missed(stale, 300));
    }

    #[test]
    fn staleness_predicate_missing_file_is_missed() {
        // GIVEN no beat file at all:
        let stale: Option<u64> = None;
        // THEN the watchdog treats it as missed (fail-loud, not fail-silent):
        assert!(is_heartbeat_missed(stale, 300));
    }

    #[test]
    fn staleness_predicate_boundary_is_not_missed() {
        // Exactly 3x the interval is stale but not yet missed (strict >).
        assert!(!is_heartbeat_missed(Some(900), 300));
        assert!(is_heartbeat_missed(Some(901), 300));
    }
}

/// John's hartbeat.md nightly window: 1:00 AM (inclusive) to 7:00 AM
/// (exclusive), local time. Nightly maintenance — and the nighttime
/// code-write autonomy John granted on 2026-10-04 (task 202.3) — only run
/// inside this window.
pub fn in_nightly_window() -> bool {
    in_nightly_window_at(&chrono::Local::now())
}

/// Testable core of [`in_nightly_window`].
pub fn in_nightly_window_at<Tz: chrono::TimeZone>(dt: &chrono::DateTime<Tz>) -> bool {
    use chrono::Timelike;
    let h = dt.hour();
    h >= 1 && h < 7
}

#[cfg(test)]
mod nightly_window_tests {
    use super::*;
    use chrono::{Local, TimeZone};

    fn at(h: u32, m: u32) -> chrono::DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 5, h, m, 0).single().unwrap()
    }

    #[test]
    fn window_predicate_matches_hartbeat() {
        // 02:30 is inside the 1–7am window; 14:00 is out.
        assert!(in_nightly_window_at(&at(2, 30)));
        assert!(!in_nightly_window_at(&at(14, 0)));
    }

    #[test]
    fn window_boundaries() {
        assert!(in_nightly_window_at(&at(1, 0))); // inclusive open
        assert!(!in_nightly_window_at(&at(0, 59)));
        assert!(in_nightly_window_at(&at(6, 59)));
        assert!(!in_nightly_window_at(&at(7, 0))); // exclusive close
        assert!(!in_nightly_window_at(&at(23, 0)));
    }
}
