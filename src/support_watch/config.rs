//! `[support]` config section (task 256).
//!
//! Business-hours window for the adaptive poll cadence. Everything has a
//! sane default so the section can be omitted entirely.

use serde::Deserialize;

/// Parsed `[support]` section.
#[derive(Debug, Clone, Deserialize)]
pub struct SupportConfig {
    /// Master switch for the support-mail watcher. Default true; the tick
    /// still no-ops gracefully when mail-mcp is not installed.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Business-hours window, "HH:MM" 24h, in `timezone`. Default 08:00–18:00.
    #[serde(default = "default_biz_start")]
    pub business_start: String,
    #[serde(default = "default_biz_end")]
    pub business_end: String,

    /// IANA timezone name for business hours. Default America/New_York.
    #[serde(default = "default_tz")]
    pub timezone: String,

    /// Auto-send drafted replies without John's approval. Default false.
    /// John's 2026-10-14 call: the support desk auto-sends. When true, a
    /// successfully drafted reply is approved and sent immediately by the
    /// poll tick; the draft stays visible in the store afterward.
    #[serde(default)]
    pub auto_send: bool,
}

fn default_true() -> bool {
    true
}
fn default_biz_start() -> String {
    "08:00".to_string()
}
fn default_biz_end() -> String {
    "18:00".to_string()
}
fn default_tz() -> String {
    "America/New_York".to_string()
}

impl Default for SupportConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            business_start: default_biz_start(),
            business_end: default_biz_end(),
            timezone: default_tz(),
            auto_send: false,
        }
    }
}

impl SupportConfig {
    /// Load the top-level `[support]` section from a full TOML string.
    /// Missing section (or unparseable input) yields the default.
    /// Fallbacks are logged (task 265) via `config::load_section`.
    pub fn load_from_toml(toml_str: &str) -> Self {
        crate::config::load_section(toml_str, "[support]", |v| v.get("support").cloned())
    }

    /// Parse "HH:MM" into (hour, minute). Returns None on bad input —
    /// callers fall back to the defaults rather than mis-scheduling.
    pub fn parse_hhmm(s: &str) -> Option<(u32, u32)> {
        let (h, m) = s.split_once(':')?;
        let h: u32 = h.trim().parse().ok()?;
        let m: u32 = m.trim().parse().ok()?;
        if h < 24 && m < 60 {
            Some((h, m))
        } else {
            None
        }
    }

    pub fn business_start_hm(&self) -> (u32, u32) {
        Self::parse_hhmm(&self.business_start).unwrap_or((8, 0))
    }

    pub fn business_end_hm(&self) -> (u32, u32) {
        Self::parse_hhmm(&self.business_end).unwrap_or((18, 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_section_yields_defaults() {
        let cfg = SupportConfig::load_from_toml("[helix]\nname = \"x\"\n");
        assert!(cfg.enabled);
        assert_eq!(cfg.business_start, "08:00");
        assert_eq!(cfg.business_end, "18:00");
        assert_eq!(cfg.timezone, "America/New_York");
        assert!(!cfg.auto_send, "auto-send must default to off");
    }

    #[test]
    fn auto_send_parses_when_set() {
        let cfg = SupportConfig::load_from_toml("[support]\nauto_send = true\n");
        assert!(cfg.auto_send);
        let cfg = SupportConfig::load_from_toml("[support]\n");
        assert!(!cfg.auto_send);
    }

    #[test]
    fn full_section_parses() {
        let cfg = SupportConfig::load_from_toml(
            "[support]\nenabled = false\nbusiness_start = \"09:30\"\nbusiness_end = \"17:00\"\ntimezone = \"America/Chicago\"\n",
        );
        assert!(!cfg.enabled);
        assert_eq!(cfg.business_start_hm(), (9, 30));
        assert_eq!(cfg.business_end_hm(), (17, 0));
        assert_eq!(cfg.timezone, "America/Chicago");
    }

    #[test]
    fn garbage_input_yields_defaults() {
        let cfg = SupportConfig::load_from_toml("not toml ][[[");
        assert!(cfg.enabled);
    }

    #[test]
    fn bad_hhmm_falls_back_to_defaults() {
        assert_eq!(SupportConfig::parse_hhmm("25:00"), None);
        assert_eq!(SupportConfig::parse_hhmm("nope"), None);
        let cfg = SupportConfig {
            business_start: "bogus".into(),
            ..Default::default()
        };
        assert_eq!(cfg.business_start_hm(), (8, 0));
    }
}
