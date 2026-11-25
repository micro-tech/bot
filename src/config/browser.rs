use serde::Deserialize;

/// Configuration for the Helix mini-browser (tasks 350+).
///
/// The browser is a headless Firefox driven over WebDriver BiDi (the W3C
/// standard; Firefox-native since CDP was removed in Fx 141). It exists so
/// Helix can drive web pages — first use case: the Gmail OAuth flow, where
/// the browser runs on the R630 so Google's loopback redirect lands where
/// google-mcp listens.
///
/// ```toml
/// [browser]
/// enabled = false              # master switch (default OFF — fail-closed)
/// firefox_binary = ""          # "" = resolve `firefox` from PATH
/// profile_dir = ""             # John's synced Firefox profile (task 353); "" = fresh profile
/// headless = true              # always true on the R630 (no display)
/// geckodriver_url = ""         # "" = thirtyfour manager auto-downloads geckodriver
/// action_timeout_secs = 30     # per-action timeout
/// screenshot_max_bytes = 512000
/// allowed_hosts = []           # navigation allowlist; empty = Google + localhost only
/// gmail_token_path = ""        # "" = $HOME/.config/google-mcp/token.json
/// ```
///
/// Security boundary (tasks 351/353): the browser runs as cobble on the
/// R630 and can reach the LAN; its Google session is John's. Scope is OAuth
/// and user-directed browsing, NOT open-ended LLM web browsing with John's
/// Google session. Navigation is confined to `allowed_hosts` (default:
/// Google + localhost); anything else is refused and the LLM is told to ask
/// the user. The tool never returns password field contents (see
/// `src/io/browser/session.rs`).
#[derive(Debug, Clone, Deserialize)]
pub struct BrowserConfig {
    /// Master switch. When false (the default), the browser is a silent
    /// no-op: no Firefox is spawned and the `browser` tool reports disabled.
    #[serde(default)]
    pub enabled: bool,

    /// Firefox binary path. Empty means resolve `firefox` from PATH
    /// (the Mozilla APT build installed by task 360).
    #[serde(default)]
    pub firefox_binary: String,

    /// Firefox profile dir (John's synced profile, task 353). Empty means
    /// a fresh throwaway profile. The copy itself is John's step, never
    /// the installer's, and the profile is never committed or backed up.
    #[serde(default)]
    pub profile_dir: String,

    /// Headless mode. True by default; the R630 has no display.
    #[serde(default = "default_headless")]
    pub headless: bool,

    /// Explicit geckodriver URL. Empty means thirtyfour's `manager`
    /// feature auto-downloads the matching geckodriver (task 350).
    #[serde(default)]
    pub geckodriver_url: String,

    /// Per-action timeout in seconds (navigate, click, type, screenshot).
    #[serde(default = "default_action_timeout_secs")]
    pub action_timeout_secs: u64,

    /// Soft cap for screenshots (bytes). The 2FA endpoint keeps the
    /// viewport small instead of downscaling (no image crate in the tree).
    #[serde(default = "default_screenshot_max_bytes")]
    pub screenshot_max_bytes: usize,

    /// Navigation allowlist (host suffixes). Empty means the built-in
    /// default: Google hosts + localhost (the OAuth flow). A host is
    /// allowed when it equals an entry or ends with `.` + the entry.
    #[serde(default)]
    pub allowed_hosts: Vec<String>,

    /// Path to the google-mcp OAuth token file (task 354 verification).
    /// Empty means `$HOME/.config/google-mcp/token.json`.
    #[serde(default)]
    pub gmail_token_path: String,
}

fn default_headless() -> bool {
    true
}

fn default_action_timeout_secs() -> u64 {
    30
}

fn default_screenshot_max_bytes() -> usize {
    512_000
}

/// Built-in navigation allowlist when `allowed_hosts` is empty: the Google
/// OAuth hosts (suffix match covers accounts/mail/myaccount.google.com)
/// plus the loopback redirect google-mcp listens on.
pub const DEFAULT_ALLOWED_HOSTS: &[&str] = &["google.com", "localhost", "127.0.0.1"];

/// Minimum Firefox for BiDi (CDP is gone in Fx 141+; BiDi needs 141).
pub const FIREFOX_MIN_VERSION: u32 = 141;

/// Firefox version the build was verified against (task 350).
pub const FIREFOX_TARGET_VERSION: u32 = 159;

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            firefox_binary: String::new(),
            profile_dir: String::new(),
            headless: true,
            geckodriver_url: String::new(),
            action_timeout_secs: default_action_timeout_secs(),
            screenshot_max_bytes: default_screenshot_max_bytes(),
            allowed_hosts: Vec::new(),
            gmail_token_path: String::new(),
        }
    }
}

impl BrowserConfig {
    /// Load the top-level `[browser]` section from a full TOML string.
    /// Missing section (or unparseable input) yields the disabled default.
    /// Fallbacks are logged (task 265) via `config::load_section`.
    pub fn load_from_toml(toml_str: &str) -> Self {
        super::load_section(toml_str, "[browser]", |v| v.get("browser").cloned())
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// The effective navigation allowlist: the configured entries, or the
    /// built-in Google + localhost default when none are configured.
    pub fn effective_allowed_hosts(&self) -> Vec<String> {
        if self.allowed_hosts.is_empty() {
            DEFAULT_ALLOWED_HOSTS
                .iter()
                .map(|s| s.to_string())
                .collect()
        } else {
            self.allowed_hosts.clone()
        }
    }

    /// True when `url` may be navigated to: http(s) only, and its host
    /// matches the effective allowlist (exact or suffix). Unparseable URLs
    /// and non-http(s) schemes are denied.
    pub fn host_allowed(&self, url: &str) -> bool {
        let Ok(parsed) = url::Url::parse(url) else {
            return false;
        };
        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            return false;
        }
        let Some(host) = parsed.host_str() else {
            return false;
        };
        let host = host.to_ascii_lowercase();
        self.effective_allowed_hosts().iter().any(|entry| {
            let entry = entry.to_ascii_lowercase();
            host == entry || host.ends_with(&format!(".{entry}"))
        })
    }

    /// The profile dir with a leading `~/` expanded, or None when unset.
    pub fn resolved_profile_dir(&self) -> Option<std::path::PathBuf> {
        expand_home(&self.profile_dir)
    }

    /// The google-mcp token file path, with `~/` expanded. Defaults to
    /// `$HOME/.config/google-mcp/token.json` when unset.
    pub fn resolved_gmail_token_path(&self) -> std::path::PathBuf {
        if let Some(p) = expand_home(&self.gmail_token_path) {
            return p;
        }
        match std::env::var("HOME") {
            Ok(home) if !home.is_empty() => {
                std::path::PathBuf::from(home).join(".config/google-mcp/token.json")
            }
            _ => std::path::PathBuf::from(".config/google-mcp/token.json"),
        }
    }
}

/// Expand a leading `~/` against $HOME. Returns None for an empty string.
fn expand_home(path: &str) -> Option<std::path::PathBuf> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        if home.is_empty() {
            return Some(std::path::PathBuf::from(trimmed));
        }
        return Some(std::path::PathBuf::from(home).join(rest));
    }
    Some(std::path::PathBuf::from(trimmed))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
[helix]
name = "Helix"

[browser]
enabled = true
firefox_binary = "/usr/bin/firefox"
profile_dir = "~/.mozilla/firefox/helix"
headless = true
geckodriver_url = "http://127.0.0.1:4444"
action_timeout_secs = 45
screenshot_max_bytes = 256000
allowed_hosts = ["google.com", "example.com"]
gmail_token_path = "/home/cobble/.config/google-mcp/token.json"
"#;

    #[test]
    fn parses_full_section() {
        let cfg = BrowserConfig::load_from_toml(FULL);
        assert!(cfg.is_enabled());
        assert_eq!(cfg.firefox_binary, "/usr/bin/firefox");
        assert_eq!(cfg.profile_dir, "~/.mozilla/firefox/helix");
        assert!(cfg.headless);
        assert_eq!(cfg.geckodriver_url, "http://127.0.0.1:4444");
        assert_eq!(cfg.action_timeout_secs, 45);
        assert_eq!(cfg.screenshot_max_bytes, 256_000);
        assert_eq!(cfg.allowed_hosts, vec!["google.com", "example.com"]);
        assert_eq!(
            cfg.gmail_token_path,
            "/home/cobble/.config/google-mcp/token.json"
        );
    }

    #[test]
    fn missing_section_is_disabled_with_safe_defaults() {
        let cfg = BrowserConfig::load_from_toml("[helix]\nname = \"Helix\"\n");
        assert!(!cfg.is_enabled());
        assert!(cfg.headless);
        assert_eq!(cfg.action_timeout_secs, 30);
        assert_eq!(cfg.screenshot_max_bytes, 512_000);
        assert!(cfg.allowed_hosts.is_empty());
        assert!(cfg.resolved_profile_dir().is_none());
    }

    #[test]
    fn garbage_input_is_disabled_by_default() {
        let cfg = BrowserConfig::load_from_toml("this is not toml ][[[");
        assert!(!cfg.is_enabled());
    }

    #[test]
    fn allowlist_defaults_to_google_plus_localhost() {
        let cfg = BrowserConfig::default();
        assert!(cfg.host_allowed("https://accounts.google.com/o/oauth2/v2/auth?x=1"));
        assert!(cfg.host_allowed("https://mail.google.com/mail/u/0/"));
        assert!(cfg.host_allowed("https://myaccount.google.com/"));
        assert!(cfg.host_allowed("http://localhost:51234/callback"));
        assert!(cfg.host_allowed("http://127.0.0.1:51234/"));
        assert!(!cfg.host_allowed("https://evil.example.com/"));
        assert!(!cfg.host_allowed("https://notgoogle.com/"));
        assert!(!cfg.host_allowed("https://google.com.evil.example.com/"));
        assert!(!cfg.host_allowed("file:///etc/passwd"));
        assert!(!cfg.host_allowed("about:config"));
        assert!(!cfg.host_allowed("not a url"));
    }

    #[test]
    fn allowlist_suffix_match_is_exact_or_dot_suffix() {
        let cfg = BrowserConfig {
            allowed_hosts: vec!["example.com".to_string()],
            ..BrowserConfig::default()
        };
        assert!(cfg.host_allowed("https://example.com/"));
        assert!(cfg.host_allowed("https://sub.example.com/"));
        assert!(!cfg.host_allowed("https://badexample.com/"));
        assert!(!cfg.host_allowed("https://example.com.evil.org/"));
    }

    #[test]
    fn profile_dir_expands_home() {
        let cfg = BrowserConfig {
            profile_dir: "~/.mozilla/firefox/helix".to_string(),
            ..BrowserConfig::default()
        };
        let resolved = cfg.resolved_profile_dir().expect("profile dir");
        // HOME is set in the test environment; the expansion must not
        // contain a literal "~".
        assert!(!resolved.to_string_lossy().contains('~'));
        assert!(resolved.ends_with(".mozilla/firefox/helix"));
    }
}
