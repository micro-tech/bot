use crate::bus::{Bus, Message};
use crate::config::okf::OkfConfig;
use crate::okf::{OkfLibrarian, api as okf_api};
use axum::{
    Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message as WsMessage, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    middleware,
    response::{Html, IntoResponse, Json},
    routing::{get, post},
};
use axum_server::tls_rustls::RustlsConfig;
use futures_util::{SinkExt, StreamExt};
use log::info;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
// use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;
use tokio::sync::broadcast;
use tokio::sync::RwLock;
use toml;

// ── AppState ──────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    bus: Arc<Bus>,
    msg_tx: broadcast::Sender<String>,
    config_str: String,
    /// Path of the config file Helix actually loaded at startup (resolved by
    /// main.rs from ./config.toml, /etc/helix/config.toml,
    /// /usr/local/etc/helix/config.toml, first-exists-wins). The web editor
    /// must read and write THIS file — not a hardcoded "config.toml" — or
    /// saves land in the wrong place whenever Helix runs off /etc/helix.
    config_path: String,
    backends_json: String, // JSON array of {id, label, kind}
    logging: LoggingConfig,
    /// Effective web auth token (config `[web] auth_token` or `HELIX_WEB_TOKEN`
    /// env var). Empty = no token configured: mutating endpoints stay open and
    /// a loud warning is logged at startup.
    auth_token: String,
    /// Named routine registry (task 201). Shared with the cron scheduler task
    /// spawned in main.rs — the WS routines API and the scheduler operate on
    /// the same registry, so toggles take effect without a restart.
    routines: Arc<RwLock<crate::cron::registry::RoutineRegistry>>,
    /// In-flight chat requests (task 230): correlation_id -> (backend_id,
    /// sent_at_ms). Inserted when a chat_request leaves the web UI, removed
    /// when the matching llm_output/ollama_response crosses the bridge (or by
    /// the 5-minute TTL sweep if the backend never answers). Drives the
    /// per-backend working/idle indicator in the agent_status snapshot.
    inflight: Arc<Mutex<HashMap<u64, (String, u64)>>>,
}

// ── Config structs ────────────────────────────────────────────────────────────
// These are a parse-schema mirror of the canonical config (see crate::config).
// The web server re-parses the raw TOML string to extract only the fields it
// needs; the rest exist so serde accepts the full tables. Canonical config
// access goes through main.rs — do not treat these as a second source of truth.

#[derive(Deserialize)]
struct Config {
    // Schema-only: the web server never reads these tables itself.
    #[allow(dead_code)]
    helix: HelixConfig,
    ollama: Vec<OllamaConfig>,
    web: WebConfig,
    #[allow(dead_code)]
    heartbeat: HeartbeatConfig,
    #[serde(default)]
    logging: LoggingConfig,
}

#[derive(Deserialize)]
#[allow(dead_code)] // schema-only: name is displayed nowhere in the web UI
struct HelixConfig {
    name: String,
}

#[derive(Deserialize)]
struct OllamaConfig {
    name: String,
    // Schema-only: the web UI lists backends by name; url/model are
    // consumed by the agent core, not the web server.
    #[allow(dead_code)]
    url: String,
    #[allow(dead_code)]
    model: String,
}

#[derive(Deserialize)]
struct WebConfig {
    // Schema-only: the bind port arrives via the start_web_server() argument,
    // resolved by main.rs from the live config file — not from this parse.
    #[allow(dead_code)]
    port: u16,
    #[serde(default)]
    tls_enabled: bool,
    #[serde(default = "default_cert_path")]
    cert_path: String,
    #[serde(default = "default_key_path")]
    key_path: String,
    /// Address to bind. Defaults to loopback-only; set to "0.0.0.0" (and set an
    /// auth token!) only if the UI must be reachable from the LAN.
    #[serde(default = "default_bind")]
    bind: String,
    /// Shared token required for mutating web endpoints (config/manifest save,
    /// log clears, OKF reload/push). `HELIX_WEB_TOKEN` env var overrides this.
    /// Empty = no auth (startup warns loudly).
    #[serde(default)]
    auth_token: String,
}

fn default_bind() -> String {
    "127.0.0.1".to_string()
}

fn default_cert_path() -> String {
    "cert.pem".to_string()
}
fn default_key_path() -> String {
    "key.pem".to_string()
}

#[derive(Deserialize)]
#[allow(dead_code)] // schema-only: the heartbeat interval is consumed by the
                    // agent core's scheduler, not the web server
struct HeartbeatConfig {
    interval_seconds: u64,
}

#[derive(Deserialize, Clone)]
struct LoggingConfig {
    #[serde(default = "default_chat_log")]
    chat_log: String,
    #[serde(default = "default_error_log")]
    error_log: String,
    #[serde(default = "default_bus_log")]
    bus_log: String,
    #[serde(default = "default_hartbeat_log")]
    hartbeat_log: String,
}

fn default_chat_log() -> String { "logs/chat_log.md".to_string() }
fn default_error_log() -> String { "logs/error_log.md".to_string() }
fn default_bus_log() -> String { "logs/bus_log.md".to_string() }
fn default_hartbeat_log() -> String { "logs/hartbeat_log.md".to_string() }

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            chat_log: default_chat_log(),
            error_log: default_error_log(),
            bus_log: default_bus_log(),
            hartbeat_log: default_hartbeat_log(),
        }
    }
}

// ── Server entry-point ────────────────────────────────────────────────────────

pub async fn start_web_server(
    bus: Arc<Bus>,
    port: u16,
    config_str: String,
    config_path: String,
    routines: Arc<RwLock<crate::cron::registry::RoutineRegistry>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // main.rs passes "none" when no config file exists anywhere; fall back to
    // the CWD-relative config.toml (previous behavior) rather than writing to
    // a file literally named "none".
    let config_path = if config_path == "none" {
        "config.toml".to_string()
    } else {
        config_path
    };
    // Parse config (with defaults)
    let parsed_cfg: Config = toml::from_str(&config_str).unwrap_or_else(|_| Config {
        helix: HelixConfig {
            name: "Helix".to_string(),
        },
        ollama: vec![],
        web: WebConfig {
            port: 8443,
            tls_enabled: true,
            cert_path: default_cert_path(),
            key_path: default_key_path(),
            bind: default_bind(),
            auth_token: String::new(),
        },
        heartbeat: HeartbeatConfig {
            interval_seconds: 300,
        },
        logging: LoggingConfig::default(),
    });

    let tls_enabled = parsed_cfg.web.tls_enabled;
    let cert_path = &parsed_cfg.web.cert_path;
    let key_path = &parsed_cfg.web.key_path;

    // Effective web auth token: HELIX_WEB_TOKEN env var wins over config file.
    // Empty = no token configured (mutating endpoints stay open; warn loudly).
    let auth_token = std::env::var("HELIX_WEB_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| parsed_cfg.web.auth_token.clone());
    if auth_token.is_empty() {
        log::warn!(
            "No web auth token configured ([web] auth_token or HELIX_WEB_TOKEN env var). \
             Mutating web endpoints (config/manifest save, log clears, OKF reload/push) \
             are unauthenticated — anyone able to reach this server can rewrite them."
        );
    }

    // Bind address: loopback-only by default. Parse defensively.
    let bind_ip: std::net::IpAddr = parsed_cfg.web.bind.parse().unwrap_or_else(|_| {
        log::warn!(
            "Invalid [web] bind address '{}'; falling back to 127.0.0.1",
            parsed_cfg.web.bind
        );
        std::net::IpAddr::from([127, 0, 0, 1])
    });
    if !bind_ip.is_loopback() && auth_token.is_empty() {
        log::warn!(
            "Web server bound to non-loopback address {} with no auth token — \
             LAN clients can rewrite config/manifest. Set [web] auth_token or HELIX_WEB_TOKEN.",
            bind_ip
        );
    }

    // Ensure certificates exist (generate self-signed if missing)
    if tls_enabled {
        ensure_certificates(cert_path, key_path)?;
    }

    // Build backend list...
    let mut backend_list = Vec::new();
    for b in &parsed_cfg.ollama {
        backend_list.push(serde_json::json!({
            "id":    b.name,
            "label": format!("Ollama {}", b.name),
            "kind":  "ollama"
        }));
    }
    backend_list.push(serde_json::json!({
        "id":    "gemini",
        "label": "Gemini",
        "kind":  "gemini"
    }));
    let backends_json = serde_json::to_string(&backend_list).unwrap_or_else(|_| "[]".to_string());

    // ── OKF Librarian setup (MUST happen BEFORE moving config_str into AppState) ──
    // We always mount the routes; handlers gracefully handle disabled state.
    let okf_enabled = toml::from_str::<toml::Value>(&config_str)
        .ok()
        .and_then(|v| v.get("helix")?.get("okf")?.get("enabled")?.as_bool())
        .unwrap_or(false);

    let okf_state: okf_api::OkfState = if okf_enabled {
        let okf_cfg = OkfConfig::load_from_toml(&config_str);
        let mut librarian = OkfLibrarian::new(okf_cfg);

        // Try to load from index file on startup if it exists
        let _ = librarian.reload_from_index_file();

        // Optionally load sample manifest in dev if registry is empty
        if librarian.registry.tool_count() == 0 {
            if let Ok(sample) = std::fs::read_to_string("okf_sample_manifest.json") {
                let _ = librarian.load_manifest_from_json(&sample);
            }
        }

        std::sync::Arc::new(tokio::sync::Mutex::new(librarian))
    } else {
        let disabled_cfg = OkfConfig::default();
        std::sync::Arc::new(tokio::sync::Mutex::new(OkfLibrarian::new(disabled_cfg)))
    };

    // Make the librarian available globally to tools, agents, CPU, and Ollama
    crate::okf::set_global_librarian(okf_state.clone());

    // Start background remote change detection poller (164.1 + 165)
    // Only when OKF is enabled in config. The poller respects auto_reload.
    if okf_enabled {
        let _poller = crate::okf::start_okf_poller();
        if _poller.is_some() {
            info!("[OKF] Background change-detection poller started");
        }
    }

    // OKF routes: read-only endpoints stay open; mutating endpoints
    // (reload / manifest push) require the web token via middleware.
    // /okf/webhook is intentionally left open — it is server-to-server traffic
    // from the configured OKF server and only triggers a reload when
    // auto_reload is enabled. (Revisit if a shared webhook secret is added.)
    let token_state = Arc::new(auth_token.clone());

    // A2A routes (task 190): the Agent Card is public discovery; the JSON-RPC
    // endpoint is a mutating surface, so it requires the web token via the
    // same middleware as the other mutating endpoints. Tasks execute on the
    // same agent loop as ACP (one loop, multiple protocol surfaces).
    let scheme = if tls_enabled { "https" } else { "http" };
    let a2a_base_url = format!("{}://{}:{}", scheme, bind_ip, port);
    let a2a_max_steps = crate::config::acp::AcpConfig::load_from_toml(&config_str).max_steps;
    let a2a_state = crate::a2a::A2aState::new(a2a_base_url, a2a_max_steps);
    let a2a_public = Router::new()
        .route(
            "/.well-known/agent-card.json",
            get(crate::a2a::agent_card),
        )
        .with_state(a2a_state.clone());
    let a2a_protected = Router::new()
        .route("/a2a", post(crate::a2a::a2a_jsonrpc))
        .route_layer(middleware::from_fn_with_state(
            token_state.clone(),
            require_web_token,
        ))
        .with_state(a2a_state);
    let okf_public = Router::new()
        .route("/okf/status", get(okf_api::okf_status))
        .route("/okf/registry", get(okf_api::okf_registry))
        .route("/okf/health", get(okf_api::okf_health))
        .route("/okf/schema", get(okf_api::okf_schema))
        .route("/okf/webhook", post(okf_api::okf_webhook))
        // v1 read endpoints the fetcher (and grok-cli remote fetch) expect:
        .route("/okf/manifest.json", get(okf_api::okf_manifest_json))
        .route(
            "/okf/bundles/{id}/manifest.json",
            get(okf_api::okf_bundle_manifest),
        )
        .route("/okf/knowledge/{kid}", get(okf_api::okf_knowledge_doc))
        .with_state(okf_state.clone());
    let okf_protected = Router::new()
        .route("/okf/reload", post(okf_api::okf_reload))
        .route("/okf/reload/force", post(okf_api::okf_force_reload))
        .route("/okf/manifest", post(okf_api::okf_push_manifest))
        .route_layer(middleware::from_fn_with_state(
            token_state,
            require_web_token,
        ))
        .with_state(okf_state.clone());
    // v1 write endpoints (grok-cli targets): accept EITHER the web token
    // (existing middleware posture) OR the OKF auth_token Bearer <redacted> grok-cli
    // sends. Fail-open when neither is configured, matching house posture.
    let okf_write_auth = OkfWriteAuth {
        web_token: Arc::new(auth_token.clone()),
        okf_token: OkfConfig::load_from_toml(&config_str)
            .auth_token
            .unwrap_or_default(),
    };
    let okf_writes = Router::new()
        .route("/okf/traces", post(okf_api::okf_traces))
        .route(
            "/okf/bundles/{bundle}/concepts",
            post(okf_api::okf_push_concept),
        )
        .route_layer(middleware::from_fn_with_state(
            okf_write_auth,
            require_okf_write_token,
        ))
        .with_state(okf_state);

    let state = AppState {
        bus,
        msg_tx: broadcast::channel(100).0,
        config_str,
        config_path,
        backends_json,
        logging: parsed_cfg.logging.clone(),
        auth_token,
        routines,
        inflight: Arc::new(Mutex::new(HashMap::new())),
    };

    // Task 230: sub-agent registry → WS status re-broadcast. Installed once
    // here (not per-connection): every registry change (spawn/finish) pushes
    // a fresh agent_status snapshot to all WS clients via the broadcast
    // channel. Read-only status traffic, same auth model as backends/tool_call.
    {
        let listener_state = state.clone();
        crate::agents::subagent_registry::set_status_listener(Arc::new(move || {
            let _ = listener_state
                .msg_tx
                .send(agent_status_snapshot(&listener_state));
        }));
    }

    // Task 230: reap in-flight entries whose backend never answered (5-minute
    // TTL), so a lost response can't wedge a backend on "working" forever.
    {
        let sweep_state = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                let cutoff = get_timestamp().saturating_sub(5 * 60 * 1000);
                let reaped = match sweep_state.inflight.lock() {
                    Ok(mut g) => {
                        let before = g.len();
                        g.retain(|_, (_, sent_at)| *sent_at >= cutoff);
                        before - g.len()
                    }
                    Err(_) => 0,
                };
                if reaped > 0 {
                    log::warn!("agent_status: reaped {} stale in-flight entries", reaped);
                    let _ = sweep_state
                        .msg_tx
                        .send(agent_status_snapshot(&sweep_state));
                }
            }
        });
    }

    // Build Axum router for main app
    let main_app = Router::new()
        .route("/", get(serve_index))
        .route("/ws", get(ws_handler))
        .route("/logs/chat", get(serve_chat_log))
        .route("/logs/chat/clear", post(clear_chat_log))
        .route("/logs/error", get(serve_error_log))
        .route("/logs/error/clear", post(clear_error_log))
        .route("/logs/bus", get(serve_bus_log))
        .route("/logs/hartbeat", get(serve_hartbeat_log))
        .route("/ops/update", post(ops_update))
        .route("/ops/update/status", get(ops_update_status))
        .route("/ops/restart", post(ops_restart))
        .route("/ops/usage", get(ops_usage))
        .with_state(state);

    // Merge OKF routes (they have their own state) with main app.
    // No permissive CORS: the UI is served same-origin from this server, so
    // cross-origin access is not needed. Binding defaults to 127.0.0.1 (see
    // [web] bind) and mutating endpoints require the web token when one is set.
    let app = main_app
        .merge(okf_public)
        .merge(okf_protected)
        .merge(okf_writes)
        .merge(a2a_public)
        .merge(a2a_protected);

    let addr = SocketAddr::new(bind_ip, port);

    if tls_enabled {
        info!("Starting HTTPS Web Server on {}:{}", bind_ip, port);
        info!("Using certificate: {} and key: {}", cert_path, key_path);

        let rustls_config = RustlsConfig::from_pem_file(cert_path, key_path).await?;
        axum_server::bind_rustls(addr, rustls_config)
            .serve(app.into_make_service())
            .await?;
    } else {
        info!("Starting HTTP Web Server on {}:{}", bind_ip, port);
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;
    }

    Ok(())
}

// ── Web auth helpers ──────────────────────────────────────────────────────────

/// Constant-time token comparison.
///
/// Plain `==` bails on the first mismatched byte, leaking the token
/// byte-by-byte to a timing oracle. On a loopback homelab service this is
/// theoretical — but the fix is one line, so there's no excuse.
fn constant_time_token_eq(provided: &str, expected: &str) -> bool {
    bool::from(provided.as_bytes().ct_eq(expected.as_bytes()))
}

/// Check an HTTP request's token against the configured web token.
/// `Authorization: Bearer <token>` or `X-Web-Token: <token>` are accepted.
/// When no token is configured this returns true (startup logs a warning).
fn http_token_ok(token: &str, headers: &HeaderMap) -> bool {
    if token.is_empty() {
        return true;
    }
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer ").or_else(|| s.strip_prefix("bearer ")));
    let x_token = headers.get("x-web-token").and_then(|v| v.to_str().ok());
    bearer.or(x_token).is_some_and(|t| constant_time_token_eq(t, token))
}

/// Axum middleware: gate mutating OKF routes behind the web token.
async fn require_web_token(
    State(token): State<Arc<String>>,
    req: axum::http::Request<axum::body::Body>,
    next: middleware::Next,
) -> Result<axum::response::Response, (StatusCode, &'static str)> {
    if http_token_ok(&token, req.headers()) {
        Ok(next.run(req).await)
    } else {
        Err((StatusCode::UNAUTHORIZED, "missing or invalid web token"))
    }
}

/// Auth state for the v1 OKF write endpoints (task 208).
/// grok-cli sends the OKF `auth_token` as a Bearer <redacted> it never sees the web
/// token, so these endpoints accept EITHER credential.
#[derive(Clone)]
struct OkfWriteAuth {
    web_token: Arc<String>,
    okf_token: String,
}

/// Axum middleware: gate POST /okf/traces and POST /okf/bundles/{bundle}/concepts
/// behind the web token OR the OKF auth_token Bearer. Fail-open when neither is
/// configured, matching the house posture (same as http_token_ok).
async fn require_okf_write_token(
    State(auth): State<OkfWriteAuth>,
    req: axum::http::Request<axum::body::Body>,
    next: middleware::Next,
) -> Result<axum::response::Response, (StatusCode, &'static str)> {
    if auth.web_token.is_empty() && auth.okf_token.is_empty() {
        return Ok(next.run(req).await);
    }
    if http_token_ok(&auth.web_token, req.headers()) {
        return Ok(next.run(req).await);
    }
    let bearer = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| {
            s.strip_prefix("Bearer ")
                .or_else(|| s.strip_prefix("bearer "))
        });
    if !auth.okf_token.is_empty()
        && bearer.is_some_and(|b| constant_time_token_eq(b, &auth.okf_token))
    {
        return Ok(next.run(req).await);
    }
    Err((StatusCode::UNAUTHORIZED, "missing or invalid token"))
}

// ── Config redaction (task 184) ───────────────────────────────────────────────
// The full config.toml (API keys, tokens, hosts, paths) must never be pushed
// to WS clients unfiltered. The initial "config" push — and the echo after a
// save — carry a redacted rendering instead. The config editor keeps working:
// the redactor is line-based (comments/formatting survive), and config_save
// restores any ***REDACTED*** placeholders the operator left untouched from
// the on-disk config before validating + writing.

/// Placeholder substituted for secret values in config text sent to clients.
const REDACTED: &str = "***REDACTED***";

/// Key-name fragments (case-insensitive) treated as secrets for redaction.
/// Bare "key" / "credential" are included deliberately: a redactor would
/// rather over-redact ("monkey" gets masked — harmless) than leak `api-key`.
fn is_secret_key(key: &str) -> bool {
    let k = key.to_lowercase();
    [
        "token",
        "secret",
        "password",
        "passwd",
        "api_key",
        "apikey",
        "key",
        "credential",
        "auth",
        "private_key",
    ]
    .iter()
    .any(|frag| k.contains(frag))
}

/// Render a display-safe copy of the raw config TOML with secret values
/// replaced by `***REDACTED***`.
///
/// Line-based (not a TOML re-serialize) so comments, ordering, and formatting
/// survive for the config editor. Best-effort: only `key = value` lines whose
/// key looks secret are masked — the real security boundary is the web token +
/// loopback bind (task 183); this is defense-in-depth for the WS push.
fn redacted_config_str(config_str: &str) -> String {
    config_str
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            // Skip section headers, comments, blank lines, and lines without '='.
            if trimmed.is_empty()
                || trimmed.starts_with('[')
                || trimmed.starts_with('#')
                || !trimmed.contains('=')
            {
                return line.to_string();
            }
            let raw_key = trimmed
                .split_once('=')
                .map(|(k, _)| k.trim())
                .unwrap_or("");
            let key = raw_key.trim_matches('"').trim_matches('\'');
            if key.is_empty() || !is_secret_key(key) {
                return line.to_string();
            }
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            format!("{}{} = \"{}\"", indent, raw_key, REDACTED)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Parse a `[section]` / `[[section]]` header line; returns the section name.
fn section_of(line: &str) -> Option<String> {
    let t = line.trim();
    if t.starts_with('[') && t.ends_with(']') {
        Some(
            t.trim_matches(|c| c == '[' || c == ']')
                .trim()
                .trim_matches('"')
                .to_string(),
        )
    } else {
        None
    }
}

/// Find the raw value text for `key` under `section` in TOML `text`.
/// Section-aware so the same key under different tables resolves correctly.
fn find_orig_value<'a>(text: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut cur = String::new();
    for line in text.lines() {
        if let Some(s) = section_of(line) {
            cur = s;
            continue;
        }
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = t.split_once('=') {
            let k = k.trim().trim_matches('"').trim_matches('\'');
            if k == key && cur == section {
                return Some(v.trim());
            }
        }
    }
    None
}

/// Restore `***REDACTED***` placeholders in `new_text` from the on-disk
/// `orig_text`, section-aware and formatting-preserving. Values the operator
/// actually changed are kept as-is.
fn restore_redacted_text(orig_text: &str, new_text: &str) -> String {
    if !new_text.contains(REDACTED) {
        return new_text.to_string();
    }
    let mut cur_section = String::new();
    let mut out = Vec::new();
    for line in new_text.lines() {
        if let Some(s) = section_of(line) {
            cur_section = s;
            out.push(line.to_string());
            continue;
        }
        let mut restored: Option<String> = None;
        if let Some((k, v)) = line.trim().split_once('=') {
            let raw_key = k.trim();
            let key = raw_key.trim_matches('"').trim_matches('\'');
            if v.trim() == format!("\"{}\"", REDACTED) && is_secret_key(key) {
                if let Some(orig_v) = find_orig_value(orig_text, &cur_section, key) {
                    let indent: String =
                        line.chars().take_while(|c| c.is_whitespace()).collect();
                    restored = Some(format!("{}{} = {}", indent, raw_key, orig_v));
                }
            }
        }
        out.push(restored.unwrap_or_else(|| line.to_string()));
    }
    out.join("\n")
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn get_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── agent_status snapshot (task 230) ──────────────────────────────────────────
// The web UI left panel renders sub-agent rows and per-backend working/idle
// indicators from this. Sent on every WS connect and re-broadcast on every
// registry change (spawn/finish) and in-flight change (request/response).
// Clients re-render from the snapshot wholesale — no incremental sync state.
fn agent_status_snapshot(state: &AppState) -> String {
    build_agent_status(&state.backends_json, &state.inflight)
}

// ── agent_status builder (task 230) ───────────────────────────────────────────
// Split from agent_status_snapshot so the bridge thread (which only holds the
// pieces, not AppState) can re-broadcast without cloning the whole state.
fn build_agent_status(
    backends_json: &str,
    inflight: &Arc<Mutex<HashMap<u64, (String, u64)>>>,
) -> String {
    let backends: Vec<Value> = serde_json::from_str(backends_json).unwrap_or_default();
    let working_backends: Vec<String> = inflight
        .lock()
        .map(|g| g.values().map(|(b, _)| b.clone()).collect())
        .unwrap_or_default();
    let backends_out: Vec<Value> = backends
        .iter()
        .map(|b| {
            let id = b["id"].as_str().unwrap_or("");
            json!({
                "id": id,
                "activity": if working_backends.iter().any(|w| w == id) { "working" } else { "idle" },
            })
        })
        .collect();
    let subagents: Vec<Value> = crate::agents::subagent_registry::snapshot()
        .iter()
        .map(crate::agents::subagent_registry::record_to_json)
        .collect();
    json!({
        "type": "agent_status",
        "backends": backends_out,
        "subagents": subagents,
    })
    .to_string()
}

// ── Bus → WS message translation (task 230) ────────────────────────────────
// Extracted from the bridge thread below so unit tests can drive it
// directly. Returns (payload, agent_status_dirty): payload is None for
// messages that produce no UI output; dirty is true when an in-flight
// backend entry was retired and the caller should re-broadcast
// agent_status so the working indicator flips promptly.
fn bridge_out_message(
    msg: &Message,
    inflight: &Arc<Mutex<HashMap<u64, (String, u64)>>>,
) -> (Option<String>, bool) {
    // Parse the inner data to check its type field reliably.
    let inner: serde_json::Value = serde_json::from_str(&msg.data).unwrap_or_default();
    let msg_type = inner["type"].as_str().unwrap_or("");

    // A response carrying correlation_id retires the matching in-flight
    // entry, flipping that backend back to idle in the next snapshot.
    let retire_inflight = || -> bool {
        match inner["correlation_id"].as_u64() {
            Some(cid) if cid != 0 => inflight
                .lock()
                .map(|mut g| g.remove(&cid).is_some())
                .unwrap_or(false),
            _ => false,
        }
    };
    let cid = inner["correlation_id"].as_u64().unwrap_or(0);

    // Ultra-defensive text extraction shared by the response arms: always
    // end up with a plain string or give up (None).
    fn extract_text(inner: &serde_json::Value) -> String {
        let mut text = String::new();
        for key in ["msg", "data", "content", "text", "response"] {
            if let Some(s) = inner[key].as_str() {
                text = s.to_string();
                break;
            } else if let Some(v) = inner.get(key) {
                if v.is_string() {
                    text = v.as_str().unwrap_or("").to_string();
                    break;
                } else if v.is_object() {
                    // drill one level
                    if let Some(s) = v["msg"]
                        .as_str()
                        .or_else(|| v["data"].as_str())
                        .or_else(|| v["content"].as_str())
                    {
                        text = s.to_string();
                        break;
                    }
                }
            }
        }
        if text.is_empty() {
            // last resort: if the whole inner is just a string or has a top-level value
            if let Some(s) = inner.as_str() {
                text = s.to_string();
            }
        }
        text
    }

    match msg_type {
        "llm_output" | "llm_response" => {
            let text = extract_text(&inner);
            if text.is_empty() {
                return (None, false);
            }
            let dirty = retire_inflight();
            (
                Some(
                    json!({ "type": "llm_output", "from": msg.from, "data": text, "correlation_id": cid })
                        .to_string(),
                ),
                dirty,
            )
        }
        "ollama_response" => {
            let text = extract_text(&inner);
            if text.is_empty() {
                return (None, false);
            }
            let from = inner["llm"].as_str().unwrap_or(&msg.from).to_string();
            let dirty = retire_inflight();
            (
                Some(
                    json!({ "type": "ollama_response", "from": from, "data": text, "correlation_id": cid })
                        .to_string(),
                ),
                dirty,
            )
        }
        // Well-formed UI messages — forward as-is (including errors so they are visible)
        "user_msg" | "config" | "manifest" | "config_status" | "manifest_status"
        | "log" | "error" | "warning" | "tool_call" | "heartbeat_status"
        | "heartbeat_missed" | "subagent_event" => (Some(msg.data.clone()), false),
        _ => {
            // Unknown — wrap so nothing is silently dropped
            (
                Some(
                    json!({
                        "type": "bus_msg",
                        "to": msg.to,
                        "from": msg.from,
                        "data": msg.data,
                        "timestamp": msg.timestamp
                    })
                    .to_string(),
                ),
                false,
            )
        }
    }
}

async fn handle_ws(socket: WebSocket, state: AppState) {
    let (mut ws_sender, mut ws_receiver) = socket.split();

    // Subscribe to the CPU→Web broadcast channel
    let mut recv = state.msg_tx.subscribe();

    // 1. Send current config (redacted — raw secrets are never pushed to clients)
    let config_msg = json!({
        "type": "config",
        "data": redacted_config_str(&state.config_str)
    })
    .to_string();
    let _ = ws_sender.send(WsMessage::Text(config_msg.into())).await;

    // 2. Send system manifest
    let manifest_path = "system_manifest.md";
    if !Path::new(manifest_path).exists() {
        fs::write(
            manifest_path,
            "# System Manifest\n\nWelcome to the Helix system.\n\nEdit this file to configure behaviour.\n",
        )
        .ok();
    }
    let manifest = fs::read_to_string(manifest_path).unwrap_or_default();
    let manifest_msg = json!({
        "type": "manifest",
        "data": manifest
    })
    .to_string();
    let _ = ws_sender.send(WsMessage::Text(manifest_msg.into())).await;

    // 3. Send backends list so the UI can build LLM selector buttons
    let backends_msg = serde_json::json!({
        "type": "backends",
        "backends": serde_json::from_str::<serde_json::Value>(&state.backends_json)
            .unwrap_or(serde_json::Value::Array(vec![]))
    })
    .to_string();
    let _ = ws_sender.send(WsMessage::Text(backends_msg.into())).await;

    // 3b. Send agent_status snapshot (task 230): per-backend working/idle
    // indicators plus the live sub-agent list, so the UI is correct from
    // the first paint without waiting for an event.
    let _ = ws_sender
        .send(WsMessage::Text(agent_status_snapshot(&state).into()))
        .await;

    // 4. Send a test log entry to confirm the WS pipe is working
    let test_log_msg = json!({
        "type": "log",
        "level": "info",
        "msg": "WebSocket connection established"
    })
    .to_string();
    let _ = state.msg_tx.send(test_log_msg);

    // Per-connection reply channel: for messages that must reach ONLY this
    // client (auth replies, the unredacted config). The broadcast channel
    // carries public traffic; secrets never go on it — every connection
    // subscribes to broadcast, so anything sent there leaks to all clients.
    let (direct_tx, mut direct_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    // 5. Task: forward broadcast + per-connection messages → WebSocket
    let recv_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                msg = recv.recv() => {
                    let message_str = match msg {
                        Ok(m) => m,
                        Err(_) => break,
                    };
                    if ws_sender
                        .send(WsMessage::Text(message_str.into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                direct = direct_rx.recv() => {
                    let message_str = match direct {
                        Some(m) => m,
                        // Sender dropped — the connection is going away.
                        None => break,
                    };
                    if ws_sender
                        .send(WsMessage::Text(message_str.into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    });

    // 6. Bus → WebSocket forwarder.
    //
    // IMPORTANT: state.bus uses std::sync::mpsc (blocking).  Calling
    // rx.recv() directly inside an async task blocks the Tokio thread and
    // starves other tasks on the same thread (including recv_task above).
    //
    // Solution: run the blocking recv loop on a dedicated OS thread via
    // std::thread::spawn, bridge into async-land through a tokio mpsc channel,
    // then have a lightweight async task drain that channel into msg_tx.
    let bus_clone = state.bus.clone();
    let msg_tx_clone = state.msg_tx.clone();

    // Bridge channel: blocking thread → async task
    let (bridge_tx, mut bridge_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    // Blocking thread: subscribes to the bus and forwards to the bridge
    let inflight_bridge = state.inflight.clone();
    let backends_bridge = state.backends_json.clone();
    let msg_tx_bridge = state.msg_tx.clone();
    std::thread::spawn(move || {
        let rx = bus_clone.subscribe("web_interface");
        while let Ok(msg) = rx.recv() {
            let (out, dirty) = bridge_out_message(&msg, &inflight_bridge);
            if dirty {
                // A backend just answered: push a fresh agent_status so its
                // working indicator flips back to idle without waiting for
                // the next registry event.
                let _ =
                    msg_tx_bridge.send(build_agent_status(&backends_bridge, &inflight_bridge));
            }
            if let Some(out) = out {
                if bridge_tx.send(out).is_err() {
                    break; // async side dropped (WS closed)
                }
            }
        }
    });

    // Async task: drains the bridge and fans out into the broadcast channel
    let bus_forward_task = tokio::spawn(async move {
        while let Some(out) = bridge_rx.recv().await {
            let _ = msg_tx_clone.send(out);
        }
    });

    // 7. No longer needed — CPU already publishes to "web_interface" which
    //    the bus_forward_task above now correctly handles.
    let cpu_forward_task = tokio::spawn(async move { /* no-op */ });

    // 6. Main loop: WebSocket messages → Bus
    //
    // `authed` tracks whether this connection presented the web token.
    // Mutating messages (config_save, manifest_save) are rejected until the
    // client sends {"type":"auth","token":"..."}. When no token is configured
    // the endpoints stay open (startup warns loudly about this).
    let mut authed = false;
    while let Some(msg) = ws_receiver.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(_) => break,
        };

        if let WsMessage::Text(text_bytes) = msg {
            let text = text_bytes.to_string();

            if let Ok(json_val) = serde_json::from_str::<Value>(&text)
                && let Some(msg_type) = json_val["type"].as_str()
            {
                match msg_type {
                    // ── Auth: present the web token ──────────────────────
                    // Client sends {"type":"auth","token":"..."}. On success
                    // the connection is marked authenticated and the full
                    // (unredacted) config is pushed for the editor.
                    "auth" => {
                        let tok = json_val["token"].as_str().unwrap_or("");
                        if !state.auth_token.is_empty()
                            && constant_time_token_eq(tok, &state.auth_token)
                        {
                            authed = true;
                            // Replies go on the per-connection channel, NEVER
                            // the broadcast channel — every WS client receives
                            // broadcast, including unauthenticated ones.
                            let ok_msg = json!({
                                "type": "auth_status",
                                "status": "ok",
                            })
                            .to_string();
                            let _ = direct_tx.send(ok_msg);
                            let full_config_msg = json!({
                                "type": "config",
                                "data": state.config_str.clone()
                            })
                            .to_string();
                            let _ = direct_tx.send(full_config_msg);
                        } else {
                            let err_msg = json!({
                                "type": "auth_status",
                                "status": "error",
                                "msg": "invalid token (or no token configured on the server)",
                            })
                            .to_string();
                            let _ = direct_tx.send(err_msg);
                        }
                    }

                    // ── Heartbeat status: read the beat file, reply direct ──
                    // Task 200. The UI asks {type:"heartbeat_status"}; we read
                    // heartbeat.md (written by the tick loop) and answer on
                    // the per-connection channel, like auth replies.
                    "heartbeat_status" => {
                        let reply = match read_heartbeat_status(&state.config_path) {
                            Some((tick, timestamp, uptime_secs, errors)) => {
                                // last_beat_ms_ago from the file mtime.
                                let last_beat_ms_ago =
                                    std::fs::metadata(resolve_log_path(&state.config_path, "heartbeat.md"))
                                        .and_then(|m| m.modified())
                                        .ok()
                                        .and_then(|mtime| {
                                            std::time::SystemTime::now()
                                                .duration_since(mtime)
                                                .ok()
                                        })
                                        .map(|d| d.as_millis() as u64)
                                        .unwrap_or(u64::MAX);
                                json!({
                                    "type": "heartbeat_status",
                                    "tick": tick,
                                    "timestamp": timestamp,
                                    "last_beat_ms_ago": last_beat_ms_ago,
                                    "uptime_secs": uptime_secs,
                                    "errors": errors,
                                })
                                .to_string()
                            }
                            None => json!({
                                "type": "heartbeat_status",
                                "status": "no_beats_yet",
                                "msg": "heartbeat.md not found — the tick loop may not have beaten yet",
                            })
                            .to_string(),
                        };
                        let _ = direct_tx.send(reply);
                    }

                    // ── Routines: task 197's contract, backed by the real ──
                    // registry (task 201). UI sends {"type":"routines_list"};
                    // we reply {"type":"routines", routines:[...]} on the
                    // per-connection channel.
                    "routines_list" => {
                        let reg = state.routines.read().await;
                        let routines: Vec<Value> = reg
                            .list()
                            .iter()
                            .map(|r| {
                                let sched_text =
                                    crate::cron::registry::parse_schedule(&r.schedule)
                                        .map(crate::cron::registry::schedule_display)
                                        .unwrap_or_else(|_| r.schedule.clone());
                                json!({
                                    "id": r.id,
                                    "name": r.name,
                                    "schedule": sched_text,
                                    // Task 220: canonical form for round-trip
                                    // editing ("daily HH:MM" / "every <n>s|m|h|d").
                                    "schedule_raw": r.schedule,
                                    "enabled": r.enabled,
                                    // Task 216: agent job payload (empty for legacy routines).
                                    "agent_role": r.agent_role,
                                    "task": r.task,
                                    "backend": r.backend,
                                })
                            })
                            .collect();
                        let reply = json!({
                            "type": "routines",
                            "routines": routines,
                        })
                        .to_string();
                        let _ = direct_tx.send(reply);
                    }

                    // UI sends {"type":"routine_toggle", id, enabled}; we
                    // persist and confirm {"type":"routine_status",
                    // id, enabled, ok}.
                    "routine_toggle" => {
                        let id = json_val["id"].as_str().unwrap_or("").to_string();
                        let enabled = json_val["enabled"].as_bool().unwrap_or(false);
                        let mut reg = state.routines.write().await;
                        let reply = match reg.set_enabled(&id, enabled) {
                            Some(r) => json!({
                                "type": "routine_status",
                                "id": r.id,
                                "enabled": r.enabled,
                                "ok": true,
                            })
                            .to_string(),
                            None => json!({
                                "type": "routine_status",
                                "id": id,
                                "enabled": enabled,
                                "ok": false,
                                "msg": "unknown routine id",
                            })
                            .to_string(),
                        };
                        let _ = direct_tx.send(reply);
                    }

                    // ── Task 220: routine CRUD for the editor UI ──────────
                    // UI sends {"type":"routine_create", name, schedule,
                    // agent_role, task, backend, enabled}; we validate and
                    // reply {"type":"routine_saved", id, ok} (ok:false with
                    // msg on bad schedule/backend/name).
                    "routine_create" => {
                        let name = json_val["name"].as_str().unwrap_or("").to_string();
                        let schedule = json_val["schedule"].as_str().unwrap_or("").to_string();
                        let agent_role = json_val["agent_role"].as_str().unwrap_or("").to_string();
                        let task = json_val["task"].as_str().unwrap_or("").to_string();
                        let backend = json_val["backend"].as_str().unwrap_or("auto").to_string();
                        let enabled = json_val["enabled"].as_bool().unwrap_or(true);
                        let mut reg = state.routines.write().await;
                        let reply = match reg.add(&name, &schedule, &agent_role, &task, &backend, enabled) {
                            Ok(r) => json!({
                                "type": "routine_saved",
                                "id": r.id,
                                "ok": true,
                            })
                            .to_string(),
                            Err(msg) => json!({
                                "type": "routine_saved",
                                "id": "",
                                "ok": false,
                                "msg": msg,
                            })
                            .to_string(),
                        };
                        let _ = direct_tx.send(reply);
                    }

                    // UI sends {"type":"routine_update", id, ...same fields};
                    // absent fields are left unchanged.
                    "routine_update" => {
                        let id = json_val["id"].as_str().unwrap_or("").to_string();
                        let opt = |k: &str| json_val.get(k).and_then(|v| v.as_str());
                        let enabled = json_val.get("enabled").and_then(|v| v.as_bool());
                        let mut reg = state.routines.write().await;
                        let reply = match reg.update(
                            &id,
                            opt("name"),
                            opt("schedule"),
                            opt("agent_role"),
                            opt("task"),
                            opt("backend"),
                            enabled,
                        ) {
                            Ok(r) => json!({
                                "type": "routine_saved",
                                "id": r.id,
                                "ok": true,
                            })
                            .to_string(),
                            Err(msg) => json!({
                                "type": "routine_saved",
                                "id": id,
                                "ok": false,
                                "msg": msg,
                            })
                            .to_string(),
                        };
                        let _ = direct_tx.send(reply);
                    }

                    // UI sends {"type":"routine_delete", id}; we confirm
                    // {"type":"routine_status", id, ok}.
                    "routine_delete" => {
                        let id = json_val["id"].as_str().unwrap_or("").to_string();
                        let mut reg = state.routines.write().await;
                        let ok = reg.remove(&id);
                        let reply = json!({
                            "type": "routine_status",
                            "id": id,
                            "ok": ok,
                        })
                        .to_string();
                        let _ = direct_tx.send(reply);
                    }

                    // ── Chat: LLM-aware routing ──────────────────────────
                    "chat" => {
                        let chat_msg = json_val["msg"].as_str().unwrap_or("").to_string();
                        if chat_msg.is_empty() {
                            continue;
                        }

                        // Write to chat log (respect config)
                        let chat_log_path = &state.logging.chat_log;
                        if let Ok(mut f) = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(chat_log_path)
                        {
                            use std::io::Write;
                            let _ = writeln!(
                                f,
                                "[{}] User: {}",
                                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                                chat_msg
                            );
                        }

                        let llm = json_val["llm"].as_str().unwrap_or("").to_string();
                        info!("Chat request received | llm='{}' | msg='{}'", llm, chat_msg);
                        println!("[WEB] Chat received: llm='{}'  msg='{}'", llm, crate::utils::truncate_str(&chat_msg, 80));

                        let bus_dest = if llm == "gemini" {
                            "gemini".to_string()
                        } else if llm.is_empty() {
                            let backends: serde_json::Value =
                                serde_json::from_str(&state.backends_json)
                                    .unwrap_or(serde_json::Value::Array(vec![]));
                            let first_id = backends[0]["id"].as_str().unwrap_or("server");
                            format!("ollama_{}", first_id)
                        } else {
                            format!("ollama_{}", llm)
                        };

                        // Task 230: mark this backend working until its response
                        // crosses the bridge (or the TTL sweep reaps it). The
                        // ms-timestamp key is bumped on the unlikely collision
                        // so in-flight keys stay unique.
                        let backend_id = bus_dest
                            .strip_prefix("ollama_")
                            .unwrap_or(&bus_dest)
                            .to_string();
                        let mut correlation_id = get_timestamp();
                        {
                            let mut guard = state.inflight.lock().unwrap();
                            while guard.contains_key(&correlation_id) {
                                correlation_id = correlation_id.wrapping_add(1);
                            }
                            guard.insert(correlation_id, (backend_id, get_timestamp()));
                        }
                        let _ = state.msg_tx.send(agent_status_snapshot(&state));

                        println!("[WEB] === SENDING BUS MESSAGE ===");
                        println!("[WEB]   to: {}", bus_dest);
                        println!("[WEB]   from: web_interface");
                        println!("[WEB]   type: chat_request");
                        println!("[WEB]   prompt preview: {}", crate::utils::truncate_str(&chat_msg, 100));

                        println!("[WEB]   to: {}", bus_dest);

                        let bus_msg = Message {
                            to: bus_dest.clone(),
                            from: "web_interface".to_string(),
                            data: json!({
                                "type": "chat_request",
                                "prompt": chat_msg,
                                "correlation_id": correlation_id,
                            })
                            .to_string(),
                            timestamp: correlation_id,
                        };
                        println!("[WEB] Publishing bus message now...");
                        let _ = state.bus.publish(bus_msg);

                        println!("[WEB] Routing to bus_dest='{}'  (llm was '{}')", bus_dest, llm);
                        info!("Routing chat to bus destination: {}", bus_dest);
                        println!("[WEB] Bus publish done for correlation_id={}", correlation_id);

                        // Echo user message back to UI
                        let echo_msg = json!({
                            "type": "user_msg",
                            "from": "You",
                            "data": chat_msg
                        })
                        .to_string();
                        let _ = state.msg_tx.send(echo_msg);
                    }

                    // ── Config save ──────────────────────────────────────
                    // Privileged write: requires the web token when one is
                    // configured (see the "auth" message above).
                    "config_save" => {
                        if !state.auth_token.is_empty() && !authed {
                            let error_msg = json!({
                                "type": "config_status",
                                "status": "error",
                                "msg": "Not authenticated: send {\"type\":\"auth\",\"token\":\"...\"} first."
                            })
                            .to_string();
                            let bus_msg = Message {
                                to: "web_interface".to_string(),
                                from: "config".to_string(),
                                data: error_msg,
                                timestamp: get_timestamp(),
                            };
                            let _ = state.bus.publish(bus_msg);
                            continue;
                        }
                        let new_config_str = json_val["data"].as_str().unwrap_or("");

                        // The UI was shown the redacted config; restore any
                        // ***REDACTED*** placeholders the operator left
                        // untouched from the on-disk config, so a save never
                        // clobbers real secrets with the placeholder text.
                        // Formatting and comments are preserved (textual merge).
                        // The merge base must be the LIVE config file (the one
                        // Helix loaded), not a hardcoded path — otherwise a
                        // save restores secrets from the wrong file.
                        let disk_config =
                            fs::read_to_string(&state.config_path).unwrap_or_default();
                        let merged_config_str =
                            restore_redacted_text(&disk_config, new_config_str);

                        // Fail-safe: if any placeholder survived the restore
                        // (e.g. the key moved sections), refuse to write
                        // rather than persist the placeholder as a secret.
                        if merged_config_str.contains(REDACTED) {
                            let error_msg = json!({
                                "type": "config_status",
                                "status": "error",
                                "msg": "Config still contains ***REDACTED*** placeholders that could not be matched to the on-disk config. Re-authenticate (or replace them with real values) and save again."
                            })
                            .to_string();
                            let bus_msg = Message {
                                to: "web_interface".to_string(),
                                from: "config".to_string(),
                                data: error_msg,
                                timestamp: get_timestamp(),
                            };
                            let _ = state.bus.publish(bus_msg);
                            continue;
                        }

                        if let Ok(_parsed) = toml::from_str::<Config>(&merged_config_str) {
                            if fs::write(&state.config_path, &merged_config_str).is_ok() {
                                let success_msg = json!({
                                        "type": "config_status",
                                        "status": "success",
                                        "msg": "Config saved successfully. Restart Helix to apply changes."
                                    })
                                    .to_string();
                                let bus_msg = Message {
                                    to: "web_interface".to_string(),
                                    from: "config".to_string(),
                                    data: success_msg,
                                    timestamp: get_timestamp(),
                                };
                                let _ = state.bus.publish(bus_msg);

                                // Echo updated config back (redacted — never raw secrets)
                                let updated_config_msg = json!({
                                    "type": "config",
                                    "data": redacted_config_str(&merged_config_str)
                                })
                                .to_string();
                                let _ = state.msg_tx.send(updated_config_msg);
                            } else {
                                let error_msg = json!({
                                    "type": "config_status",
                                    "status": "error",
                                    "msg": "Failed to write config file."
                                })
                                .to_string();
                                let bus_msg = Message {
                                    to: "web_interface".to_string(),
                                    from: "config".to_string(),
                                    data: error_msg,
                                    timestamp: get_timestamp(),
                                };
                                let _ = state.bus.publish(bus_msg);
                            }
                        } else {
                            let error_msg = json!({
                                "type": "config_status",
                                "status": "error",
                                "msg": "Invalid TOML syntax."
                            })
                            .to_string();
                            let bus_msg = Message {
                                to: "web_interface".to_string(),
                                from: "config".to_string(),
                                data: error_msg,
                                timestamp: get_timestamp(),
                            };
                            let _ = state.bus.publish(bus_msg);
                        }
                    }

                    // ── Manifest save ────────────────────────────────────
                    // Privileged write (the agent's system prompt): same auth
                    // gate as config_save.
                    "manifest_save" => {
                        if !state.auth_token.is_empty() && !authed {
                            let error_msg = json!({
                                "type": "manifest_status",
                                "status": "error",
                                "msg": "Not authenticated: send {\"type\":\"auth\",\"token\":\"...\"} first."
                            })
                            .to_string();
                            let bus_msg = Message {
                                to: "web_interface".to_string(),
                                from: "manifest".to_string(),
                                data: error_msg,
                                timestamp: get_timestamp(),
                            };
                            let _ = state.bus.publish(bus_msg);
                            continue;
                        }
                        let new_manifest = json_val["data"].as_str().unwrap_or("");

                        if fs::write("system_manifest.md", new_manifest).is_ok() {
                            let success_msg = json!({
                                "type": "manifest_status",
                                "status": "success",
                                "msg": "Manifest saved successfully."
                            })
                            .to_string();
                            let bus_msg = Message {
                                to: "web_interface".to_string(),
                                from: "manifest".to_string(),
                                data: success_msg,
                                timestamp: get_timestamp(),
                            };
                            let _ = state.bus.publish(bus_msg);

                            // Echo updated manifest back
                            let updated_manifest_msg = json!({
                                "type": "manifest",
                                "data": new_manifest
                            })
                            .to_string();
                            let _ = state.msg_tx.send(updated_manifest_msg);
                        } else {
                            let error_msg = json!({
                                "type": "manifest_status",
                                "status": "error",
                                "msg": "Failed to write manifest file."
                            })
                            .to_string();
                            let bus_msg = Message {
                                to: "web_interface".to_string(),
                                from: "manifest".to_string(),
                                data: error_msg,
                                timestamp: get_timestamp(),
                            };
                            let _ = state.bus.publish(bus_msg);
                        }
                    }

                    "slash_cmd" => {
                        let cmd = json_val["cmd"].as_str().unwrap_or("").trim().to_string();
                        let result = handle_slash_command(&cmd);
                        let msg_out = serde_json::json!({
                            "type": "user_msg",
                            "from": "You",
                            "data": cmd.clone()
                        })
                        .to_string();
                        let _ = state.msg_tx.send(msg_out);
                        let reply = serde_json::json!({
                            "type": "ollama_response",
                            "llm": "system",
                            "msg": result
                        })
                        .to_string();
                        let bus_msg = crate::bus::Message {
                            to: "web_interface".to_string(),
                            from: "system".to_string(),
                            data: reply,
                            timestamp: get_timestamp(),
                        };
                        let _ = state.bus.publish(bus_msg);
                    }

                    _ => {}
                }
            }
        }
    }

    recv_task.abort();
    bus_forward_task.abort();
    cpu_forward_task.abort();
}

// ── Axum route helpers ────────────────────────────────────────────────────────

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn serve_index() -> Html<String> {
    Html(MAIN_HTML.to_string())
}

// ── Log file handlers ────────────────────────────────────────────────────────

async fn serve_chat_log(State(state): State<AppState>) -> impl IntoResponse {
    let path = &state.logging.chat_log;
    if !std::path::Path::new(path).exists() {
        let _ = std::fs::write(path, "[INIT] Chat log created\n");
    }
    let content =
        fs::read_to_string(path).unwrap_or_else(|_| "chat_log.md not found or empty".to_string());
    Html(format!(
        "<pre style='white-space:pre-wrap;'>{}</pre>",
        html_escape(&content)
    ))
}

async fn clear_chat_log(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if !http_token_ok(&state.auth_token, &headers) {
        return (StatusCode::UNAUTHORIZED, "missing or invalid web token").into_response();
    }
    let init_line = format!(
        "[INIT] Chat log cleared via web UI at {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );
    let _ = fs::write(&state.logging.chat_log, init_line);
    Html("<span style='color:#69f0ae'>Chat log cleared.</span>").into_response()
}

async fn clear_error_log(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if !http_token_ok(&state.auth_token, &headers) {
        return (StatusCode::UNAUTHORIZED, "missing or invalid web token").into_response();
    }
    let init_line = format!(
        "[INIT] Error log cleared via web UI at {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );
    let _ = fs::write(&state.logging.error_log, init_line);
    Html("<span style='color:#69f0ae'>Error log cleared.</span>").into_response()
}

async fn serve_error_log(State(state): State<AppState>) -> impl IntoResponse {
    let path = &state.logging.error_log;
    if !std::path::Path::new(path).exists() {
        let _ = std::fs::write(path, "[INIT] Error log created\n");
    }
    let content =
        fs::read_to_string(path).unwrap_or_else(|_| "error_log.md not found or empty".to_string());
    Html(format!(
        "<pre style='white-space:pre-wrap;'>{}</pre>",
        html_escape(&content)
    ))
}

async fn serve_bus_log(State(state): State<AppState>) -> impl IntoResponse {
    let content = fs::read_to_string(&state.logging.bus_log)
        .unwrap_or_else(|_| "bus_log.md not found or empty".to_string());
    Html(format!(
        "<pre style='white-space:pre-wrap;'>{}</pre>",
        html_escape(&content)
    ))
}

async fn serve_hartbeat_log(State(state): State<AppState>) -> impl IntoResponse {
    let content =
        fs::read_to_string(resolve_log_path(&state.config_path, &state.logging.hartbeat_log))
            .unwrap_or_else(|_| "hartbeat_log.md not found or empty".to_string());
    Html(format!(
        "<pre style='white-space:pre-wrap;'>{}</pre>",
        html_escape(&content)
    ))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Resolve a log path against the directory of the config file Helix actually
/// loaded (task 188). Relative paths in [logging] are interpreted there, not
/// in the process CWD — otherwise the web UI reads a different file than the
/// one the runtime writes.
fn resolve_log_path(config_path: &str, log_path: &str) -> String {
    let p = std::path::Path::new(log_path);
    if p.is_absolute() {
        return log_path.to_string();
    }
    let base = if config_path == "none" {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    } else {
        std::path::Path::new(config_path)
            .parent()
            .filter(|par| !par.as_os_str().is_empty())
            .map(|par| par.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    };
    base.join(p).to_string_lossy().to_string()
}

/// Read heartbeat.md (written by the tick loop in main.rs) and extract the
/// beat fields. Returns (tick, timestamp, uptime_secs, errors).
fn read_heartbeat_status(config_path: &str) -> Option<(u64, String, u64, u64)> {
    let content = std::fs::read_to_string(resolve_log_path(config_path, "heartbeat.md")).ok()?;
    crate::utils::parse_heartbeat_md(&content)
}

const MAIN_HTML: &str = include_str!("static/index.html");

// ── Ops endpoints: web UI self-update + restart (tasks 228, 229) ─────────────
// Both mutate the host, so both are auth-gated with http_token_ok — the same
// gate as the log-clear endpoints. Fail-open only when no [web] auth_token.

/// Directory holding the ops job files (update script/log/pid), resolved with
/// the same config-dir rule as every other log path (task 188).
fn ops_dir(state: &AppState) -> std::path::PathBuf {
    let log = resolve_log_path(&state.config_path, "logs/web-update.log");
    std::path::Path::new(&log)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("logs"))
}

/// The checkout the running backend was built from: the directory holding the
/// config file Helix actually loaded (WorkingDirectory under systemd).
fn repo_dir(state: &AppState) -> std::path::PathBuf {
    std::path::Path::new(&state.config_path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/home/cobble/helix"))
        })
}

/// Single-quote a string for embedding in a shell script.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Is this pid alive? /proc-based, no extra crates.
fn process_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

fn read_pid(path: &std::path::Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Last `n` chars of a file, char-boundary safe. Empty when unreadable.
fn tail_chars(path: &std::path::Path, n: usize) -> String {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    if content.len() <= n {
        return content;
    }
    let mut start = content.len() - n;
    while !content.is_char_boundary(start) {
        start += 1;
    }
    content[start..].to_string()
}

/// Build the self-update shell script.
///
/// The drill is John's own: git pull → cargo build --release →
/// cargo run --bin installer. cargo lives in ~/.cargo/bin, which systemd
/// services don't have on PATH, so the script exports it. The installer
/// needs root for /usr/local/bin + /etc/systemd/system — headless sudo only
/// works with a NOPASSWD rule, otherwise the log shows sudo's complaint.
///
/// `cgroup_escape` false (no systemd-run available) means the installer step
/// is skipped: running it from inside helix.service's cgroup would SIGTERM
/// the updater mid-install and leave helix down. The log then says exactly
/// what to run by hand. Fail-safe beats fail-dead, sir.
fn build_update_script(repo: &str, log: &str, pid: &str, cgroup_escape: bool) -> String {
    let repo_q = sh_quote(repo);
    let log_q = sh_quote(log);
    let pid_q = sh_quote(pid);
    let installer_step = if cgroup_escape {
        "echo '--- cargo run --bin installer ---'\ncargo run --bin installer\nrc=$?\n".to_string()
    } else {
        format!(
            "echo 'NOTE: no cgroup escape available — skipping the installer step.'\necho 'Finish by hand on the R630: cd {repo_q} && cargo run --bin installer'\nrc=2\n"
        )
    };
    format!(
        "echo $$ > {pid_q}\n{{\n\
         echo '=== helix web update started '$(date -u +%FT%TZ)' ==='\n\
         cd {repo_q} || {{ echo 'cannot cd to repo dir'; echo 'EXIT:1'; exit 1; }}\n\
         echo '--- git pull ---'\n\
         git pull || {{ echo 'git pull FAILED'; echo 'EXIT:1'; exit 1; }}\n\
         export PATH=\"$HOME/.cargo/bin:$PATH\"\n\
         echo '--- cargo build --release ---'\n\
         cargo build --release || {{ echo 'build FAILED'; echo 'EXIT:1'; exit 1; }}\n\
         {installer_step}\
         echo \"=== update finished rc=$rc '$(date -u +%FT%TZ)' ===\"\n\
         echo \"EXIT:$rc\"\n\
         }} >> {log_q} 2>&1\n"
    )
}

/// POST /ops/update — launch a detached self-update job (task 228).
async fn ops_update(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if !http_token_ok(&state.auth_token, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "missing or invalid web token"})),
        )
            .into_response();
    }
    let dir = ops_dir(&state);
    let log_path = dir.join("web-update.log");
    let pid_path = dir.join("web-update.pid");
    if read_pid(&pid_path).is_some_and(process_alive) {
        let pid = read_pid(&pid_path).unwrap_or(0);
        return (
            StatusCode::CONFLICT,
            Json(json!({"status": "already_running", "pid": pid})),
        )
            .into_response();
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("cannot create ops dir: {e}")})),
        )
            .into_response();
    }
    let repo = repo_dir(&state).to_string_lossy().to_string();
    // systemd-run --scope puts the job in its own cgroup, so the installer's
    // `systemctl stop helix` can't SIGTERM the updater mid-install. A mere
    // setsid child would die there and leave helix down — hence the upgrade.
    let have_systemd_run = std::process::Command::new("systemd-run")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let script = build_update_script(
        &repo,
        &log_path.to_string_lossy(),
        &pid_path.to_string_lossy(),
        have_systemd_run,
    );
    let script_path = dir.join("web-update.sh");
    if let Err(e) = std::fs::write(&script_path, &script) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("cannot write update script: {e}")})),
        )
            .into_response();
    }
    let _ = std::fs::write(
        &log_path,
        format!(
            "update requested at {}\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        ),
    );
    let _ = std::fs::remove_file(&pid_path);

    // setsid + spawn-without-wait: the endpoint never blocks on the job.
    let launched = if have_systemd_run {
        let unit = format!(
            "helix-web-update-{}",
            chrono::Local::now().format("%Y%m%d%H%M%S")
        );
        std::process::Command::new("setsid")
            .args([
                "systemd-run",
                "--scope",
                &format!("--unit={unit}"),
                "--quiet",
                "bash",
            ])
            .arg(&script_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| "systemd-run --scope")
            .map_err(|e| e.to_string())
    } else {
        std::process::Command::new("setsid")
            .arg("bash")
            .arg(&script_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| "setsid (safe mode: installer step skipped)")
            .map_err(|e| e.to_string())
    };
    match launched {
        Ok(via) => Json(json!({"status": "started", "via": via})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("could not launch updater: {e}")})),
        )
            .into_response(),
    }
}

/// GET /ops/update/status — poll the update job (task 228).
async fn ops_update_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !http_token_ok(&state.auth_token, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "missing or invalid web token"})),
        )
            .into_response();
    }
    let dir = ops_dir(&state);
    let running = read_pid(&dir.join("web-update.pid")).is_some_and(process_alive);
    Json(json!({
        "running": running,
        "log_tail": tail_chars(&dir.join("web-update.log"), 6000),
    }))
    .into_response()
}

/// GET /ops/usage — per-day, per-backend token usage + spend (task 236).
/// Answers "where are usage stats": real metered tokens, not estimates.
async fn ops_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !http_token_ok(&state.auth_token, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "missing or invalid web token"})),
        )
            .into_response();
    }
    Json(crate::router::spend::usage_summary_today()).into_response()
}

/// POST /ops/restart — restart the helix service (task 229).
async fn ops_restart(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if !http_token_ok(&state.auth_token, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "missing or invalid web token"})),
        )
            .into_response();
    }
    // --no-block: queue the restart and return before systemd SIGTERMs us.
    // The installer's pattern is interactive sudo, which cannot work headless;
    // `sudo -n` fails fast instead of hanging when no NOPASSWD rule exists.
    let direct = std::process::Command::new("systemctl")
        .args(["--no-block", "restart", "helix"])
        .output();
    let via = if matches!(&direct, Ok(o) if o.status.success()) {
        Some("systemctl")
    } else {
        match std::process::Command::new("sudo")
            .args(["-n", "systemctl", "--no-block", "restart", "helix"])
            .output()
        {
            Ok(o) if o.status.success() => Some("sudo -n systemctl"),
            Ok(o) => {
                let detail = String::from_utf8_lossy(&o.stderr).trim().to_string();
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "error": "restart failed",
                        "detail": detail,
                        "hint": "the web backend runs as cobble; allow it via polkit or add a NOPASSWD sudoers rule for 'systemctl restart helix'",
                    })),
                )
                    .into_response();
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": format!("could not run systemctl: {e}")})),
                )
                    .into_response();
            }
        }
    };
    Json(json!({"status": "restart_initiated", "via": via})).into_response()
}

/// Returns true if a file exists AND contains valid PEM data (not a placeholder).
fn is_valid_pem(path: &str, expected_header: &str) -> bool {
    std::fs::read_to_string(path)
        .map(|s| s.contains(expected_header))
        .unwrap_or(false)
}

/// Generate self-signed certificate + key if they are missing or contain
/// placeholder content (not real PEM data).
fn ensure_certificates(cert_path: &str, key_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let cert_ok = is_valid_pem(cert_path, "-----BEGIN CERTIFICATE-----");
    let key_ok = is_valid_pem(key_path, "-----BEGIN");

    if cert_ok && key_ok {
        return Ok(());
    }

    log::info!("TLS cert/key missing or invalid — generating self-signed certificate");

    let key_pair = rcgen::KeyPair::generate()?;
    let params = rcgen::CertificateParams::new(vec!["localhost".to_string()])?;
    let cert = params.self_signed(&key_pair)?;

    std::fs::write(cert_path, cert.pem())?;
    std::fs::write(key_path, key_pair.serialize_pem())?;

    log::info!(
        "Self-signed certificate written to {} / {}",
        cert_path,
        key_path
    );

    Ok(())
}

/// Execute a slash command typed directly in the chat box.
/// Returns the result string to display in the chat.
fn handle_slash_command(cmd: &str) -> String {
    let parts: Vec<&str> = cmd.splitn(3, ' ').collect();
    let verb = parts[0].trim_start_matches('/').to_lowercase();
    let args = serde_json::json!({});

    match verb.as_str() {
        "status" => crate::tools::execute("system_status", &args),
        "tools" => crate::tools::execute("list_tools", &args),
        "notes" => crate::tools::execute("list_notes", &args),
        "beliefs" => crate::tools::execute("get_beliefs", &args),
        "note" => {
            // /note <title>
            let title = parts.get(1).copied().unwrap_or("untitled");
            crate::tools::execute("read_note", &serde_json::json!({"title": title}))
        }
        "set" => {
            // /set key=value
            let kv = parts.get(1).copied().unwrap_or("");
            if let Some((k, v)) = kv.split_once('=') {
                crate::tools::execute(
                    "set_belief",
                    &serde_json::json!({"key": k.trim(), "value": v.trim()}),
                )
            } else {
                "Usage: /set key=value".to_string()
            }
        }
        "log" => {
            // /log [filename] — default to chat_log.md
            let file = parts
                .get(1)
                .map(|f| {
                    if f.contains('/') {
                        f.to_string()
                    } else {
                        format!("logs/{}", f)
                    }
                })
                .unwrap_or_else(|| "logs/chat_log.md".to_string());
            crate::tools::execute("read_log", &serde_json::json!({"log_file": file}))
        }
        "bayes" => {
            // /bayes [show|status|update <evidence>|reset]
            let sub = parts.get(1).copied().unwrap_or("show").to_lowercase();
            match sub.as_str() {
                "show" | "status" => crate::tools::execute("bayes_show", &args),
                "reset" => crate::tools::execute("bayes_reset", &args),
                "update" => {
                    let evidence = parts.get(2).copied().unwrap_or("");
                    if evidence.is_empty() {
                        "Usage: /bayes update <evidence>\nExample: /bayes update positive_signal"
                            .to_string()
                    } else {
                        crate::tools::execute(
                            "bayes_update",
                            &serde_json::json!({"evidence": evidence}),
                        )
                    }
                }
                _ => format!(
                    "Unknown bayes sub-command '{}'.\nAvailable: /bayes show  /bayes status  /bayes update <evidence>  /bayes reset",
                    sub
                ),
            }
        }
        "persona" => {
            // /persona [status|approve [role]|discard [role]|draft [role]]
            // Task 233: review self-authored persona drafts. Drafts land as
            // .pending and never go live without John's explicit approval.
            let sub = parts.get(1).copied().unwrap_or("status").to_lowercase();
            let role_arg = parts.get(2).copied().unwrap_or("").trim();
            // "agent" (or empty) means Helix's own agent.md; anything else is a role.
            let role_lower = role_arg.to_lowercase();
            let role: Option<&str> = match role_lower.as_str() {
                "" | "agent" | "helix" => None,
                r => Some(r),
            };
            let live_path = match crate::persona::persona_path(role) {
                Some(p) => p,
                None => return format!("Invalid role name '{}'.", role_arg),
            };
            match sub.as_str() {
                "status" => {
                    let mut lines = vec![format!(
                        "Persona self-authoring: {}",
                        if crate::persona::self_author_enabled() { "enabled" } else { "disabled" }
                    )];
                    let live = if live_path.is_file() { "present" } else { "missing" };
                    lines.push(format!("{}: {}", live_path.display(), live));
                    let pending = crate::persona::list_pending();
                    if pending.is_empty() {
                        lines.push("No pending drafts.".to_string());
                    } else {
                        lines.push("Pending drafts (awaiting /persona approve):".to_string());
                        for p in pending {
                            lines.push(format!("  {}", p.display()));
                        }
                    }
                    lines.join("\n")
                }
                "approve" => match crate::persona::apply_pending_persona(&live_path) {
                    Ok(msg) => msg,
                    Err(e) => format!("Cannot approve: {}", e),
                },
                "discard" => match crate::persona::discard_pending_persona(&live_path) {
                    Ok(msg) => msg,
                    Err(e) => format!("Cannot discard: {}", e),
                },
                "draft" => {
                    if !crate::persona::self_author_enabled() {
                        "Persona self-authoring is disabled ([helix.persona] self_author).".to_string()
                    } else {
                        // Block this worker thread until the draft lands, same
                        // pattern as spawn_subagent's sync entry point.
                        let handle = match tokio::runtime::Handle::try_current() {
                            Ok(h) => h,
                            Err(_) => return "Drafting needs a Tokio runtime.".to_string(),
                        };
                        let role_owned = role.map(|r| r.to_string());
                        let out = tokio::task::block_in_place(|| {
                            handle.block_on(crate::persona::ensure_persona_file(
                                role_owned.as_deref(),
                            ))
                        });
                        match out {
                            Ok(p) => format!(
                                "Draft staged at {} — review with /persona status, then /persona approve{}.",
                                p.display(),
                                role.map(|r| format!(" {}", r)).unwrap_or_default()
                            ),
                            Err(e) => format!("Draft failed: {}", e),
                        }
                    }
                }
                _ => "Usage: /persona [status|approve [role]|discard [role]|draft [role]]".to_string(),
            }
        }
        "help" => format!(
            "Slash commands:\n\
             /status              — system health\n\
             /tools               — list all tools\n\
             /notes               — list saved notes\n\
             /note <title>        — read a note\n\
             /beliefs             — show agent beliefs\n\
             /set k=v             — store a belief\n\
             /log [file]          — tail a log file\n\
             /bayes show          — show Bayesian belief state\n\
             /bayes update <ev>   — apply Bayesian evidence update\n\
             /bayes reset         — reset to default priors\n\
             /persona [status]    — persona files + pending drafts\n\
             /persona approve [r] — apply a pending persona draft\n\
             /persona discard [r] — discard a pending persona draft\n\
             /persona draft [r]   — draft a persona now (self-authoring)\n\
             /help                — this message"
        ),
        other => format!("Unknown command '/{}'  — type /help for a list", other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Bus;

    #[tokio::test]
    async fn test_server_start() {
        let bus = Arc::new(Bus::new());
        // Full start expects certs/port, but verifies no panic on startup path
        let routines = Arc::new(RwLock::new(
            crate::cron::registry::RoutineRegistry::load_or_seed(
                std::env::temp_dir().join("helix-test-routines.json"),
            ),
        ));
        tokio::spawn(async move {
            let _ =
                start_web_server(bus, 8443, "".to_string(), "config.toml".to_string(), routines)
                    .await;
        });
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    // ── web auth (task 183) ─────────────────────────────────────────────────

    #[test]
    fn test_http_token_ok_no_token_configured() {
        // Empty token = open (startup warns); never lock the operator out.
        let headers = HeaderMap::new();
        assert!(http_token_ok("", &headers));
    }

    #[test]
    fn test_http_token_ok_rejects_missing_header() {
        let headers = HeaderMap::new();
        assert!(!http_token_ok("s3cret", &headers));
    }

    #[test]
    fn test_http_token_ok_accepts_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer s3cret".parse().unwrap());
        assert!(http_token_ok("s3cret", &headers));

        let mut wrong = HeaderMap::new();
        wrong.insert("authorization", "Bearer wrong".parse().unwrap());
        assert!(!http_token_ok("s3cret", &wrong));
    }

    #[test]
    fn test_http_token_ok_accepts_x_web_token() {
        let mut headers = HeaderMap::new();
        headers.insert("x-web-token", "s3cret".parse().unwrap());
        assert!(http_token_ok("s3cret", &headers));
        assert!(!http_token_ok("other", &headers));
    }

    #[test]
    fn test_constant_time_token_eq_matches_and_rejects() {
        // GIVEN identical tokens, WHEN compared, THEN they match.
        assert!(constant_time_token_eq("s3cret", "s3cret"));
        // WHEN they differ (or differ in length), THEN they never match.
        assert!(!constant_time_token_eq("s3cret", "wr0ng!"));
        assert!(!constant_time_token_eq("s3cret", "s3cret-longer"));
        assert!(!constant_time_token_eq("", "s3cret"));
    }

    #[test]
    fn test_is_secret_key_catches_hyphenated_and_bare_key() {
        // Hyphenated and bare forms must redact — the old list missed them.
        assert!(is_secret_key("api-key"));
        assert!(is_secret_key("credential"));
        assert!(is_secret_key("API_KEY"));
        // Non-secrets stay visible.
        assert!(!is_secret_key("username"));
        assert!(!is_secret_key("theme"));
    }

    #[test]
    fn test_web_config_defaults_bind_loopback() {
        // bind defaults to 127.0.0.1; auth_token defaults to empty.
        let cfg: WebConfig = toml::from_str("port = 8443").unwrap();
        assert_eq!(cfg.bind, "127.0.0.1");
        assert!(cfg.auth_token.is_empty());
    }

    #[test]
    fn test_web_config_parses_bind_and_token() {
        let cfg: WebConfig =
            toml::from_str("port = 8443\nbind = \"0.0.0.0\"\nauth_token = \"s3cret\"").unwrap();
        assert_eq!(cfg.bind, "0.0.0.0");
        assert_eq!(cfg.auth_token, "s3cret");
    }

    // ── config redaction (task 184) ─────────────────────────────────────────

    #[test]
    fn test_redacted_config_hides_secrets() {
        let cfg = "[helix]\nname = \"Helix\"\n\n\
                   [web]\nport = 8443\nauth_token = \"s3cret\"\n\
                   # a comment line\napi_key = \"key123\"\n";
        let red = redacted_config_str(cfg);
        assert!(!red.contains("s3cret"), "token leaked: {}", red);
        assert!(!red.contains("key123"), "api key leaked: {}", red);
        assert!(red.contains(REDACTED));
        // Non-secrets and structure survive.
        assert!(red.contains("8443"));
        assert!(red.contains("[web]"));
        assert!(red.contains("# a comment line"));
        assert!(red.contains("name = \"Helix\""));
    }

    #[test]
    fn test_redacted_config_hides_hyphenated_keys() {
        // api-key (hyphenated) slipped through the old fragment list.
        let cfg = "[web]\nport = 8443\napi-key = \"key123\"\n";
        let red = redacted_config_str(cfg);
        assert!(!red.contains("key123"), "hyphenated key leaked: {}", red);
        assert!(red.contains(REDACTED));
        assert!(red.contains("8443"));
    }

    #[test]
    fn test_redacted_config_section_aware() {        // Same key name under different sections resolves per-section.
        let cfg = "[a]\npassword = \"pw-a\"\n[b]\npassword = \"pw-b\"\n";
        let red = redacted_config_str(cfg);
        assert!(!red.contains("pw-a"));
        assert!(!red.contains("pw-b"));

        let new_text = "[a]\npassword = \"***REDACTED***\"\n[b]\npassword = \"pw-B2\"\n";
        let merged = restore_redacted_text(cfg, new_text);
        assert!(merged.contains("password = \"pw-a\""), "got: {}", merged);
        assert!(merged.contains("password = \"pw-B2\""), "got: {}", merged);
        assert!(!merged.contains(REDACTED));
    }

    #[test]
    fn test_restore_redacted_keeps_operator_changes() {
        let orig = "[web]\nauth_token = \"old-secret\"\nport = 8443\n";
        let new = "[web]\nauth_token = \"brand-new-secret\"\nport = 9999\n";
        let merged = restore_redacted_text(orig, new);
        assert!(merged.contains("auth_token = \"brand-new-secret\""), "got: {}", merged);
        assert!(merged.contains("port = 9999"), "got: {}", merged);
    }

    #[test]
    fn test_restore_redacted_no_placeholders_passthrough() {
        let orig = "a = 1\n";
        let new = "a = 2\n# comment\n";
        assert_eq!(restore_redacted_text(orig, new), new);
    }

    #[test]
    fn test_redacted_config_unparseable_shape_kept() {
        // Lines that don't look like key=value pass through untouched.
        let cfg = "just some text\n[unclosed\nkey_without_value\n";
        assert_eq!(redacted_config_str(cfg), cfg.trim_end());
    }

    // ── A2A routes (task 190) ────────────────────────────────────────────────

    /// Build the exact A2A router assembly used in `start_web_server`:
    /// public Agent Card + token-gated JSON-RPC endpoint.
    fn a2a_test_app() -> Router {
        let a2a_state = crate::a2a::A2aState::new("http://127.0.0.1:1".to_string(), 5);
        let token_state = Arc::new("s3cret".to_string());
        Router::new()
            .route(
                "/.well-known/agent-card.json",
                get(crate::a2a::agent_card),
            )
            .with_state(a2a_state.clone())
            .merge(
                Router::new()
                    .route("/a2a", post(crate::a2a::a2a_jsonrpc))
                    .route_layer(middleware::from_fn_with_state(
                        token_state,
                        require_web_token,
                    ))
                    .with_state(a2a_state),
            )
    }

    #[tokio::test]
    async fn test_a2a_agent_card_is_public() {
        use tower::ServiceExt;
        let req = axum::http::Request::builder()
            .uri("/.well-known/agent-card.json")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = a2a_test_app().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let card: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(card["protocolVersion"], "1.0");
        assert_eq!(card["authentication"]["schemes"][0], "bearer");
    }

    #[tokio::test]
    async fn test_a2a_jsonrpc_requires_token() {
        use tower::ServiceExt;
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tasks/list","params":{}}"#;
        // No token -> 401.
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/a2a")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body))
            .unwrap();
        let resp = a2a_test_app().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        // Wrong token -> 401.
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/a2a")
            .header("content-type", "application/json")
            .header("authorization", "Bearer wrong-token")
            .body(axum::body::Body::from(body))
            .unwrap();
        let resp = a2a_test_app().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_a2a_jsonrpc_dispatch_with_token() {
        use tower::ServiceExt;
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/a2a")
            .header("content-type", "application/json")
            .header("authorization", "Bearer s3cret")
            .body(axum::body::Body::from(
                r#"{"jsonrpc":"2.0","id":7,"method":"tasks/list","params":{}}"#,
            ))
            .unwrap();
        let resp = a2a_test_app().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], 7);
        assert!(v["result"].is_array());
    }

    // ── ops endpoints: self-update + restart (tasks 228, 229) ────────────────

    #[test]
    fn build_update_script_full_mode_runs_johns_drill() {
        // GIVEN cgroup escape is available,
        let s = build_update_script("/home/cobble/helix", "/x/web-update.log", "/x/web-update.pid", true);
        // WHEN the script is generated,
        // THEN it runs John's exact drill and reports progress to the log:
        assert!(s.contains("git pull"), "must git pull");
        assert!(s.contains("cargo build --release"), "must build release");
        assert!(s.contains("cargo run --bin installer"), "must run the installer, not reimplement it");
        assert!(s.contains("$HOME/.cargo/bin"), "cargo needs its PATH export under systemd");
        assert!(s.contains("echo $$ > '/x/web-update.pid'"), "must record its pid for the status endpoint");
        assert!(s.contains(">> '/x/web-update.log' 2>&1"), "must append all output to the log");
        assert!(s.contains("EXIT:"), "must leave an EXIT marker for the UI");
    }

    #[test]
    fn build_update_script_safe_mode_skips_installer_step() {
        // GIVEN no systemd-run (no cgroup escape),
        let s = build_update_script("/home/cobble/helix", "/x/web-update.log", "/x/web-update.pid", false);
        // WHEN the script is generated,
        // THEN the installer step is skipped — running it inside
        // helix.service's cgroup would SIGTERM the updater mid-install:
        assert!(!s.contains("echo '--- cargo run --bin installer ---'"), "must NOT execute the installer without cgroup escape");
        assert!(s.contains("git pull"), "pull still runs");
        assert!(s.contains("cargo build --release"), "build still runs");
        assert!(s.contains("Finish by hand"), "must tell John the manual finish command");
    }

    #[test]
    fn build_update_script_quotes_paths_safely() {
        // GIVEN paths with spaces and quotes,
        let s = build_update_script("/tmp/a b", "/tmp/x'y.log", "/tmp/x.pid", true);
        // WHEN the script is generated,
        // THEN paths are single-quote-escaped for bash:
        assert!(s.contains("'/tmp/a b'"));
        assert!(s.contains("'/tmp/x'\\''y.log'"));
    }

    #[test]
    fn tail_chars_returns_tail_and_respects_char_boundaries() {
        // GIVEN a temp file with multibyte content,
        let dir = std::env::temp_dir().join("helix-ops-test");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("tail.txt");
        std::fs::write(&p, "héllo wörld — tail me").unwrap();
        // WHEN we take a short tail,
        let t = tail_chars(&p, 7);
        // THEN we get whole chars only, never a panic or a split rune:
        assert!(t.chars().count() <= 7);
        assert!(t.ends_with("tail me") || "héllo wörld — tail me".ends_with(&t));
        // AND a missing file yields empty, not a panic:
        assert_eq!(tail_chars(&dir.join("nope.txt"), 10), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn process_alive_sees_own_pid_and_rejects_bogus() {
        // GIVEN our own pid and a bogus one,
        // WHEN checked,
        // THEN the live one reports alive and the bogus one dead:
        assert!(process_alive(std::process::id()));
        assert!(!process_alive(u32::MAX));
    }

    #[test]
    fn read_pid_roundtrip() {
        // GIVEN a pid file,
        let dir = std::env::temp_dir().join("helix-ops-test2");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("test.pid");
        std::fs::write(&p, "12345\n").unwrap();
        // WHEN read back,
        // THEN we get the pid; garbage yields None:
        assert_eq!(read_pid(&p), Some(12345));
        std::fs::write(&p, "not-a-pid").unwrap();
        assert_eq!(read_pid(&p), None);
        assert_eq!(read_pid(&dir.join("missing.pid")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Minimal router with just the ops endpoints, for HTTP contract tests.
    /// config_path doubles as the repo dir + log dir anchor.
    fn ops_test_app(auth_token: &str, anchor_dir: &str) -> Router {
        let bus = Arc::new(Bus::new());
        let (msg_tx, _rx) = broadcast::channel(16);
        let routines = Arc::new(RwLock::new(
            crate::cron::registry::RoutineRegistry::load_or_seed(
                std::env::temp_dir().join("helix-ops-test-routines.json"),
            ),
        ));
        let logging = LoggingConfig {
            chat_log: format!("{anchor_dir}/chat_log.md"),
            ..Default::default()
        };
        let state = AppState {
            bus,
            msg_tx,
            config_str: String::new(),
            config_path: format!("{anchor_dir}/config.toml"),
            backends_json: "[]".to_string(),
            logging,
            auth_token: auth_token.to_string(),
            routines,
            inflight: Arc::new(Mutex::new(HashMap::new())),
        };
        Router::new()
            .route("/ops/update", post(ops_update))
            .route("/ops/update/status", get(ops_update_status))
            .route("/ops/restart", post(ops_restart))
            .route("/ops/usage", get(ops_usage))
            .with_state(state)
    }

    #[tokio::test]
    async fn ops_endpoints_reject_missing_token() {
        // GIVEN a token is configured,
        use tower::ServiceExt;
        let dir = std::env::temp_dir().join("helix-ops-test3");
        let _ = std::fs::create_dir_all(&dir);
        let app = ops_test_app("s3cret", dir.to_str().unwrap());
        // WHEN each ops endpoint is hit without credentials,
        // THEN all three reject with 401 (no side effects — the update job
        // never spawns, systemctl never runs):
        for (method, uri) in [
            ("POST", "/ops/update"),
            ("GET", "/ops/update/status"),
            ("POST", "/ops/restart"),
        ] {
            let req = axum::http::Request::builder()
                .method(method)
                .uri(uri)
                .body(axum::body::Body::empty())
                .unwrap();
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{method} {uri}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn ops_update_status_polling_contract() {
        // GIVEN an authed app with an ops dir,
        use tower::ServiceExt;
        let dir = std::env::temp_dir().join("helix-ops-test4");
        let _ = std::fs::create_dir_all(&dir);
        let anchor = dir.to_str().unwrap().to_string();
        let app = ops_test_app("s3cret", &anchor);
        // NOTE: the endpoint resolves logs/ under the config anchor (task 188).
        let logs = dir.join("logs");
        let _ = std::fs::create_dir_all(&logs);
        let authed = |method: &str, uri: &str| {
            axum::http::Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", "Bearer s3cret")
                .body(axum::body::Body::empty())
                .unwrap()
        };
        // WHEN no job has ever run,
        // THEN status reports not-running with an empty log tail:
        let resp = app
            .clone()
            .oneshot(authed("GET", "/ops/update/status"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["running"], false);
        assert_eq!(v["log_tail"], "");
        // WHEN a live pid + log exist (simulating a running job),
        std::fs::write(logs.join("web-update.pid"), format!("{}\n", std::process::id())).unwrap();
        std::fs::write(logs.join("web-update.log"), "building…\n").unwrap();
        let resp = app
            .clone()
            .oneshot(authed("GET", "/ops/update/status"))
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // THEN the contract reports running with the log tail:
        assert_eq!(v["running"], true);
        assert!(v["log_tail"].as_str().unwrap().contains("building"));
        // WHEN the pid is stale,
        std::fs::write(logs.join("web-update.pid"), "4294967295\n").unwrap();
        let resp = app
            .clone()
            .oneshot(authed("GET", "/ops/update/status"))
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // THEN running flips false but the log remains readable:
        assert_eq!(v["running"], false);
        assert!(v["log_tail"].as_str().unwrap().contains("building"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn ops_restart_failure_returns_structured_error() {
        // GIVEN an authed app on a box with no helix unit (this sandbox),
        use tower::ServiceExt;
        let dir = std::env::temp_dir().join("helix-ops-test5");
        let _ = std::fs::create_dir_all(&dir);
        let app = ops_test_app("s3cret", dir.to_str().unwrap());
        // WHEN restart is requested,
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/ops/restart")
            .header("authorization", "Bearer s3cret")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // THEN we get honest structured JSON, never a panic or a hang:
        // either initiated (a real helix box) or a detailed error here.
        assert!(
            v.get("status").is_some() || v.get("error").is_some(),
            "must answer with status or error JSON, got: {v}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// AppState with a realistic backends list for agent_status tests.
    fn status_test_state() -> AppState {
        let bus = Arc::new(Bus::new());
        let (msg_tx, _rx) = broadcast::channel(16);
        let routines = Arc::new(RwLock::new(
            crate::cron::registry::RoutineRegistry::load_or_seed(
                std::env::temp_dir().join("helix-status-test-routines.json"),
            ),
        ));
        AppState {
            bus,
            msg_tx,
            config_str: String::new(),
            config_path: String::new(),
            backends_json: serde_json::json!([
                {"id": "server", "label": "Ollama server", "kind": "ollama"},
                {"id": "desktop", "label": "Ollama desktop", "kind": "ollama"},
                {"id": "gemini", "label": "Gemini", "kind": "gemini"},
            ])
            .to_string(),
            logging: LoggingConfig::default(),
            auth_token: String::new(),
            routines,
            inflight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn bus_msg(data: serde_json::Value) -> Message {
        Message {
            to: "web_interface".to_string(),
            from: "cpu".to_string(),
            data: data.to_string(),
            timestamp: get_timestamp(),
        }
    }

    #[test]
    fn agent_status_snapshot_has_backends_activity_and_subagents() {
        use crate::agents::subagent_registry as reg;
        // GIVEN one working sub-agent and one in-flight request on "server":
        reg::reset_for_tests();
        reg::register(reg::SubagentRecord::new(
            "test-ws-snap-1".to_string(),
            "local",
            "builder",
            "build the widget",
        ));
        let state = status_test_state();
        state
            .inflight
            .lock()
            .unwrap()
            .insert(4242, ("server".to_string(), get_timestamp()));

        // WHEN we build the snapshot,
        let v: serde_json::Value =
            serde_json::from_str(&agent_status_snapshot(&state)).unwrap();

        // THEN the shape matches the UI contract: every backend carries an
        // activity flag and the live sub-agent row is present:
        assert_eq!(v["type"], "agent_status");
        let backends = v["backends"].as_array().unwrap();
        assert_eq!(backends.len(), 3);
        let activity = |id: &str| {
            backends
                .iter()
                .find(|b| b["id"] == id)
                .unwrap()["activity"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(activity("server"), "working");
        assert_eq!(activity("desktop"), "idle");
        assert_eq!(activity("gemini"), "idle");
        let subs = v["subagents"].as_array().unwrap();
        let me = subs.iter().find(|s| s["id"] == "test-ws-snap-1").unwrap();
        assert_eq!(me["status"], "working");
        assert_eq!(me["role_hint"], "builder");
        assert_eq!(me["task_summary"], "build the widget");
        reg::reset_for_tests();
    }

    #[test]
    fn bridge_forwards_subagent_event_as_is() {
        // GIVEN a subagent_event bus message,
        let msg = bus_msg(serde_json::json!({
            "type": "subagent_event",
            "event": "spawned",
            "subagent": {"id": "subagent-abc", "status": "working"},
        }));
        let inflight = Arc::new(Mutex::new(HashMap::new()));

        // WHEN it crosses the bridge,
        let (out, dirty) = bridge_out_message(&msg, &inflight);

        // THEN it is forwarded byte-identical, with no side effects:
        assert_eq!(out.as_deref(), Some(msg.data.as_str()));
        assert!(!dirty);
    }

    #[test]
    fn bridge_retires_inflight_on_response_and_flips_activity() {
        use crate::agents::subagent_registry as reg;
        // GIVEN backend "desktop" marked working by an in-flight request:
        let state = status_test_state();
        state
            .inflight
            .lock()
            .unwrap()
            .insert(777, ("desktop".to_string(), get_timestamp()));
        let before: serde_json::Value =
            serde_json::from_str(&agent_status_snapshot(&state)).unwrap();
        let activity_of = |v: &serde_json::Value, id: &str| {
            v["backends"]
                .as_array()
                .unwrap()
                .iter()
                .find(|b| b["id"] == id)
                .unwrap()["activity"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(activity_of(&before, "desktop"), "working");

        // WHEN its llm_output response (carrying the correlation_id) crosses
        // the bridge,
        let msg = bus_msg(serde_json::json!({
            "type": "llm_output",
            "correlation_id": 777u64,
            "msg": "done thinking",
        }));
        let (out, dirty) = bridge_out_message(&msg, &state.inflight);

        // THEN the payload keeps the correlation_id, the entry is retired,
        // and the next snapshot flips the backend back to idle:
        assert!(dirty);
        let v: serde_json::Value = serde_json::from_str(&out.unwrap()).unwrap();
        assert_eq!(v["type"], "llm_output");
        assert_eq!(v["correlation_id"], 777);
        assert_eq!(v["data"], "done thinking");
        assert!(!state.inflight.lock().unwrap().contains_key(&777));
        let after: serde_json::Value =
            serde_json::from_str(&agent_status_snapshot(&state)).unwrap();
        assert_eq!(activity_of(&after, "desktop"), "idle");
        reg::reset_for_tests();
    }

    #[test]
    fn bridge_forwards_unmatched_response_without_retiring() {
        // GIVEN an empty in-flight set,
        let inflight = Arc::new(Mutex::new(HashMap::new()));

        // WHEN an llm_output with an unknown correlation_id crosses,
        let msg = bus_msg(serde_json::json!({
            "type": "llm_output",
            "correlation_id": 999u64,
            "msg": "hi",
        }));
        let (out, dirty) = bridge_out_message(&msg, &inflight);

        // THEN it still forwards (the UI wants the text), but nothing is
        // retired and no status re-broadcast is requested:
        assert!(out.is_some());
        assert!(!dirty);
        assert!(inflight.lock().unwrap().is_empty());
    }
}
