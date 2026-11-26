//! Hourly network capture logger (task 376).
//!
//! Task 375 gave the chat-command loop live wire tools, so the nightly
//! network report can be grounded in a live capture. But a live capture
//! is a snapshot, not history. This module is the 24h archive: a seeded
//! cron routine (`network-capture-log`, every hour) runs an mcp_wire
//! capture and appends one compact summary line to
//! `<runtime>/logs/network_capture.log`, which the loop's `read_log` can
//! read back as `logs/network_capture.log` for the nightly report.
//!
//! Design (376.1, recorded in the task list):
//! - Location: `<runtime>/logs/network_capture.log` (under `logs/`).
//! - Cadence: hourly, legacy in-process routine — no LLM is spawned to
//!   poll.
//! - Capture: `mcp__wire__interfaces` first (pick the primary interface),
//!   then `mcp__wire__capture` {interface, count 50, duration_secs 10}.
//!   Both go through the existing `mcp_client::try_execute` path, on a
//!   blocking thread with a timeout. No new shell, no new subprocess.
//! - Summary: `pkts` = captured count; `retr`/`rst`/`dupack`/`ooo`
//!   counted from the packet Info column. True interface error/drop
//!   counters are NOT available from the wire tools (the `interfaces`
//!   tool returns no statistics), so the line records only what a live
//!   capture can observe and fabricates nothing.
//! - Line: `2026-11-21T05:00:00 ens18 pkts=14 retr=0 rst=0 dupack=0 ooo=0`
//! - Retention: cap 48 lines (48h). A missed capture logs a warning and
//!   appends nothing — the archive holds only real samples.
//! - Fail-soft: any failure returns Err (the caller logs a warning);
//!   nothing here panics or breaks Helix.

use std::io;
use std::path::Path;
use std::time::Duration;

/// File name of the capture log inside the runtime `logs/` dir.
pub const CAPTURE_LOG_FILENAME: &str = "network_capture.log";

/// Max lines kept in the capture log (48h at hourly cadence). The file
/// stays ~3KB, so `read_log`'s 2000-char tail covers ~33 lines (>24h).
pub const MAX_CAPTURE_LOG_LINES: usize = 48;

/// Packets per hourly sample (whichever hits first: count or duration).
pub const CAPTURE_COUNT: i64 = 50;
/// Seconds per hourly sample.
pub const CAPTURE_DURATION_SECS: i64 = 10;
/// Budget for the capture MCP call (tshark itself caps at duration+30s).
pub const CAPTURE_TIMEOUT_SECS: u64 = 60;
/// Budget for the interfaces MCP call.
pub const INTERFACES_TIMEOUT_SECS: u64 = 30;

/// One hourly capture, reduced to the counts a live capture can observe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureSummary {
    pub interface: String,
    pub captured: usize,
    /// Packets whose Info column says Retransmission.
    pub retransmissions: usize,
    /// Packets whose Info column carries a TCP RST.
    pub resets: usize,
    /// Packets whose Info column says Dup ACK.
    pub dup_acks: usize,
    /// Packets whose Info column says Out-Of-Order.
    pub out_of_order: usize,
}

/// Parse only the FIRST JSON value in `text`, ignoring any trailing
/// document. mcp_wire's `tool_ok_json` emits the same value twice
/// (structuredContent + a text block with the same JSON), and Helix's
/// `result_to_string` concatenates both, so an MCP tool output can
/// carry two JSON documents back-to-back. Strict `from_str` rejects
/// that ("trailing characters"); a first-value parse takes the leading
/// document and ignores the rest (task 382).
fn parse_first_json(text: &str) -> Result<serde_json::Value, serde_json::Error> {
    let mut values = serde_json::Deserializer::from_str(text).into_iter::<serde_json::Value>();
    match values.next() {
        Some(v) => v,
        // Empty input: no first value at all.
        None => Err(serde::de::Error::custom("EOF while parsing a value")),
    }
}

/// Reduce an `mcp__wire__capture` JSON output to a `CaptureSummary`.
/// `captured` falls back to the packets array length when the field is
/// absent. Returns Err when the output is not JSON or carries no
/// interface and no packets at all.
pub fn summarize_capture(capture_json: &str) -> Result<CaptureSummary, String> {
    let v: serde_json::Value =
        parse_first_json(capture_json).map_err(|e| format!("capture output is not JSON: {e}"))?;
    let interface = v
        .get("interface")
        .and_then(|i| i.as_str())
        .ok_or_else(|| "capture output has no interface".to_string())?
        .to_string();
    let packets = v.get("packets").and_then(|p| p.as_array());
    let captured = v
        .get("captured")
        .and_then(|c| c.as_u64())
        .map(|c| c as usize)
        .or_else(|| packets.map(|a| a.len()))
        .ok_or_else(|| "capture output has no captured count or packets".to_string())?;
    let mut summary = CaptureSummary {
        interface,
        captured,
        retransmissions: 0,
        resets: 0,
        dup_acks: 0,
        out_of_order: 0,
    };
    if let Some(packets) = packets {
        for pkt in packets {
            let info = pkt.get("info").and_then(|i| i.as_str()).unwrap_or("");
            if info.contains("Retransmission") {
                summary.retransmissions += 1;
            }
            if info.contains("[RST") {
                summary.resets += 1;
            }
            if info.contains("Dup ACK") {
                summary.dup_acks += 1;
            }
            if info.contains("Out-Of-Order") {
                summary.out_of_order += 1;
            }
        }
    }
    Ok(summary)
}

/// True for an IPv4 address in a LAN range (192.168/16, 10/8, 172.16/12).
/// Used to prefer the LAN interface over a tunnel/VPN interface when
/// picking the primary capture interface.
fn is_lan_ipv4(addr: &str) -> bool {
    if addr.strip_prefix("192.168.").is_some_and(|r| !r.is_empty()) {
        return true;
    }
    if addr.strip_prefix("10.").is_some_and(|r| !r.is_empty()) {
        return true;
    }
    if let Some(rest) = addr.strip_prefix("172.")
        && let Some(second) = rest.split('.').next().and_then(|s| s.parse::<u8>().ok())
    {
        return (16..=31).contains(&second);
    }
    false
}

/// Pick the primary capture interface from an `mcp__wire__interfaces`
/// JSON output: first non-loopback interface that is up and has an IPv4
/// address, preferring one with a LAN-range IPv4. `None` when no
/// interface qualifies (the caller logs a warning and skips the tick).
pub fn pick_primary_interface(interfaces_json: &str) -> Option<String> {
    let v: serde_json::Value = parse_first_json(interfaces_json).ok()?;
    let ifaces = v.get("interfaces")?.as_array()?;
    let mut fallback: Option<String> = None;
    for iface in ifaces {
        let Some(name) = iface.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        if name == "lo" {
            continue;
        }
        let operstate = iface
            .get("operstate")
            .and_then(|o| o.as_str())
            .unwrap_or("");
        if operstate != "up" {
            continue;
        }
        let mut has_ipv4 = false;
        let mut has_lan_ipv4 = false;
        if let Some(addrs) = iface.get("addresses").and_then(|a| a.as_array()) {
            for a in addrs {
                if a.get("family").and_then(|f| f.as_str()) != Some("inet") {
                    continue;
                }
                if let Some(addr) = a.get("address").and_then(|x| x.as_str()) {
                    has_ipv4 = true;
                    if is_lan_ipv4(addr) {
                        has_lan_ipv4 = true;
                    }
                }
            }
        }
        if !has_ipv4 {
            continue;
        }
        if has_lan_ipv4 {
            return Some(name.to_string());
        }
        if fallback.is_none() {
            fallback = Some(name.to_string());
        }
    }
    fallback
}

/// One compact log line for a capture summary. Timestamp is local ISO
/// (no spaces) so the line splits cleanly on whitespace.
pub fn format_capture_line(summary: &CaptureSummary, timestamp: &str) -> String {
    format!(
        "{} {} pkts={} retr={} rst={} dupack={} ooo={}",
        timestamp,
        summary.interface,
        summary.captured,
        summary.retransmissions,
        summary.resets,
        summary.dup_acks,
        summary.out_of_order
    )
}

/// Append one capture line to `<logs_dir>/network_capture.log`,
/// keeping at most `MAX_CAPTURE_LOG_LINES` (oldest lines dropped).
/// Creates the logs dir and the file when missing.
pub fn append_capture_line(logs_dir: &Path, line: &str) -> io::Result<()> {
    std::fs::create_dir_all(logs_dir)?;
    let path = logs_dir.join(CAPTURE_LOG_FILENAME);
    let mut lines: Vec<String> = match std::fs::read_to_string(&path) {
        Ok(content) => content.lines().map(|l| l.to_string()).collect(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    lines.push(line.to_string());
    if lines.len() > MAX_CAPTURE_LOG_LINES {
        let drop = lines.len() - MAX_CAPTURE_LOG_LINES;
        lines.drain(..drop);
    }
    let mut out = lines.join("\n");
    out.push('\n');
    std::fs::write(&path, out)
}

/// Run one MCP tool call on a blocking thread with a timeout.
/// Returns the tool output, or an Err describing why it did not arrive.
async fn call_mcp_with_timeout(
    tool: &str,
    args: serde_json::Value,
    timeout_secs: u64,
) -> Result<String, String> {
    let tool_owned = tool.to_string();
    let fut =
        tokio::task::spawn_blocking(move || crate::mcp_client::try_execute(&tool_owned, &args));
    match tokio::time::timeout(Duration::from_secs(timeout_secs), fut).await {
        Ok(Ok(Some(out))) => Ok(out),
        Ok(Ok(None)) => Err(format!(
            "mcp tool {tool} unavailable (MCP disabled or not configured)"
        )),
        Ok(Err(e)) => Err(format!("mcp call task failed: {e}")),
        Err(_) => Err(format!("mcp tool {tool} timed out after {timeout_secs}s")),
    }
}

/// True when an MCP output is a tool-layer error rather than a result.
/// Mirrors `chat_command::mcp_handler::is_tool_error`: the MCP layer
/// prefixes tool errors with "MCP tool reported an error:" and the
/// manager prefixes its own failures with "mcp:".
fn is_tool_error(output: &str) -> bool {
    let t = output.trim_start();
    t.starts_with("MCP tool reported an error:") || t.starts_with("mcp:")
}

/// One hourly capture cycle: pick the primary interface, capture, reduce
/// to a summary line, append it to `<runtime_dir>/logs/network_capture.log`.
/// Returns the appended line on success. On ANY failure returns Err with
/// the reason — the caller logs a warning and moves on (fail-soft); a
/// missed capture appends nothing.
pub async fn run_capture_cycle(runtime_dir: &Path) -> Result<String, String> {
    let interfaces_tool = crate::mcp_client::naming::tool_name("wire", "interfaces");
    let ifaces_out = call_mcp_with_timeout(
        &interfaces_tool,
        serde_json::json!({}),
        INTERFACES_TIMEOUT_SECS,
    )
    .await?;
    if is_tool_error(&ifaces_out) {
        return Err(format!("wire interfaces call failed: {ifaces_out}"));
    }
    let interface = pick_primary_interface(&ifaces_out).ok_or_else(|| {
        "no usable network interface found (need one up, non-loopback, with IPv4)".to_string()
    })?;
    let capture_tool = crate::mcp_client::naming::tool_name("wire", "capture");
    let capture_out = call_mcp_with_timeout(
        &capture_tool,
        serde_json::json!({
            "interface": interface,
            "count": CAPTURE_COUNT,
            "duration_secs": CAPTURE_DURATION_SECS,
        }),
        CAPTURE_TIMEOUT_SECS,
    )
    .await?;
    if is_tool_error(&capture_out) {
        return Err(format!("wire capture call failed: {capture_out}"));
    }
    let summary = summarize_capture(&capture_out)?;
    let timestamp = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let line = format_capture_line(&summary, &timestamp);
    append_capture_line(&runtime_dir.join("logs"), &line)
        .map_err(|e| format!("failed to append to capture log: {e}"))?;
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture_json(packets: &str) -> String {
        format!(
            r#"{{"interface":"ens18","filter":"","requested_count":50,"duration_secs":10,"captured":3,"packets":{packets}}}"#
        )
    }

    #[test]
    fn summarize_capture_counts_anomalies_from_info() {
        // GIVEN a capture with one clean packet, one retransmission, one
        // RST, one Dup ACK and one out-of-order segment:
        let json = capture_json(
            r#"[
            {"number":1,"info":"443 → 12345 [SYN]"},
            {"number":2,"info":"443 → 12345 [TCP Retransmission] 443 → 12345 [PSH, ACK]"},
            {"number":3,"info":"443 → 12345 [RST, ACK]"},
            {"number":4,"info":"443 → 12345 [TCP Dup ACK 2#1]"},
            {"number":5,"info":"443 → 12345 [TCP Out-Of-Order] 443 → 12345 [PSH, ACK]"}
            ]"#,
        );
        // WHEN reduced:
        let s = summarize_capture(&json).expect("valid capture");
        // THEN the counts land on the right buckets:
        assert_eq!(s.interface, "ens18");
        assert_eq!(s.captured, 3);
        assert_eq!(s.retransmissions, 1);
        assert_eq!(s.resets, 1);
        assert_eq!(s.dup_acks, 1);
        assert_eq!(s.out_of_order, 1);
    }

    #[test]
    fn summarize_capture_empty_capture_is_all_zeros() {
        let s = summarize_capture(&capture_json("[]")).expect("empty capture");
        assert_eq!(s.captured, 3); // the `captured` field, not the array
        assert_eq!(s.retransmissions, 0);
        assert_eq!(s.resets, 0);
        assert_eq!(s.dup_acks, 0);
        assert_eq!(s.out_of_order, 0);
    }

    #[test]
    fn summarize_capture_falls_back_to_packet_count() {
        // GIVEN no `captured` field, only the packets array:
        let json =
            r#"{"interface":"ens18","packets":[{"number":1,"info":"x"},{"number":2,"info":"y"}]}"#;
        let s = summarize_capture(json).expect("fallback");
        assert_eq!(s.captured, 2);
    }

    #[test]
    fn summarize_capture_rejects_bad_json_and_missing_interface() {
        assert!(summarize_capture("not json").is_err());
        assert!(summarize_capture(r#"{"captured":1,"packets":[]}"#).is_err());
        assert!(summarize_capture(r#"{"interface":"ens18"}"#).is_err());
    }

    #[test]
    fn summarize_capture_ignores_trailing_doubled_json() {
        // GIVEN the doubled output mcp_wire actually emits: the pretty
        // structuredContent JSON followed by the identical compact text
        // block, joined with "\n" by result_to_string (task 382):
        let single = capture_json(
            r#"[
            {"number":1,"info":"443 → 12345 [SYN]"},
            {"number":2,"info":"443 → 12345 [TCP Retransmission] 443 → 12345 [PSH, ACK]"}
            ]"#,
        );
        let v: serde_json::Value = serde_json::from_str(&single).unwrap();
        let doubled = format!(
            "{}\n{}",
            serde_json::to_string_pretty(&v).unwrap(),
            serde_json::to_string(&v).unwrap()
        );
        // WHEN reduced:
        let s = summarize_capture(&doubled).expect("doubled capture output");
        // THEN the first document is used and the trailing copy ignored:
        assert_eq!(s.interface, "ens18");
        assert_eq!(s.captured, 3);
        assert_eq!(s.retransmissions, 1);
        assert_eq!(s.resets, 0);
        assert_eq!(s.dup_acks, 0);
        assert_eq!(s.out_of_order, 0);
    }

    #[test]
    fn pick_primary_interface_prefers_lan_over_tunnel() {
        // GIVEN lo, a tailscale tunnel (100.x, up) and the LAN (192.168.x, up):
        let json = r#"{"interfaces":[
            {"name":"lo","operstate":"up","addresses":[{"family":"inet","address":"127.0.0.1"}]},
            {"name":"tailscale0","operstate":"up","addresses":[{"family":"inet","address":"100.84.56.110"}]},
            {"name":"ens18","operstate":"up","addresses":[{"family":"inet","address":"192.168.1.168"}]}
        ]}"#;
        assert_eq!(pick_primary_interface(json).as_deref(), Some("ens18"));
    }

    #[test]
    fn pick_primary_interface_falls_back_to_any_up_ipv4() {
        // GIVEN only a tunnel interface up with IPv4:
        let json = r#"{"interfaces":[
            {"name":"lo","operstate":"up","addresses":[{"family":"inet","address":"127.0.0.1"}]},
            {"name":"tailscale0","operstate":"up","addresses":[{"family":"inet","address":"100.84.56.110"}]}
        ]}"#;
        assert_eq!(pick_primary_interface(json).as_deref(), Some("tailscale0"));
    }

    #[test]
    fn pick_primary_interface_none_when_nothing_usable() {
        // GIVEN only loopback, or everything down / without IPv4:
        let json = r#"{"interfaces":[
            {"name":"lo","operstate":"up","addresses":[{"family":"inet","address":"127.0.0.1"}]},
            {"name":"ens18","operstate":"down","addresses":[{"family":"inet","address":"192.168.1.168"}]},
            {"name":"eth1","operstate":"up","addresses":[{"family":"inet6","address":"fe80::1"}]}
        ]}"#;
        assert_eq!(pick_primary_interface(json), None);
        assert_eq!(pick_primary_interface("not json"), None);
    }

    #[test]
    fn pick_primary_interface_ignores_trailing_doubled_json() {
        // GIVEN the doubled output mcp_wire actually emits: the pretty
        // structuredContent JSON followed by the identical compact text
        // block, joined with "\n" by result_to_string (task 382):
        let single = r#"{"interfaces":[
            {"name":"lo","operstate":"up","addresses":[{"family":"inet","address":"127.0.0.1"}]},
            {"name":"tailscale0","operstate":"up","addresses":[{"family":"inet","address":"100.84.56.110"}]},
            {"name":"ens18","operstate":"up","addresses":[{"family":"inet","address":"192.168.1.168"}]}
        ]}"#;
        let v: serde_json::Value = serde_json::from_str(single).unwrap();
        let doubled = format!(
            "{}\n{}",
            serde_json::to_string_pretty(&v).unwrap(),
            serde_json::to_string(&v).unwrap()
        );
        // WHEN picking:
        // THEN the first document is used and ens18 wins:
        assert_eq!(pick_primary_interface(&doubled).as_deref(), Some("ens18"));
    }

    #[test]
    fn is_lan_ipv4_ranges() {
        assert!(is_lan_ipv4("192.168.1.168"));
        assert!(is_lan_ipv4("10.0.0.5"));
        assert!(is_lan_ipv4("172.16.0.1"));
        assert!(is_lan_ipv4("172.31.255.254"));
        assert!(!is_lan_ipv4("172.15.0.1"));
        assert!(!is_lan_ipv4("172.32.0.1"));
        assert!(!is_lan_ipv4("100.84.56.110"));
        assert!(!is_lan_ipv4("8.8.8.8"));
    }

    #[test]
    fn format_capture_line_shape() {
        let s = CaptureSummary {
            interface: "ens18".to_string(),
            captured: 14,
            retransmissions: 0,
            resets: 1,
            dup_acks: 2,
            out_of_order: 0,
        };
        assert_eq!(
            format_capture_line(&s, "2026-11-21T05:00:00"),
            "2026-11-21T05:00:00 ens18 pkts=14 retr=0 rst=1 dupack=2 ooo=0"
        );
    }

    fn tmp_logs(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join("helix-network-capture-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn append_capture_line_creates_file_and_appends() {
        // GIVEN a fresh logs dir:
        let dir = tmp_logs("append_creates");
        // WHEN two lines are appended:
        append_capture_line(
            &dir,
            "2026-11-21T05:00:00 ens18 pkts=1 retr=0 rst=0 dupack=0 ooo=0",
        )
        .unwrap();
        append_capture_line(
            &dir,
            "2026-11-21T06:00:00 ens18 pkts=2 retr=0 rst=0 dupack=0 ooo=0",
        )
        .unwrap();
        // THEN both lines are in the file, in order:
        let content = std::fs::read_to_string(dir.join(CAPTURE_LOG_FILENAME)).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("pkts=1"));
        assert!(lines[1].contains("pkts=2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_capture_line_caps_at_48_lines() {
        // GIVEN an empty logs dir:
        let dir = tmp_logs("append_caps");
        // WHEN 50 lines are appended:
        for i in 0..50 {
            append_capture_line(
                &dir,
                &format!("2026-11-21T{i:02}:00:00 ens18 pkts={i} retr=0 rst=0 dupack=0 ooo=0"),
            )
            .unwrap();
        }
        // THEN exactly 48 lines remain: the oldest two dropped, the
        // newest kept:
        let content = std::fs::read_to_string(dir.join(CAPTURE_LOG_FILENAME)).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), MAX_CAPTURE_LOG_LINES);
        assert!(
            lines[0].contains("pkts=2"),
            "oldest kept is #2: {}",
            lines[0]
        );
        assert!(
            lines[47].contains("pkts=49"),
            "newest kept is #49: {}",
            lines[47]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
