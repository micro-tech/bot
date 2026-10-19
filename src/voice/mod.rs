//! Voice integration (tasks 267, 268): voice-box TTS/STT HTTP client,
//! `[voice]` config, and the /api/voice/* proxy surface.
//!
//! Architecture (decided with John 2026-10-18): the browser is the
//! microphone and speaker, the voice box is a headless inference server,
//! and Helix is the calling harness. The browser never talks to the voice
//! box directly (avoids CORS wrestling; auth stays in Helix).

pub mod client;
pub mod config;

pub use client::{speak, transcribe};
pub use config::VoiceConfig;

/// Read the live `[voice]` config. Missing file/section → defaults.
/// Same runtime dir the support watcher uses (set once at startup).
pub(crate) fn live_config() -> VoiceConfig {
    let path = crate::support_watch::runtime_dir().join("config.toml");
    std::fs::read_to_string(&path)
        .map(|s| VoiceConfig::load_from_toml(&s))
        .unwrap_or_default()
}
