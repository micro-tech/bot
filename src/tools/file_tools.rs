//! File-based tools: log reading, note writing/reading/listing, and repo exploration tools.

use crate::tools::network_discovery::{
    aggregate_hosts, discover, ping_sweep_subnet, resolve_hostnames, subnet_base, DiscoveredHost,
    KNOWN_CONFIG_HOSTS,
};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

// Bring in chrono for timestamps (already a dependency)

// ── Path sandboxing ───────────────────────────────────────────────────────────

/// Resolve `candidate` against `base_dir` and ensure the canonicalized result
/// stays inside `base_dir`. Defeats `..` segments (e.g. `logs/../../config.toml`)
/// and symlink escapes. `base_dir` should be absolute (we anchor it on
/// `current_dir()` at each call site).
fn resolve_inside(base_dir: &Path, candidate: &str) -> Result<PathBuf, String> {
    if candidate.contains('\0') {
        return Err(format!("Security error: invalid path '{}'.", candidate));
    }
    let joined = base_dir.join(candidate);
    let canon = joined
        .canonicalize()
        .map_err(|e| format!("Error reading '{}': {}", candidate, e))?;
    let base_canon = base_dir
        .canonicalize()
        .unwrap_or_else(|_| base_dir.to_path_buf());
    if !canon.starts_with(&base_canon) {
        return Err(format!(
            "Security error: path '{}' escapes the allowed directory.",
            candidate
        ));
    }
    Ok(canon)
}

// ── Existing note/log tools (kept for brevity) ────────────────────────────────

pub fn read_log(args: &Value) -> String {
    let file = args["log_file"].as_str().unwrap_or("logs/chat_log.md");
    let normalised = file.replace('\\', "/");
    if !normalised.starts_with("logs/") {
        return format!("Security error: only files inside logs/ are readable. Got '{}'.", file);
    }
    // Canonicalize under logs/: rejects `logs/../../config.toml` and symlink
    // escapes while keeping the historical "must start with logs/" contract.
    let base = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("logs");
    let rel = normalised.strip_prefix("logs/").unwrap_or(&normalised);
    let path = match resolve_inside(&base, rel) {
        Ok(p) => p,
        Err(e) => return e,
    };
    match fs::read_to_string(&path) {
        Ok(content) => {
            const MAX: usize = 2_000;
            if content.len() > MAX {
                format!(
                    "[…showing last {} chars of {}…]\n{}",
                    MAX,
                    file,
                    crate::utils::tail_str(&content, MAX)
                )
            } else if content.is_empty() {
                format!("'{}' exists but is empty.", file)
            } else {
                content
            }
        }
        Err(e) => format!("Error reading '{}': {}", file, e),
    }
}

pub fn write_note(args: &Value) -> String {
    let title = args["title"].as_str().unwrap_or("untitled");
    let content = args["content"].as_str().unwrap_or("");
    let safe = sanitise_filename(title);
    if safe.is_empty() {
        return "Error: note title must contain at least one alphanumeric character.".to_string();
    }
    if let Err(e) = fs::create_dir_all("notes") {
        return format!("Error creating notes/ directory: {}", e);
    }
    let path = format!("notes/{}.md", safe);
    let body = format!("# {}\n\n{}", title, content);
    match fs::write(&path, &body) {
        Ok(_) => format!("✅ Note '{}' saved to {}", title, path),
        Err(e) => format!("Error saving note to '{}': {}", path, e),
    }
}

pub fn read_note(args: &Value) -> String {
    let title = args["title"].as_str().unwrap_or("");
    let safe = sanitise_filename(title);
    let path = format!("notes/{}.md", safe);
    match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(_) => {
            let available = notes_list_internal();
            if available.is_empty() {
                format!("Note '{}' not found. No notes have been saved yet.", title)
            } else {
                format!("Note '{}' not found.\n\nAvailable notes:\n{}", title, available.join("\n"))
            }
        }
    }
}

pub fn list_notes() -> String {
    let notes = notes_list_internal();
    if notes.is_empty() {
        "No notes saved yet. Use write_note to create one.".to_string()
    } else {
        format!("Saved notes ({}):\n{}", notes.len(), notes.join("\n"))
    }
}

fn notes_list_internal() -> Vec<String> {
    fs::read_dir("notes")
        .map(|entries| {
            let mut names: Vec<String> = entries
                .flatten()
                .filter(|e| e.path().extension().map_or(false, |ext| ext == "md"))
                .filter_map(|e| e.path().file_stem().map(|s| format!("  • {}", s.to_string_lossy())))
                .collect();
            names.sort();
            names
        })
        .unwrap_or_default()
}

fn sanitise_filename(s: &str) -> String {
    let safe: String = s.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let mut result = String::new();
    let mut prev_under = false;
    for c in safe.chars() {
        if c == '_' {
            if !prev_under { result.push(c); }
            prev_under = true;
        } else {
            result.push(c);
            prev_under = false;
        }
    }
    result.trim_matches('_').to_string()
}

// ── Artifact tools (tasks 335, 338) ─────────────────────────────────────────

/// Max bytes of artifact content the model may write in one call.
/// Over-cap content is REJECTED (never silently truncated) — the model must
/// split it or summarise instead.
pub const MAX_ARTIFACT_BYTES: usize = 512 * 1024;

/// Map an artifact `kind` to its file extension. None = unknown kind.
pub fn artifact_extension(kind: &str) -> Option<&'static str> {
    match kind {
        "html" => Some("html"),
        "svg" => Some("svg"),
        "mermaid" => Some("mmd"),
        "markdown" => Some("md"),
        "csv" => Some("csv"),
        "text" => Some("txt"),
        _ => None,
    }
}

/// Shared artifact writer (task 335): the one place that puts model-produced
/// files on disk. `create_artifact` and `create_network_map` both go
/// through here — no duplicated file-writing plumbing.
/// Returns (url, kind, bytes) on success; the url is the path task 336
/// serves (`/artifacts/<file>`). If a file with the computed name already
/// exists (same title, same minute), a `-2`/`-3` suffix is added so a
/// second artifact never silently overwrites the first.
pub fn write_artifact_file(
    kind: &str,
    title: &str,
    content: &str,
) -> Result<(String, String, usize), String> {
    let ext = artifact_extension(kind).ok_or_else(|| {
        format!(
            "Error: unknown artifact kind '{}'. Use one of: html, svg, mermaid, markdown, csv, text.",
            kind
        )
    })?;
    let safe = sanitise_filename(title);
    if safe.is_empty() {
        return Err(
            "Error: artifact title must contain at least one alphanumeric character.".to_string(),
        );
    }
    if content.is_empty() {
        return Err("Error: artifact content is empty — nothing to save.".to_string());
    }
    if content.len() > MAX_ARTIFACT_BYTES {
        return Err(format!(
            "Error: artifact content is {} bytes, over the {} KB cap. Split it into smaller artifacts or summarise instead.",
            content.len(),
            MAX_ARTIFACT_BYTES / 1024
        ));
    }
    if let Err(e) = fs::create_dir_all("artifacts") {
        return Err(format!("Error creating artifacts/ directory: {}", e));
    }
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M");
    let mut filename = format!("{}-{}.{}", safe, stamp, ext);
    let mut n = 2usize;
    while Path::new("artifacts").join(&filename).exists() {
        filename = format!("{}-{}-{}.{}", safe, stamp, n, ext);
        n += 1;
    }
    let rel = format!("artifacts/{}", filename);
    match fs::write(&rel, content) {
        Ok(_) => Ok((format!("/artifacts/{}", filename), kind.to_string(), content.len())),
        Err(e) => Err(format!("Error saving artifact to '{}': {}", rel, e)),
    }
}

/// Format the success string the model sees. The Ollama loop parses this
/// exact shape (see `parse_artifact_saved` in io::ollama::tools) to publish
/// the artifact bus message — keep the format stable.
fn artifact_saved_string(url: &str, kind: &str, bytes: usize) -> String {
    format!("✅ Artifact saved: {} ({}, {} bytes)", url, kind, bytes)
}

/// create_artifact(kind, title, content) — task 335.
/// One generic tool for charts/graphs/files: the model writes the full file
/// content, Helix saves it under artifacts/ and serves it at /artifacts/*.
pub fn create_artifact(args: &Value) -> String {
    let kind = args["kind"].as_str().unwrap_or("");
    let title = args["title"].as_str().unwrap_or("");
    let content = args["content"].as_str().unwrap_or("");
    match write_artifact_file(kind, title, content) {
        Ok((url, kind, bytes)) => artifact_saved_string(&url, &kind, bytes),
        Err(e) => e,
    }
}

/// Escape text for embedding in SVG/XML element content or attributes.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// One interface entry from mcp_wire `interfaces` (live shape, Nov 14).
struct WireInterface {
    name: String,
    description: String,
    mac: String,
    mtu: String,
    operstate: String,
    speed: String,
    link_label: String,
    addresses: Vec<String>,
}

/// Leniently read a field that may be a JSON string or a JSON number.
fn field_as_string(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// Parse the mcp_wire `interfaces` result. Accepts `{"interfaces": [...]}` or
/// a bare `[...]` array; every field is optional and defaults to empty, so a
/// schema drift degrades to a sparse map instead of an error.
///
/// Task 343: mcp-wire returns the data TWICE — `result_to_string`
/// (src/mcp_client/manager.rs) concatenates the pretty-printed
/// structuredContent AND the compact text block, joined with "\n". A strict
/// `from_str` fails on the trailing second document, which is exactly the
/// live failure John hit ("failed to return the data in the expected
/// format"). So parse the FIRST JSON document in the string and ignore
/// anything after it. The fix belongs here, not in `result_to_string`
/// (other tools rely on its shape) and not in mcp-wire alone (the bot must
/// tolerate any MCP server that returns both text + structuredContent).
fn parse_wire_interfaces(raw: &str) -> Result<Vec<WireInterface>, String> {
    let mut stream = serde_json::Deserializer::from_str(raw).into_iter::<Value>();
    let v: Value = match stream.next() {
        Some(Ok(v)) => v,
        Some(Err(e)) => {
            return Err(format!(
                "Error: wire interfaces output is not JSON ({}): {}",
                e,
                crate::utils::truncate_str(raw, 200)
            ));
        }
        None => {
            return Err(format!(
                "Error: wire interfaces output is not JSON (empty input): {}",
                crate::utils::truncate_str(raw, 200)
            ));
        }
    };
    let arr = v
        .get("interfaces")
        .and_then(|i| i.as_array())
        .or_else(|| v.as_array())
        .ok_or_else(|| "Error: wire interfaces output has no 'interfaces' array.".to_string())?;
    let mut out = Vec::new();
    for it in arr {
        let speed = match it.get("speed_mbps") {
            Some(Value::Number(n)) => {
                let mbps = n.as_u64().unwrap_or(0);
                if mbps > 0 {
                    format!("{} Mbps", mbps)
                } else {
                    String::new()
                }
            }
            Some(Value::String(s)) if !s.is_empty() => s.clone(),
            _ => String::new(),
        };
        let link_label = it
            .get("link_type")
            .and_then(|l| l.get("label"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let addresses: Vec<String> = it
            .get("addresses")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        out.push(WireInterface {
            name: field_as_string(it, "name"),
            description: field_as_string(it, "description"),
            mac: field_as_string(it, "mac"),
            mtu: field_as_string(it, "mtu"),
            operstate: field_as_string(it, "operstate"),
            speed,
            link_label,
            addresses,
        });
    }
    Ok(out)
}

/// Render the network map as a standalone SVG (task 338).
/// One card per R630 interface + a notes footer. No host discovery, no
/// traffic flows — those need wire `capture` (task 341).
fn render_network_map_svg(title: &str, ifaces: &[WireInterface]) -> String {
    let card_h: i32 = 118;
    let header_h: i32 = 76;
    let footer_h: i32 = 88;
    let width: i32 = 920;
    let height = header_h + (ifaces.len() as i32) * card_h + footer_h;
    let mut s = String::new();
    s.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\" font-family=\"sans-serif\">\n",
        width, height, width, height
    ));
    s.push_str("<rect width=\"100%\" height=\"100%\" fill=\"#111827\"/>\n");
    s.push_str(&format!(
        "<text x=\"24\" y=\"34\" font-size=\"22\" font-weight=\"bold\" fill=\"#e5e7eb\">{}</text>\n",
        xml_escape(title)
    ));
    s.push_str("<text x=\"24\" y=\"58\" font-size=\"13\" fill=\"#9ca3af\">R630 (Helix) — interfaces from mcp_wire</text>\n");
    let mut y = header_h;
    for it in ifaces {
        let state_color = if it.operstate.eq_ignore_ascii_case("up") {
            "#34d399"
        } else {
            "#f87171"
        };
        let state_label = if it.operstate.is_empty() {
            "?"
        } else {
            &it.operstate
        };
        let mut meta = Vec::new();
        if !it.mac.is_empty() {
            meta.push(format!("MAC {}", it.mac));
        }
        if !it.mtu.is_empty() {
            meta.push(format!("MTU {}", it.mtu));
        }
        if !it.speed.is_empty() {
            meta.push(it.speed.clone());
        }
        if !it.link_label.is_empty() {
            meta.push(it.link_label.clone());
        }
        let meta_line = if meta.is_empty() {
            "—".to_string()
        } else {
            meta.join("  ·  ")
        };
        let addr_line = if it.addresses.is_empty() {
            "no addresses".to_string()
        } else {
            it.addresses.join(", ")
        };
        let desc = if it.description.is_empty() {
            it.name.clone()
        } else {
            it.description.clone()
        };
        s.push_str(&format!(
            "<rect x=\"24\" y=\"{}\" width=\"872\" height=\"102\" rx=\"10\" fill=\"#1f2937\" stroke=\"#374151\"/>\n",
            y
        ));
        s.push_str(&format!(
            "<circle cx=\"48\" cy=\"{}\" r=\"7\" fill=\"{}\"/>\n",
            y + 26,
            state_color
        ));
        s.push_str(&format!(
            "<text x=\"64\" y=\"{}\" font-size=\"16\" font-weight=\"bold\" fill=\"#e5e7eb\">{}  ({})</text>\n",
            y + 32,
            xml_escape(&it.name),
            xml_escape(state_label)
        ));
        s.push_str(&format!(
            "<text x=\"64\" y=\"{}\" font-size=\"13\" fill=\"#9ca3af\">{}</text>\n",
            y + 56,
            xml_escape(&desc)
        ));
        s.push_str(&format!(
            "<text x=\"64\" y=\"{}\" font-size=\"13\" fill=\"#d1d5db\">{}</text>\n",
            y + 78,
            xml_escape(&meta_line)
        ));
        s.push_str(&format!(
            "<text x=\"64\" y=\"{}\" font-size=\"13\" fill=\"#93c5fd\">{}</text>\n",
            y + 98,
            xml_escape(&addr_line)
        ));
        y += card_h;
    }
    if ifaces.is_empty() {
        s.push_str(&format!(
            "<text x=\"24\" y=\"{}\" font-size=\"14\" fill=\"#f87171\">No interfaces returned by wire.</text>\n",
            header_h + 24
        ));
    }
    let footer_y = header_h + (ifaces.len() as i32) * card_h;
    s.push_str(&format!(
        "<text x=\"24\" y=\"{}\" font-size=\"12\" fill=\"#9ca3af\">Known LAN hosts (from Helix config, not discovered): 192.168.1.149 Ollama server (gpt-oss:20b)  ·  192.168.1.196 desktop (gemma4:e4b)</text>\n",
        footer_y + 24
    ));
    s.push_str(&format!(
        "<text x=\"24\" y=\"{}\" font-size=\"12\" fill=\"#9ca3af\">Interfaces only — other LAN hosts, traffic flows, and live hosts need wire capture (task 341).</text>\n",
        footer_y + 46
    ));
    s.push_str("</svg>\n");
    s
}

/// create_network_map — task 338. Dedicated one-call tool: fetches mcp_wire
/// `interfaces` server-side and renders the SVG itself, because gemma4:e4b
/// is weak at multi-step tool composition (task 334). The LLM does NOT need
/// to call wire + create_artifact separately. Reuses the `create_artifact`
/// writer — no duplicated file-writing code.
pub fn create_network_map(args: &Value) -> String {
    let title = args["title"].as_str().unwrap_or("Network Map");
    let raw = match crate::mcp_client::try_execute("mcp__wire__interfaces", &serde_json::json!({})) {
        Some(r) => r,
        None => {
            return "Error: wire MCP server is not available — is [mcp] enabled and the wire server configured in config.toml?"
                .to_string();
        }
    };
    let ifaces = match parse_wire_interfaces(&raw) {
        Ok(i) => i,
        Err(e) => return e,
    };
    let svg = render_network_map_svg(title, &ifaces);
    match write_artifact_file("svg", title, &svg) {
        Ok((url, kind, bytes)) => artifact_saved_string(&url, &kind, bytes),
        Err(e) => e,
    }
}

/// Render the full network map as a standalone SVG (task 347).
/// Gateway (Starlink router) is the root node at the top, Helix is
/// highlighted as "this machine" on its own row, discovered devices hang
/// below in a 3-per-row grid with IP, MAC, hostname and discovery tags.
fn render_full_network_map_svg(
    title: &str,
    gateway: Option<&str>,
    hosts: &[DiscoveredHost],
) -> String {
    let width: i32 = 920;
    let margin: i32 = 24;
    let full_w: i32 = width - 2 * margin; // 872
    let header_h: i32 = 76;
    let gw_h: i32 = 84;
    let helix_h: i32 = 96;
    let card_w: i32 = 280;
    let card_h: i32 = 118;
    let gap: i32 = 12;
    let footer_h: i32 = 88;
    let cols: i32 = 3;

    let helix = hosts.iter().find(|h| h.is_helix);
    let devices: Vec<&DiscoveredHost> = hosts.iter().filter(|h| !h.is_helix).collect();
    let rows = (devices.len() as i32 + cols - 1) / cols;
    let helix_row_h = if helix.is_some() { helix_h + gap } else { 0 };
    let height = header_h + gw_h + gap + helix_row_h + rows * (card_h + gap) + footer_h;

    let mut s = String::new();
    s.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\" font-family=\"sans-serif\">\n",
        width, height, width, height
    ));
    s.push_str("<rect width=\"100%\" height=\"100%\" fill=\"#111827\"/>\n");
    s.push_str(&format!(
        "<text x=\"24\" y=\"34\" font-size=\"22\" font-weight=\"bold\" fill=\"#e5e7eb\">{}</text>\n",
        xml_escape(title)
    ));
    s.push_str("<text x=\"24\" y=\"58\" font-size=\"13\" fill=\"#9ca3af\">Starlink router → LAN — gateway + discovered devices</text>\n");

    // Gateway root node.
    let mut y = header_h;
    let gw_label = gateway.unwrap_or("not detected");
    let gw_fill = if gateway.is_some() { "#1f2937" } else { "#3f1d1d" };
    s.push_str(&format!(
        "<rect x=\"24\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"10\" fill=\"{}\" stroke=\"#34d399\" stroke-width=\"2\"/>\n",
        y, full_w, gw_h, gw_fill
    ));
    s.push_str(&format!(
        "<text x=\"48\" y=\"{}\" font-size=\"16\" font-weight=\"bold\" fill=\"#e5e7eb\">⬢ Starlink router (gateway) — {}</text>\n",
        y + 32,
        xml_escape(gw_label)
    ));
    s.push_str(&format!(
        "<text x=\"48\" y=\"{}\" font-size=\"13\" fill=\"#9ca3af\">Root node — every LAN device hangs off this router</text>\n",
        y + 58
    ));
    y += gw_h + gap;

    // Helix, emphasized on its own row.
    if let Some(h) = helix {
        s.push_str(&format!(
            "<rect x=\"24\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"10\" fill=\"#1f2937\" stroke=\"#34d399\" stroke-width=\"2\"/>\n",
            y, full_w, helix_h
        ));
        let name = h
            .hostname
            .clone()
            .unwrap_or_else(|| "Helix (R630)".to_string());
        s.push_str(&format!(
            "<text x=\"48\" y=\"{}\" font-size=\"16\" font-weight=\"bold\" fill=\"#e5e7eb\">★ {} — this machine</text>\n",
            y + 30,
            xml_escape(&name)
        ));
        let mac = h.mac.clone().unwrap_or_else(|| "no MAC".to_string());
        s.push_str(&format!(
            "<text x=\"48\" y=\"{}\" font-size=\"13\" fill=\"#d1d5db\">{}  ·  {}</text>\n",
            y + 54,
            xml_escape(&h.ip),
            xml_escape(&mac)
        ));
        let tags = h
            .methods
            .iter()
            .map(|m| m.tag())
            .collect::<Vec<_>>()
            .join(" · ");
        s.push_str(&format!(
            "<text x=\"48\" y=\"{}\" font-size=\"13\" fill=\"#9ca3af\">{}</text>\n",
            y + 78,
            xml_escape(&tags)
        ));
        y += helix_h + gap;
    }

    // Device grid.
    for (i, h) in devices.iter().enumerate() {
        let col = (i as i32) % cols;
        let row = (i as i32) / cols;
        let x = margin + col * (card_w + gap);
        let cy = y + row * (card_h + gap);
        s.push_str(&format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"10\" fill=\"#1f2937\" stroke=\"#374151\"/>\n",
            x, cy, card_w, card_h
        ));
        let name = h.hostname.clone().unwrap_or_else(|| h.ip.clone());
        s.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" font-size=\"15\" font-weight=\"bold\" fill=\"#e5e7eb\">{}</text>\n",
            x + 16,
            cy + 28,
            xml_escape(&name)
        ));
        s.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" font-size=\"13\" fill=\"#93c5fd\">{}</text>\n",
            x + 16,
            cy + 50,
            xml_escape(&h.ip)
        ));
        let mac = h.mac.clone().unwrap_or_else(|| "no MAC".to_string());
        s.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" font-size=\"12\" fill=\"#d1d5db\">{}</text>\n",
            x + 16,
            cy + 72,
            xml_escape(&mac)
        ));
        let tags = h
            .methods
            .iter()
            .map(|m| m.tag())
            .collect::<Vec<_>>()
            .join(" · ");
        s.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" font-size=\"12\" fill=\"#9ca3af\">{}</text>\n",
            x + 16,
            cy + 94,
            xml_escape(&tags)
        ));
    }
    y += rows * (card_h + gap);

    if hosts.is_empty() {
        s.push_str(&format!(
            "<text x=\"24\" y=\"{}\" font-size=\"14\" fill=\"#f87171\">No devices discovered — nothing answered ARP or the ping sweep.</text>\n",
            y + 24
        ));
        y += 28;
    }

    // Footer limits.
    s.push_str(&format!(
        "<text x=\"24\" y=\"{}\" font-size=\"12\" fill=\"#9ca3af\">Discovery: ARP neighbours + ICMP ping sweep of the gateway /24 (+ known-config hosts). Silent hosts (no ping answer, not in ARP) are invisible.</text>\n",
        y + 24
    ));
    s.push_str(&format!(
        "<text x=\"24\" y=\"{}\" font-size=\"12\" fill=\"#9ca3af\">Hosts on other subnets/VLANs are not shown. The Starlink dish is upstream of the router — a note only, not a discovered node.</text>\n",
        y + 46
    ));
    s.push_str("</svg>\n");
    s
}

/// Helix's own LAN IPs, from wire interfaces (best effort). Used to mark
/// "this machine" on the full map. Falls back to the gateway subnet's
/// .168 (the R630's Helix address) when wire is unavailable.
fn helix_own_ips(gateway_base: &str) -> Vec<String> {
    if let Some(raw) =
        crate::mcp_client::try_execute("mcp__wire__interfaces", &serde_json::json!({}))
    {
        if let Ok(ifaces) = parse_wire_interfaces(&raw) {
            let mut ips: Vec<String> = ifaces
                .iter()
                .flat_map(|i| {
                    i.addresses
                        .iter()
                        .map(|a| a.split('/').next().unwrap_or("").to_string())
                })
                .filter(|a| !a.is_empty() && !a.starts_with("127.") && !a.starts_with("::"))
                .collect();
            ips.sort();
            ips.dedup();
            if !ips.is_empty() {
                return ips;
            }
        }
    }
    if gateway_base == "192.168.1" {
        vec!["192.168.1.168".to_string()]
    } else {
        Vec::new()
    }
}

/// create_full_network_map — task 347. Full-network map: gateway
/// (Starlink router) at the top, Helix highlighted, discovered devices
/// below. Discovery: gateway + ARP (344) -> ping sweep (345) ->
/// hostnames (346) -> merge (dedupe by IP) -> hierarchical SVG.
/// The sweep runs only when this tool is called, never on a timer.
/// Keeps `create_network_map` (interfaces-only) as-is.
pub fn create_full_network_map(args: &Value) -> String {
    let title = args["title"].as_str().unwrap_or("Full Network Map");
    let disc = discover();
    let base = disc
        .gateway
        .as_deref()
        .and_then(subnet_base)
        .unwrap_or_else(|| "192.168.1".to_string());
    let ping_live = ping_sweep_subnet(&base);
    let mut to_resolve: Vec<String> = disc
        .arp_entries
        .iter()
        .map(|e| e.ip.clone())
        .collect();
    to_resolve.extend(ping_live.iter().cloned());
    to_resolve.extend(KNOWN_CONFIG_HOSTS.iter().map(|c| c.ip.to_string()));
    to_resolve.sort();
    to_resolve.dedup();
    let hostnames = resolve_hostnames(&to_resolve);
    let helix_ips = helix_own_ips(&base);
    let hosts = aggregate_hosts(
        disc.gateway.as_deref(),
        &disc.arp_entries,
        &ping_live,
        &hostnames,
        &helix_ips,
    );
    let svg = render_full_network_map_svg(title, disc.gateway.as_deref(), &hosts);
    match write_artifact_file("svg", title, &svg) {
        Ok((url, kind, bytes)) => artifact_saved_string(&url, &kind, bytes),
        Err(e) => e,
    }
}

// ── NEW: Repo Explorer Tools (read-only) ─────────────────────────────────────

/// tool_glob: list files matching a glob pattern (e.g. "**/*.rs").
pub fn repo_glob(args: &Value) -> String {
    let pattern = args["pattern"].as_str().unwrap_or("**/*");
    // We reuse the existing glob_search helper from the crate root tools.
    // For now we just echo what would be searched (real impl would call glob_search).
    format!("[repo_glob] pattern = {}", pattern)
}

/// tool_read with optional line range support.
pub fn repo_read(args: &Value) -> String {
    let path = args["path"].as_str().unwrap_or("");
    let start = args["start_line"].as_u64().map(|v| v as usize);
    let end = args["end_line"].as_u64().map(|v| v as usize);

    if path.is_empty() {
        return "Error: 'path' is required for repo_read".to_string();
    }

    // Constrain to the repository root (canonicalized): blocks `..` escapes
    // (e.g. `../../.ssh/id_rsa`), absolute paths outside the repo, and
    // symlink escapes. Replaces the old absolute-path-only check.
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let canon = match resolve_inside(&root, path) {
        Ok(p) => p,
        Err(e) => return e,
    };

    match fs::read_to_string(&canon) {
        Ok(content) => {
            let lines: Vec<&str> = content.lines().collect();
            let total = lines.len();

            let s = start.unwrap_or(1).saturating_sub(1);
            let e = end.unwrap_or(total).min(total);

            if s >= e || s >= total {
                return format!("File '{}' has {} lines. Requested range out of bounds.", path, total);
            }

            let slice = &lines[s..e];
            let header = if start.is_some() || end.is_some() {
                format!("[{}:{}] ", s + 1, e)
            } else {
                String::new()
            };
            format!("{}{}", header, slice.join("\n"))
        }
        Err(e) => format!("Error reading '{}': {}", path, e),
    }
}

/// tool_grep: search for a pattern inside files (simple line grep).
pub fn repo_grep(args: &Value) -> String {
    let pattern = args["pattern"].as_str().unwrap_or("");
    let path = args["path"].as_str().unwrap_or(".");

    if pattern.is_empty() {
        return "Error: 'pattern' is required for repo_grep".to_string();
    }

    // Very simple implementation – in real code we would use the search_file_content helper.
    // Here we just return a placeholder that the explorer can consume.
    format!("[repo_grep] pattern='{}' path='{}' (implementation pending real search)", pattern, path)
}

// ── OKF / Cross-Machine Project Mapping Tools ────────────────────────────────

/// Scans a directory (recursively, limited depth) and returns a structured snapshot.
/// Useful for feeding into Mermaid diagram generation or OKF knowledge bundles.
///
/// Args:
///   path: base directory (relative or absolute — be careful with absolute)
///   max_depth: optional, default 4
///   include_hidden: optional, default false
pub fn scan_directory(args: &Value) -> String {
    let base_path = args["path"].as_str().unwrap_or(".");
    let max_depth = args["max_depth"].as_u64().unwrap_or(4) as usize;
    let include_hidden = args["include_hidden"].as_bool().unwrap_or(false);

    let mut tree = Vec::new();
    let mut file_count = 0usize;
    let mut dir_count = 0usize;

    fn walk(
        dir: &std::path::Path,
        current_depth: usize,
        max_depth: usize,
        include_hidden: bool,
        tree: &mut Vec<serde_json::Value>,
        file_count: &mut usize,
        dir_count: &mut usize,
    ) {
        if current_depth > max_depth {
            return;
        }

        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();

                if !include_hidden && name.starts_with('.') {
                    continue;
                }

                let is_dir = path.is_dir();
                let rel_path = path.strip_prefix(dir).unwrap_or(&path).to_string_lossy().to_string();

                if is_dir {
                    *dir_count += 1;
                    let mut children = Vec::new();
                    walk(&path, current_depth + 1, max_depth, include_hidden, &mut children, file_count, dir_count);

                    tree.push(serde_json::json!({
                        "type": "directory",
                        "name": name,
                        "path": rel_path,
                        "children": children
                    }));
                } else {
                    *file_count += 1;
                    tree.push(serde_json::json!({
                        "type": "file",
                        "name": name,
                        "path": rel_path,
                        "size": entry.metadata().map(|m| m.len()).unwrap_or(0)
                    }));
                }
            }
        }
    }

    let start = std::path::Path::new(base_path);
    walk(start, 0, max_depth, include_hidden, &mut tree, &mut file_count, &mut dir_count);

    let result = serde_json::json!({
        "base_path": base_path,
        "scanned_at": chrono::Utc::now().to_rfc3339(),
        "max_depth": max_depth,
        "file_count": file_count,
        "dir_count": dir_count,
        "tree": tree
    });

    serde_json::to_string_pretty(&result).unwrap_or_else(|_| "Failed to serialize scan result".to_string())
}

/// Generates a Mermaid flowchart from a directory scan.
/// This is the key piece for your "file changes → Mermaid maps → OKF server" workflow.
///
/// Args:
///   path: directory to scan
///   title: optional diagram title
///   max_depth: optional
///   diagram_style: "tree" | "changes" | "hybrid" (default "tree")
pub fn generate_mermaid_project_map(args: &Value) -> String {
    let base_path = args["path"].as_str().unwrap_or(".");
    let title = args["title"].as_str().unwrap_or("Project Structure");
    let max_depth = args["max_depth"].as_u64().unwrap_or(3) as usize;
    let style = args["diagram_style"].as_str().unwrap_or("tree");

    // First get the scan data
    let scan_json = scan_directory(&serde_json::json!({
        "path": base_path,
        "max_depth": max_depth,
        "include_hidden": false
    }));

    let scan: serde_json::Value = serde_json::from_str(&scan_json).unwrap_or(serde_json::json!({}));
    let tree = scan.get("tree").and_then(|t| t.as_array()).cloned().unwrap_or_default();

    let mut mermaid = format!("```mermaid\ngraph TD\n    subgraph \"{}\" [{}]\n", title, base_path);

    let mut node_id = 0u32;
    let mut id_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    fn add_node(
        node: &serde_json::Value,
        parent_id: Option<&str>,
        mermaid: &mut String,
        node_id: &mut u32,
        id_map: &mut std::collections::HashMap<String, String>,
    ) {
        let name = node["name"].as_str().unwrap_or("?");
        let path = node["path"].as_str().unwrap_or(name);
        let typ = node["type"].as_str().unwrap_or("file");

        *node_id += 1;
        let nid = format!("n{}", node_id);
        id_map.insert(path.to_string(), nid.clone());

        let label = if typ == "directory" {
            format!("📁 {}", name)
        } else {
            format!("📄 {}", name)
        };

        mermaid.push_str(&format!("        {}[\"{}\"]\n", nid, label));

        if let Some(pid) = parent_id {
            mermaid.push_str(&format!("        {} --> {}\n", pid, nid));
        }

        if typ == "directory" {
            if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
                for child in children {
                    add_node(child, Some(&nid), mermaid, node_id, id_map);
                }
            }
        }
    }

    // Root node
    mermaid.push_str(&format!("        root[\"{}\"]\n", base_path));

    for item in &tree {
        add_node(item, Some("root"), &mut mermaid, &mut node_id, &mut id_map);
    }

    mermaid.push_str("    end\n");

    // Add styling
    mermaid.push_str("\n    classDef dir fill:#e3f2fd,stroke:#1565c0\n");
    mermaid.push_str("    classDef file fill:#f1f8e9,stroke:#2e7d32\n");

    mermaid.push_str("```");

    // Build rich OKF-style knowledge payload
    let knowledge_payload = serde_json::json!({
        "type": "okf_knowledge",
        "content_type": "mermaid",
        "id": format!("project-map-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S")),
        "title": title,
        "diagram_type": style,
        "source_machine": std::env::var("COMPUTERNAME").unwrap_or_else(|_| "unknown-pc".to_string()),
        "base_path": base_path,
        "detected_at": chrono::Utc::now().to_rfc3339(),
        "mermaid": mermaid,
        "scan_summary": {
            "files": scan["file_count"],
            "dirs": scan["dir_count"],
            "depth": max_depth
        },
        "metadata": {
            "generated_by": "helix-file-tools",
            "purpose": "cross-machine project mapping for OKF"
        }
    });

    serde_json::to_string_pretty(&knowledge_payload).unwrap_or_else(|_| mermaid)
}

/// Convenience wrapper: produce a ready-to-send OKF knowledge entry for a project map.
/// You can POST the output of this directly (or the mermaid field) to your central OKF server.
pub fn generate_okf_project_map_knowledge(args: &Value) -> String {
    let map = generate_mermaid_project_map(args);
    format!(
        "✅ Generated OKF-ready project map knowledge.\n\n\
         You can now send this to your OKF server (Dell 630 / Proxmox) using:\n\
         POST /okf/manifest  or  /okf/push\n\n\
         Content:\n{}",
        map
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::network_discovery::DiscoveryMethod;
    use serde_json::json;

    /// Scope guard: runs cleanup even if the test panics, so probe files and
    /// symlinks never leak into the workspace on a failed assertion.
    struct Cleanup<F: FnOnce()> {
        f: Option<F>,
    }
    impl<F: FnOnce()> Cleanup<F> {
        fn new(f: F) -> Self {
            Self { f: Some(f) }
        }
    }
    impl<F: FnOnce()> Drop for Cleanup<F> {
        fn drop(&mut self) {
            if let Some(f) = self.f.take() {
                f();
            }
        }
    }

    #[test]
    fn test_repo_read_basic() {
        // This test assumes Cargo.toml exists at project root.
        let result = repo_read(&json!({"path": "Cargo.toml"}));
        assert!(result.contains("[package]"), "expected Cargo.toml content, got: {}", result);
    }

    #[test]
    fn test_repo_glob_placeholder() {
        let result = repo_glob(&json!({"pattern": "**/*.rs"}));
        assert!(result.contains("**/*.rs"));
    }

    // ── path traversal (task 185) ───────────────────────────────────────────

    #[test]
    fn test_read_log_rejects_dotdot_traversal() {
        // logs/../example.config.toml passes the old starts_with("logs/") check and
        // resolves to an existing file outside logs/ — must be rejected.
        // (Was config.toml before PR #25 renamed it to example.config.toml.)
        let result = read_log(&json!({"log_file": "logs/../example.config.toml"}));
        assert!(
            result.contains("escapes the allowed directory"),
            "traversal must be rejected, got: {}",
            result
        );
    }

    #[test]
    fn test_read_log_rejects_non_logs_prefix() {
        let result = read_log(&json!({"log_file": "/etc/passwd"}));
        assert!(result.contains("Security error"), "got: {}", result);
    }

    #[test]
    fn test_read_log_rejects_symlink_escape() {
        // A symlink inside logs/ pointing outside must not be followed.
        // Unix-only; skipped elsewhere.
        #[cfg(unix)]
        {
            let link = std::path::PathBuf::from("logs/_test_evil_link_do_not_use");
            let _ = std::fs::remove_file(&link);
            if std::os::unix::fs::symlink("../example.config.toml", &link).is_err() {
                return; // cannot create symlinks here — skip
            }
            // Panic-proof: the symlink is removed even if the assert below fails.
            let _cleanup =
                Cleanup::new(|| _ = std::fs::remove_file("logs/_test_evil_link_do_not_use"));
            let result = read_log(&json!({"log_file": "logs/_test_evil_link_do_not_use"}));
            assert!(
                result.contains("escapes the allowed directory"),
                "symlink escape must be rejected, got: {}",
                result
            );
        }
    }

    #[test]
    fn test_repo_read_rejects_parent_escape() {
        // Plant a probe file just outside the repo root; `../<probe>` must be
        // rejected even though the target exists (old code would read it).
        // Panic-proof: the probe is removed even if the assert below fails.
        let probe = std::path::PathBuf::from("../_test_escape_probe_xyz.txt");
        std::fs::write(&probe, "probe").ok();
        let _cleanup = Cleanup::new(|| _ = std::fs::remove_file("../_test_escape_probe_xyz.txt"));
        let result = repo_read(&json!({"path": "../_test_escape_probe_xyz.txt"}));
        assert!(
            result.contains("escapes the allowed directory"),
            "escape must be rejected, got: {}",
            result
        );
    }

    #[test]
    fn test_repo_read_rejects_absolute_outside_repo() {
        let result = repo_read(&json!({"path": "/etc/passwd"}));
        assert!(
            result.contains("escapes the allowed directory"),
            "absolute escape must be rejected, got: {}",
            result
        );
    }

    #[test]
    fn test_repo_read_missing_is_error_not_panic() {
        let result = repo_read(&json!({"path": "no_such_file_xyz.rs"}));
        assert!(result.contains("Error reading"), "got: {}", result);
    }

    // ── Artifact tools (tasks 335, 338) ─────────────────────────────────────

    /// Extract the `/artifacts/<file>` path from a create_artifact success
    /// string. None unless the string matches the success shape.
    fn artifact_path_from_result(result: &str) -> Option<String> {
        let rest = result.strip_prefix("✅ Artifact saved: ")?;
        let end = rest.find(" (")?;
        Some(rest[..end].to_string())
    }

    /// Remove a created artifact file. Deliberately does NOT remove the
    /// `artifacts/` directory: parallel artifact tests share it, and a
    /// `remove_dir` here races with another test's write — the directory
    /// vanishing between its `create_dir_all` and its `fs::write` fails
    /// that write with ENOENT ("No such file or directory"). The empty
    /// dir is gitignored and harmless (same as `logs/`).
    /// Tests must not leak files into the worktree.
    fn remove_artifact(rel_url: &str) {
        let path = rel_url.trim_start_matches('/');
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn create_artifact_writes_file_and_returns_url() {
        let content = "<html><body><h1>test</h1></body></html>";
        let result =
            create_artifact(&json!({"kind": "html", "title": "Test Chart", "content": content}));
        let url =
            artifact_path_from_result(&result).expect(&format!("expected success, got: {}", result));
        // sanitise_filename preserves case (spaces -> underscores).
        assert!(url.starts_with("/artifacts/Test_Chart-"), "url: {}", url);
        assert!(url.ends_with(".html"), "url: {}", url);
        assert!(result.contains("(html,"), "result: {}", result);
        let _cleanup = Cleanup::new(|| remove_artifact(&url));
        let on_disk =
            std::fs::read_to_string(url.trim_start_matches('/')).expect("artifact file must exist");
        assert_eq!(on_disk, content);
    }

    #[test]
    fn create_artifact_rejects_unknown_kind() {
        let result = create_artifact(&json!({"kind": "pdf", "title": "x", "content": "hi"}));
        assert!(result.contains("unknown artifact kind"), "got: {}", result);
        assert!(!result.contains("Artifact saved"), "got: {}", result);
    }

    #[test]
    fn create_artifact_rejects_oversize_content() {
        let big = "x".repeat(MAX_ARTIFACT_BYTES + 1);
        let result = create_artifact(&json!({"kind": "text", "title": "big", "content": big}));
        assert!(result.contains("over the 512 KB cap"), "got: {}", result);
    }

    #[test]
    fn create_artifact_rejects_empty_content() {
        let result = create_artifact(&json!({"kind": "text", "title": "empty", "content": ""}));
        assert!(result.contains("empty"), "got: {}", result);
    }

    #[test]
    fn create_artifact_sanitises_path_traversal_title() {
        let result =
            create_artifact(&json!({"kind": "text", "title": "../../etc/passwd", "content": "pwn"}));
        let url =
            artifact_path_from_result(&result).expect(&format!("expected success, got: {}", result));
        assert!(!url.contains(".."), "url must not contain ..: {}", url);
        assert!(
            !url["/artifacts/".len()..].contains('/'),
            "flat filename: {}",
            url
        );
        let _cleanup = Cleanup::new(|| remove_artifact(&url));
        assert!(std::path::Path::new(url.trim_start_matches('/')).exists());
    }

    #[test]
    fn create_artifact_second_same_minute_does_not_overwrite() {
        let r1 = create_artifact(&json!({"kind": "text", "title": "Dup Title", "content": "one"}));
        let u1 = artifact_path_from_result(&r1).expect(&r1);
        let _c1 = Cleanup::new(|| remove_artifact(&u1));
        let r2 = create_artifact(&json!({"kind": "text", "title": "Dup Title", "content": "two"}));
        let u2 = artifact_path_from_result(&r2).expect(&r2);
        let _c2 = Cleanup::new(|| remove_artifact(&u2));
        assert_ne!(u1, u2, "second write must get a distinct filename");
        assert_eq!(
            std::fs::read_to_string(u1.trim_start_matches('/')).unwrap(),
            "one"
        );
        assert_eq!(
            std::fs::read_to_string(u2.trim_start_matches('/')).unwrap(),
            "two"
        );
    }

    const WIRE_INTERFACES_STUB: &str = r#"{"interfaces": [
        {"name": "ens18", "description": "Ethernet", "mac": "aa:bb:cc:dd:ee:ff", "mtu": 1500, "operstate": "up", "speed_mbps": 1000, "link_type": {"arphrd": 1, "label": "Ethernet"}, "addresses": ["192.168.1.168/24"], "link_layer_types": ["ethernet"]},
        {"name": "tailscale0", "description": "Tailscale", "mac": "", "mtu": 1280, "operstate": "up", "speed_mbps": 0, "link_type": {"arphrd": 65534, "label": "None"}, "addresses": ["100.84.56.110/32"], "link_layer_types": []},
        {"name": "lo", "description": "Loopback", "mac": "", "mtu": 65536, "operstate": "up", "speed_mbps": 0, "link_type": {"arphrd": 772, "label": "Loopback"}, "addresses": ["127.0.0.1/8", "::1/128"], "link_layer_types": []}
    ]}"#;

    #[test]
    fn create_network_map_renders_wire_interfaces() {
        let _tl = crate::tools::subagent_tool::test_env_lock();
        crate::mcp_client::stub_try_execute_for_tests("mcp__wire__interfaces", WIRE_INTERFACES_STUB);
        let _cleanup_overrides = Cleanup::new(crate::mcp_client::clear_test_overrides);
        let result = create_network_map(&json!({"title": "Test Net Map"}));
        let url =
            artifact_path_from_result(&result).expect(&format!("expected success, got: {}", result));
        assert!(url.ends_with(".svg"), "url: {}", url);
        let _cleanup = Cleanup::new(|| remove_artifact(&url));
        let svg =
            std::fs::read_to_string(url.trim_start_matches('/')).expect("svg must exist");
        assert!(svg.contains("ens18"), "svg must name ens18");
        assert!(svg.contains("tailscale0"), "svg must name tailscale0");
        assert!(svg.contains("192.168.1.168/24"), "svg must show the address");
        assert!(
            svg.contains("100.84.56.110/32"),
            "svg must show the tailscale address"
        );
        assert!(svg.starts_with("<svg"), "must be an SVG document");
    }

    #[test]
    fn create_network_map_tolerates_doubled_mcp_payload() {
        // Task 343 regression: live, `result_to_string` returns the
        // pretty-printed structuredContent + "\n" + the compact text block
        // — two JSON documents back to back. A strict `from_str` fails on
        // the trailing second document; the parser must use the first and
        // ignore the rest. This test FAILS on the old strict parse.
        let _tl = crate::tools::subagent_tool::test_env_lock();
        let value: Value =
            serde_json::from_str(WIRE_INTERFACES_STUB).expect("stub must be valid JSON");
        let pretty = serde_json::to_string_pretty(&value).expect("pretty-print must succeed");
        let doubled = format!("{}\n{}", pretty, WIRE_INTERFACES_STUB);
        // Direct parser check on the doubled payload.
        let ifaces = parse_wire_interfaces(&doubled).expect("doubled payload must parse");
        assert_eq!(ifaces.len(), 3, "all three stub interfaces must parse");
        assert_eq!(ifaces[0].name, "ens18");
        // End-to-end through the tool, as the live path runs it.
        crate::mcp_client::stub_try_execute_for_tests("mcp__wire__interfaces", &doubled);
        let _cleanup_overrides = Cleanup::new(crate::mcp_client::clear_test_overrides);
        let result = create_network_map(&json!({"title": "Test Doubled Map"}));
        let url =
            artifact_path_from_result(&result).expect(&format!("expected success, got: {}", result));
        assert!(url.ends_with(".svg"), "url: {}", url);
        let _cleanup = Cleanup::new(|| remove_artifact(&url));
        let svg =
            std::fs::read_to_string(url.trim_start_matches('/')).expect("svg must exist");
        assert!(svg.contains("ens18"), "svg must name ens18");
        assert!(svg.contains("192.168.1.168/24"), "svg must show the address");
    }

    #[test]
    fn create_network_map_reports_wire_unavailable() {
        let _tl = crate::tools::subagent_tool::test_env_lock();
        crate::mcp_client::override_config_for_tests(crate::config::mcp::McpConfig::default());
        let _cleanup_overrides = Cleanup::new(crate::mcp_client::clear_test_overrides);
        let result = create_network_map(&json!({}));
        assert!(
            result.contains("wire MCP server is not available"),
            "got: {}",
            result
        );
    }

    #[test]
    fn render_full_network_map_shows_gateway_helix_and_tags() {
        // Task 347.4: canned discovery input — gateway is the root node,
        // Helix is marked, every device shows IP/MAC/tag, known-config
        // hosts appear with the 'configured' tag.
        let hosts = vec![
            DiscoveredHost {
                ip: "192.168.1.168".to_string(),
                mac: Some("aa:bb:cc:dd:ee:ff".to_string()),
                interface: Some("ens18".to_string()),
                hostname: Some("bot".to_string()),
                methods: vec![DiscoveryMethod::Ping],
                is_helix: true,
            },
            DiscoveredHost {
                ip: "192.168.1.149".to_string(),
                mac: None,
                interface: None,
                hostname: Some("Ollama server (gpt-oss:20b)".to_string()),
                methods: vec![DiscoveryMethod::Configured],
                is_helix: false,
            },
            DiscoveredHost {
                ip: "192.168.1.196".to_string(),
                mac: Some("aa:bb:cc:dd:ee:02".to_string()),
                interface: Some("ens18".to_string()),
                hostname: Some("desktop".to_string()),
                methods: vec![
                    DiscoveryMethod::Arp,
                    DiscoveryMethod::Ping,
                    DiscoveryMethod::Configured,
                ],
                is_helix: false,
            },
        ];
        let svg = render_full_network_map_svg("Full Network Map", Some("192.168.1.1"), &hosts);
        assert!(svg.starts_with("<svg"), "must be an SVG document");
        assert!(
            svg.contains("Starlink router (gateway) — 192.168.1.1"),
            "gateway must be the root node"
        );
        assert!(svg.contains("this machine"), "Helix must be marked");
        assert!(svg.contains("192.168.1.168"), "Helix IP must show");
        assert!(
            svg.contains("192.168.1.149"),
            "configured host must show even when silent"
        );
        assert!(svg.contains("configured"), "configured tag must show");
        assert!(svg.contains("192.168.1.196"), "desktop must show");
        assert!(svg.contains("desktop"), "hostname must show");
        assert!(
            svg.contains("arp · ping · configured"),
            "merged tags must show"
        );
        assert!(
            svg.contains("Starlink dish is upstream"),
            "limits footer must state the dish note"
        );
    }

    #[test]
    fn render_full_network_map_escapes_xml_and_handles_no_gateway() {
        let hosts = vec![DiscoveredHost {
            ip: "192.168.1.50".to_string(),
            mac: None,
            interface: None,
            hostname: Some("<script>alert(1)</script>".to_string()),
            methods: vec![DiscoveryMethod::Ping],
            is_helix: false,
        }];
        let svg = render_full_network_map_svg("Map <&>", None, &hosts);
        assert!(!svg.contains("<script>"), "raw script tag must not appear");
        assert!(svg.contains("&lt;script&gt;"), "must be escaped");
        assert!(svg.contains("not detected"), "missing gateway must be stated");
    }

    #[test]
    fn render_network_map_svg_escapes_xml() {
        let ifaces = vec![WireInterface {
            name: "eth0".to_string(),
            description: "<script>alert(1)</script> & friends".to_string(),
            mac: "aa:bb".to_string(),
            mtu: "1500".to_string(),
            operstate: "up".to_string(),
            speed: "100 Mbps".to_string(),
            link_label: "Ethernet".to_string(),
            addresses: vec!["10.0.0.1/24".to_string()],
        }];
        let svg = render_network_map_svg("Map <&>", &ifaces);
        assert!(!svg.contains("<script>"), "raw script tag must not appear");
        assert!(svg.contains("&lt;script&gt;"), "must be escaped");
        assert!(svg.contains("Map &lt;&amp;&gt;"), "title must be escaped");
    }
}