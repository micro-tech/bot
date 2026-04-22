//! Bot — Discord gateway + Ollama model pre-warm and keep-alive.
//!
//! ## Startup sequence
//! 1. Load `.env` via `dotenv`.
//! 2. Initialise `env_logger` for structured logging.
//! 3. **Ollama preload** — send a dummy request so the configured model is
//!    resident in GPU/CPU memory before the first user message arrives.
//! 4. **Ollama keep-alive** — spawn a background Tokio task that pings Ollama
//!    every N seconds (default 240 s) so the model is never evicted.
//! 5. Start the Serenity Discord client.
//!
//! ## Environment variables (`.env`)
//! | Variable                 | Default                         | Purpose                              |
//! |-------------------------|---------------------------------|--------------------------------------|
//! | `DISCORD_TOKEN`          | *(required)*                    | Discord bot token                    |
//! | `OLLAMA_URL`             | `http://192.168.1.149:11434`    | Ollama base URL                      |
//! | `OLLAMA_MODEL`           | `qwen3.5:0.8b`                  | Model to preload and keep warm       |
//! | `OLLAMA_PRELOAD`         | `true`                          | Set `false` to skip startup preload  |
//! | `OLLAMA_KEEP_ALIVE_SECS` | `240`                           | Heartbeat interval; `0` = disabled   |

use std::env;
use std::time::Duration;

use log::{error, info, warn};
use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::prelude::*;

// ── tunables for the Ollama keepalive / preload ───────────────────────────────

/// Seconds to wait for a TCP connection to Ollama.
const KA_CONNECT_TIMEOUT_SECS: u64 = 5;

/// Maximum total seconds allowed for a single Ollama HTTP request.
const KA_REQUEST_TIMEOUT_SECS: u64 = 30;

/// Retry attempts before giving up on a preload or keepalive ping.
const KA_MAX_RETRIES: u32 = 3;

/// Default interval between keepalive heartbeats (4 min < Ollama's 5-min timeout).
const KA_DEFAULT_INTERVAL_SECS: u64 = 240;

/// `keep_alive` value sent to Ollama — tells it to hold the model for 1 hour.
const KA_KEEP_ALIVE_FIELD: &str = "1h";

// ── Discord event handler ─────────────────────────────────────────────────────

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        info!("Discord bot logged in as '{}'", ready.user.name);
    }

    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        let content = msg.content.to_lowercase();
        let reply = match content.as_str() {
            "hello" => Some("Hello!"),
            "ping" => Some("pong"),
            "help" => Some("Available commands: hello, ping, help"),
            _ => None,
        };

        if let Some(text) = reply {
            if let Err(e) = msg.reply(&ctx.http, text).await {
                error!("Failed to reply to '{}': {:?}", content, e);
            } else {
                info!("Replied '{}' to {}", text, msg.author.name);
            }
        }
    }
}

// ── Ollama preload ────────────────────────────────────────────────────────────

/// Send a dummy `POST /api/generate` (empty prompt, `num_predict: 0`) to Ollama
/// so the configured model is loaded into GPU/CPU memory **before** the first
/// real user message arrives, eliminating the cold-start delay.
///
/// Retries up to [`KA_MAX_RETRIES`] times with exponential-backoff delays
/// (2 s → 4 s → 8 s, capped at 30 s) to tolerate transient Starlink drops.
///
/// Returns `Ok(())` on success.  On total failure returns `Err(String)` — the
/// caller should log a warning and continue; a failed preload is not fatal.
async fn preload_model(base_url: &str, model: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(KA_CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(KA_REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let url = format!("{}/api/generate", base_url);
    info!(
        "Ollama preload ▶  model='{}' @ {} — warming up…",
        model, base_url
    );

    let mut delay_ms: u64 = 2_000;

    for attempt in 1..=KA_MAX_RETRIES {
        let payload = serde_json::json!({
            "model":       model,
            "prompt":      "",
            "stream":      false,
            "num_predict": 0
        });

        let result = tokio::time::timeout(
            Duration::from_secs(KA_REQUEST_TIMEOUT_SECS),
            client.post(&url).json(&payload).send(),
        )
        .await;

        match result {
            // ── timed out waiting for response ────────────────────────────────
            Err(_elapsed) => {
                warn!(
                    "Ollama preload attempt {}/{} timed out after {}s",
                    attempt, KA_MAX_RETRIES, KA_REQUEST_TIMEOUT_SECS
                );
            }

            // ── HTTP response received ────────────────────────────────────────
            Ok(Ok(resp)) => {
                let status = resp.status();
                if status.is_success() {
                    info!(
                        "Ollama model preloaded successfully ✅  (attempt {}/{}, HTTP {})",
                        attempt, KA_MAX_RETRIES, status
                    );
                    return Ok(());
                }
                let body = resp.text().await.unwrap_or_default();
                warn!(
                    "Ollama preload attempt {}/{} — HTTP {}: {}",
                    attempt,
                    KA_MAX_RETRIES,
                    status,
                    &body[..body.len().min(200)]
                );
            }

            // ── network / connection error ────────────────────────────────────
            Ok(Err(e)) => {
                warn!(
                    "Ollama preload attempt {}/{} — network error: {}",
                    attempt, KA_MAX_RETRIES, e
                );
            }
        }

        if attempt < KA_MAX_RETRIES {
            info!("Ollama preload retrying in {}ms…", delay_ms);
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            delay_ms = (delay_ms * 2).min(30_000);
        }
    }

    Err(format!(
        "Ollama preload failed after {} attempts — \
         model '{}' may have a cold-start delay on the first query",
        KA_MAX_RETRIES, model
    ))
}

// ── Ollama keep-alive ─────────────────────────────────────────────────────────

/// Spawn a long-lived background Tokio task that sends periodic keep-alive
/// requests to Ollama so the model stays resident in memory between sessions.
///
/// Each heartbeat POSTs to `/api/generate` with `keep_alive: "1h"` and an
/// empty prompt.  Ollama resets its eviction timer on receipt.
///
/// The task runs until the process exits and **never panics** — failures are
/// logged as warnings only.
///
/// Pass `interval_secs = 0` to disable without spawning a task.
fn spawn_keepalive_task(base_url: String, model: String, interval_secs: u64) {
    if interval_secs == 0 {
        info!("Ollama keep-alive heartbeat disabled (OLLAMA_KEEP_ALIVE_SECS=0)");
        return;
    }

    info!(
        "Ollama keep-alive ❤  heartbeat task spawned — model='{}', interval={}s",
        model, interval_secs
    );

    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(KA_CONNECT_TIMEOUT_SECS))
            .timeout(Duration::from_secs(KA_REQUEST_TIMEOUT_SECS))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                error!("Ollama keep-alive: failed to build HTTP client — {}", e);
                return;
            }
        };

        let url = format!("{}/api/generate", base_url);
        let interval = Duration::from_secs(interval_secs);

        loop {
            tokio::time::sleep(interval).await;
            ping_once(&client, &url, &model).await;
        }
    });
}

/// Fire one keepalive ping with up to [`KA_MAX_RETRIES`] attempts and
/// exponential-backoff delays.  Logs warnings on failure; never panics.
async fn ping_once(client: &reqwest::Client, url: &str, model: &str) {
    let payload = serde_json::json!({
        "model":      model,
        "prompt":     "",
        "stream":     false,
        "keep_alive": KA_KEEP_ALIVE_FIELD
    });

    let mut delay_ms: u64 = 2_000;

    for attempt in 1..=KA_MAX_RETRIES {
        let result = tokio::time::timeout(
            Duration::from_secs(KA_REQUEST_TIMEOUT_SECS),
            client.post(url).json(&payload).send(),
        )
        .await;

        match result {
            Err(_) => {
                warn!(
                    "Ollama keep-alive attempt {}/{} timed out",
                    attempt, KA_MAX_RETRIES
                );
            }
            Ok(Ok(resp)) if resp.status().is_success() => {
                info!(
                    "Ollama keep-alive ❤  ping OK (attempt {}/{}, HTTP {})",
                    attempt,
                    KA_MAX_RETRIES,
                    resp.status()
                );
                return;
            }
            Ok(Ok(resp)) => {
                warn!(
                    "Ollama keep-alive attempt {}/{} — HTTP {}",
                    attempt,
                    KA_MAX_RETRIES,
                    resp.status()
                );
            }
            Ok(Err(e)) => {
                warn!(
                    "Ollama keep-alive attempt {}/{} — network error: {}",
                    attempt, KA_MAX_RETRIES, e
                );
            }
        }

        if attempt < KA_MAX_RETRIES {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            delay_ms = (delay_ms * 2).min(30_000);
        }
    }

    error!(
        "Ollama keep-alive ❌  all {} attempts failed — model may be unloaded from memory",
        KA_MAX_RETRIES
    );
}

// ── config helpers ────────────────────────────────────────────────────────────

/// Read `OLLAMA_PRELOAD` from the environment.
/// Defaults to `true`; returns `false` only when explicitly set to `"false"`.
fn read_preload_flag() -> bool {
    env::var("OLLAMA_PRELOAD")
        .unwrap_or_else(|_| "true".to_string())
        .trim()
        .to_lowercase()
        != "false"
}

/// Read `OLLAMA_KEEP_ALIVE_SECS` from the environment.
/// Falls back to [`KA_DEFAULT_INTERVAL_SECS`] when absent or unparseable.
fn read_keep_alive_secs() -> u64 {
    env::var("OLLAMA_KEEP_ALIVE_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(KA_DEFAULT_INTERVAL_SECS)
}

// ── entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // ── 1. load .env and initialise logging ───────────────────────────────────
    dotenv::dotenv().ok();
    env_logger::init();

    info!("Bot starting up…");

    // ── 2. read Ollama config ─────────────────────────────────────────────────
    let ollama_url =
        env::var("OLLAMA_URL").unwrap_or_else(|_| "http://192.168.1.149:11434".to_string());
    let ollama_model = env::var("OLLAMA_MODEL").unwrap_or_else(|_| "qwen3.5:0.8b".to_string());

    info!(
        "Ollama config — url='{}', model='{}'",
        ollama_url, ollama_model
    );

    // ── 3. Ollama model preload (Task #89 — subtask 89.3) ─────────────────────
    if read_preload_flag() {
        match preload_model(&ollama_url, &ollama_model).await {
            Ok(()) => info!("Ollama model preloaded successfully"),
            Err(e) => warn!("Ollama preload skipped — {}", e),
        }
    } else {
        info!("Ollama preload disabled via OLLAMA_PRELOAD=false");
    }

    // ── 4. Ollama keep-alive heartbeat (Task #89 — subtask 89.4 / 89.5) ──────
    let keep_alive_secs = read_keep_alive_secs();
    spawn_keepalive_task(ollama_url, ollama_model, keep_alive_secs);

    // ── 5. Discord client ─────────────────────────────────────────────────────
    let token = env::var("DISCORD_TOKEN").expect("DISCORD_TOKEN must be set in .env");

    let intents =
        GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT | GatewayIntents::GUILDS;

    let mut client = Client::builder(&token, intents)
        .event_handler(Handler)
        .await
        .expect("Failed to create Discord client");

    info!("Discord client created — connecting…");

    if let Err(e) = client.start().await {
        error!("Discord client error: {:?}", e);
    }
}
