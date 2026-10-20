//! `[voice]` config section (task 267.1).
//!
//! Knobs for the voice-box TTS/STT server (audio.cpp). Everything is
//! optional — the section can be omitted entirely and the defaults below
//! (verified against the live box on 2026-10-18) apply.

use serde::Deserialize;

/// Parsed `[voice]` section.
#[derive(Debug, Clone, Deserialize)]
pub struct VoiceConfig {
    /// Voice-box LAN host. Default 192.168.1.142 (the voice LXC).
    /// A value starting with "http" is used as-is (scheme included);
    /// otherwise "http://" is prepended.
    #[serde(default = "default_host")]
    pub host: String,

    /// Voice-box HTTP port. Default 8081.
    #[serde(default = "default_port")]
    pub port: u16,

    /// TTS model id for /v1/audio/speech. Default "supertonic-tts".
    #[serde(default = "default_tts_model")]
    pub tts_model: String,

    /// Default TTS voice preset. Default "M1".
    #[serde(default = "default_voice")]
    pub default_voice: String,

    /// HTTP timeout for voice-box calls, milliseconds. Default 30000.
    /// CPU synthesis runs ~2.4s per short sentence; 30s leaves headroom
    /// for longer texts without hanging the chat.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,

    /// STT model id for /v1/audio/transcriptions. Default "" (empty) which
    /// omits the model field and lets the server pick its default ASR
    /// model. Set once John names the whisper GGUF in server.json (268.1).
    #[serde(default)]
    pub stt_model: String,

    /// Push-to-talk hotkey for the web UI mic (KeyboardEvent.code, e.g.
    /// "ControlRight"). Default "ControlRight". Empty string disables the
    /// hotkey entirely. Page focus is required (browser limitation).
    #[serde(default = "default_ptt_hotkey")]
    pub ptt_hotkey: String,

    /// Read agent replies aloud automatically via TTS. Default false.
    /// The web UI toggle overrides this per-session (localStorage).
    #[serde(default)]
    pub autoplay: bool,
}

fn default_host() -> String {
    "192.168.1.142".to_string()
}
fn default_port() -> u16 {
    8081
}
fn default_tts_model() -> String {
    "supertonic-tts".to_string()
}
fn default_voice() -> String {
    "M1".to_string()
}
fn default_timeout_ms() -> u64 {
    30_000
}
fn default_ptt_hotkey() -> String {
    "ControlRight".to_string()
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            tts_model: default_tts_model(),
            default_voice: default_voice(),
            timeout_ms: default_timeout_ms(),
            stt_model: String::new(),
            ptt_hotkey: default_ptt_hotkey(),
            autoplay: false,
        }
    }
}

impl VoiceConfig {
    /// Load the top-level `[voice]` section from a full TOML string.
    /// Missing section (or unparseable input) yields the defaults.
    /// Fallbacks are logged (task 265) via `config::load_section`.
    pub fn load_from_toml(toml_str: &str) -> Self {
        crate::config::load_section(toml_str, "[voice]", |v| v.get("voice").cloned())
    }

    /// Base URL for the voice box, e.g. "http://192.168.1.142:8081".
    pub fn base_url(&self) -> String {
        let host = self.host.trim().trim_end_matches('/');
        if host.starts_with("http://") || host.starts_with("https://") {
            format!("{host}:{}", self.port)
        } else {
            format!("http://{host}:{}", self.port)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_section_yields_defaults() {
        let cfg = VoiceConfig::load_from_toml("[helix]\nname = \"x\"\n");
        assert_eq!(cfg.host, "192.168.1.142");
        assert_eq!(cfg.port, 8081);
        assert_eq!(cfg.tts_model, "supertonic-tts");
        assert_eq!(cfg.default_voice, "M1");
        assert_eq!(cfg.timeout_ms, 30_000);
        assert!(cfg.stt_model.is_empty());
        assert_eq!(cfg.ptt_hotkey, "ControlRight");
        assert!(!cfg.autoplay);
        assert_eq!(cfg.base_url(), "http://192.168.1.142:8081");
    }

    #[test]
    fn full_section_parses() {
        let cfg = VoiceConfig::load_from_toml(
            "[voice]\nhost = \"192.168.1.200\"\nport = 9090\ntts_model = \"other-tts\"\ndefault_voice = \"F1\"\ntimeout_ms = 60000\nstt_model = \"whisper-large\"\nptt_hotkey = \"KeyV\"\nautoplay = true\n",
        );
        assert_eq!(cfg.host, "192.168.1.200");
        assert_eq!(cfg.port, 9090);
        assert_eq!(cfg.tts_model, "other-tts");
        assert_eq!(cfg.default_voice, "F1");
        assert_eq!(cfg.timeout_ms, 60_000);
        assert_eq!(cfg.stt_model, "whisper-large");
        assert_eq!(cfg.ptt_hotkey, "KeyV");
        assert!(cfg.autoplay);
        assert_eq!(cfg.base_url(), "http://192.168.1.200:9090");
    }

    #[test]
    fn empty_ptt_hotkey_disables() {
        let cfg = VoiceConfig::load_from_toml("[voice]\nptt_hotkey = \"\"\n");
        assert_eq!(cfg.ptt_hotkey, "");
        // Everything else keeps defaults.
        assert!(!cfg.autoplay);
        assert_eq!(cfg.host, "192.168.1.142");
    }

    #[test]
    fn partial_section_keeps_defaults() {
        let cfg = VoiceConfig::load_from_toml("[voice]\ndefault_voice = \"F2\"\n");
        assert_eq!(cfg.default_voice, "F2");
        assert_eq!(cfg.host, "192.168.1.142");
        assert_eq!(cfg.port, 8081);
    }

    #[test]
    fn garbage_input_yields_defaults() {
        let cfg = VoiceConfig::load_from_toml("not toml ][[[");
        assert_eq!(cfg.host, "192.168.1.142");
    }

    #[test]
    fn host_with_scheme_is_used_as_is() {
        let cfg = VoiceConfig {
            host: "https://voice.example.com".into(),
            port: 443,
            ..Default::default()
        };
        assert_eq!(cfg.base_url(), "https://voice.example.com:443");
    }
}
