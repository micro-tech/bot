pub mod manifest;
pub mod mcp;
pub mod acp;
pub mod shell;
pub mod ssh;
pub mod okf;
pub mod project_scanner;
pub mod reasoning;
pub mod socket;

use log::{debug, warn};

/// Parse one config section out of a full TOML document (task 265).
/// Every section loader in this module (plus `[support]`) funnels through
/// here so a fallback is never silent again:
/// - Unparseable TOML -> `warn!` + defaults. The whole document is broken;
///   the main loader fails too, but this must say so instead of quietly
///   running on defaults.
/// - Missing section -> `debug!` + defaults. The normal off state, not a
///   failure — no warning for a section the operator never wrote.
/// - Section present but the wrong shape -> `warn!` + defaults. The
///   operator's settings were ignored; that must be loud.
/// `extract` pulls the section out of the parsed document, so nested
/// sections (`[helix.okf]`) work too.
pub(crate) fn load_section<T>(
    toml_str: &str,
    label: &str,
    extract: impl FnOnce(&toml::Value) -> Option<toml::Value>,
) -> T
where
    T: serde::de::DeserializeOwned + Default,
{
    let value = match toml::from_str::<toml::Value>(toml_str) {
        Ok(v) => v,
        Err(_) => {
            // Task 265 (secrets): do NOT log the parse error — its Display
            // echoes the offending config line, which may hold a secret.
            warn!("config: TOML unparseable, {label} falls back to defaults (fix the syntax error in the config file)");
            return T::default();
        }
    };
    let Some(section) = extract(&value) else {
        debug!("config: no {label} section, using defaults");
        return T::default();
    };
    match section.try_into() {
        Ok(cfg) => cfg,
        Err(_) => {
            // Task 265 (secrets): do NOT log the shape error — serde
            // type-mismatch errors can echo the offending value, which
            // may be a secret (e.g. a string where a number was expected).
            warn!("config: {label} has the wrong shape, falling back to defaults (fix the section in the config file)");
            T::default()
        }
    }
}
