//! Mini-browser installer hook (task 360).
//!
//! Runs during a Helix install and checks for Firefox and everything else
//! the mini-browser needs, installing what is missing, so the R630 install
//! is one step instead of the manual doc in task 356.
//!
//! CHECK-VS-INSTALL DECISION: install, not check-and-report. The installer
//! already re-execs under sudo and runs as root, and already installs
//! things (chat-mcp binary + config in task 295) — an APT install is the
//! same privilege it already exercises. John's note: the installer already
//! asks for the password, so the Firefox install goes in that same bit of
//! code (the sudo re-exec / privileged section) — no separate privilege
//! step. Check-only would just reprint 356's doc and leave him running
//! the same commands by hand.
//!
//! What it does:
//! 1. Firefox check: `firefox --version` (minimum 141, target 159 per
//!    task 350) plus whether the installed Firefox is the Mozilla APT
//!    build or the Ubuntu snap. If missing, a snap, or below 141: add
//!    the Mozilla APT repo (key + sources list + pin, idempotently) and
//!    `apt install firefox`. NEVER install the snap.
//! 2. Geckodriver: do NOT hand-download or pin a version — thirtyfour's
//!    `manager` feature owns it. thirtyfour 0.38 exposes no download-only
//!    API (its `ensure_driver` is private; only `launch` downloads), so
//!    the installer ensures the manager cache dir
//!    (`~/.cache/thirtyfour/drivers`) exists and is owned by the runtime
//!    user; the first browser launch downloads geckodriver.
//! 3. Config: ensure the `[browser]` section exists in the live config
//!    with safe defaults (same pattern as the 295 chat-block append —
//!    only if missing, never overwrite existing values).
//! 4. Ownership: every dir the browser needs is created with runtime-user
//!    ownership (the installer runs as root; root-owned dirs would break
//!    the cobble runtime). Ownership itself is the installer binary's job
//!    (like the 295 hook); this module reports the paths.
//! 5. MUST NOT touch John's personal Firefox profile (the 353 Sync copy
//!    is his step, not the installer's).
//! 6. Failure mode: if the Firefox install fails, warn loudly, record it,
//!    and continue the rest of the install (same fail-soft pattern as
//!    295) — Helix still works without the browser, and the browser tool
//!    reports "browser unavailable" at runtime until Firefox is present.
//!
//! Task 262 independence: this module takes plain paths and edits only
//! the live config file. It never touches the `include_str!` default-config
//! / manifest paths, so it compiles and tests cleanly whether or not
//! 262.4 is decided. The logic lives in this shared lib module (same
//! pattern as the 295 chat-mcp hook) so it tests through the lib even
//! though the installer binary itself is blocked by task 262 on a fresh
//! checkout.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Minimum Firefox for BiDi (CDP is gone in Fx 141+). Target is 159
/// (current stable, Nov 2026 — task 350); the minimum is the decision
/// threshold, the target lives in `config::browser::FIREFOX_TARGET_VERSION`.
pub const FIREFOX_MIN_VERSION: u32 = 141;

/// Mozilla APT signing-key URL (documented Mozilla procedure).
pub const MOZILLA_KEY_URL: &str = "https://packages.mozilla.org/apt/repo-signing-key.gpg";

/// The Mozilla APT sources line (Signed-By the keyring below).
pub const MOZILLA_SOURCES_LINE: &str = "deb [signed-by=/etc/apt/keyrings/packages.mozilla.org.asc] https://packages.mozilla.org/apt mozilla main";

/// APT pin so Mozilla's Firefox wins over Ubuntu's snap-shim package.
pub const MOZILLA_PIN: &str = "Package: *\nPin: origin packages.mozilla.org\nPin-Priority: 1000\n";

/// Default `[browser]` section the installer appends when the live config
/// has none. Fail-closed: the browser stays disabled until John enables
/// it (after the profile copy in task 353).
pub const BROWSER_CONFIG_DEFAULTS: &str =
    "[browser]\nenabled = false\nheadless = true\naction_timeout_secs = 30\n";

/// What the Firefox check found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirefoxStatus {
    /// No `firefox` on PATH.
    Missing,
    /// The installed Firefox is the Ubuntu snap (breaks automation and
    /// profile paths — must be replaced by the Mozilla APT build).
    Snap,
    /// A Mozilla APT build, but older than the minimum.
    TooOld(u32),
    /// A Mozilla APT build at or above the minimum.
    Ok(u32),
}

/// What the installer should do about Firefox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirefoxAction {
    /// Install (or replace) via the Mozilla APT repo.
    Install,
    /// Already fine; nothing to do.
    AlreadyOk,
}

impl FirefoxStatus {
    /// The install decision for this status.
    pub fn action(self) -> FirefoxAction {
        match self {
            FirefoxStatus::Missing | FirefoxStatus::Snap | FirefoxStatus::TooOld(_) => {
                FirefoxAction::Install
            }
            FirefoxStatus::Ok(_) => FirefoxAction::AlreadyOk,
        }
    }
}

/// All paths one browser-deps install touches.
#[derive(Debug, Clone)]
pub struct BrowserLayout {
    /// `<primary_runtime_dir>/config.toml` (the live Helix config).
    pub helix_config_path: PathBuf,
    /// The browser profile dir (`<home>/.mozilla/firefox/helix`).
    pub profile_dir: PathBuf,
    /// The thirtyfour manager cache dir (`<home>/.cache/thirtyfour/drivers`).
    pub manager_cache_dir: PathBuf,
}

/// Report for one hook run, for installer logging.
#[derive(Debug, Clone)]
pub struct BrowserInstallReport {
    pub layout: BrowserLayout,
    pub firefox_status: FirefoxStatus,
    pub firefox_installed: bool,
    pub repo_added: bool,
    pub browser_section_appended: bool,
    /// Dirs the installer binary must chown to the runtime user.
    pub dirs_to_own: Vec<PathBuf>,
}

/// Home dir for a runtime user. Root installs land in `/root`; every other
/// user lands in `/home/<user>`. (Same convention as chat_mcp_install.)
pub fn home_dir_for_user(user: &str) -> PathBuf {
    if user == "root" {
        PathBuf::from("/root")
    } else {
        PathBuf::from(format!("/home/{user}"))
    }
}

/// Layout for a Helix install owned by `user` with live config in
/// `primary_runtime_dir` (e.g. `/home/cobble/helix`).
pub fn layout_for_user(user: &str, primary_runtime_dir: &str) -> BrowserLayout {
    let home_dir = home_dir_for_user(user);
    BrowserLayout {
        helix_config_path: Path::new(primary_runtime_dir).join("config.toml"),
        profile_dir: home_dir.join(".mozilla/firefox/helix"),
        manager_cache_dir: home_dir.join(".cache/thirtyfour/drivers"),
    }
}

// ── Pure helpers (unit-tested) ────────────────────────────────────────

/// Parse a Firefox major version from `firefox --version` output
/// ("Mozilla Firefox 159.0" -> 159). None when unparseable.
pub fn parse_firefox_version(output: &str) -> Option<u32> {
    let after = output.split("Firefox ").nth(1)?;
    let num: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    num.parse().ok()
}

/// True when the installed Firefox is the Ubuntu snap, given the
/// `snap list firefox` result and the resolved firefox binary path.
/// Either signal alone is enough: snap's `firefox` package, or a binary
/// living under /snap/.
pub fn detect_snap_firefox(snap_list_shows_firefox: bool, firefox_realpath: &str) -> bool {
    snap_list_shows_firefox || firefox_realpath.starts_with("/snap/")
}

/// Combine a version parse + snap detection into a [`FirefoxStatus`].
/// `version_output` is `firefox --version` stdout, or None when the
/// binary is missing.
pub fn firefox_status_from(
    version_output: Option<&str>,
    snap_list_shows_firefox: bool,
    firefox_realpath: &str,
) -> FirefoxStatus {
    let Some(output) = version_output else {
        return FirefoxStatus::Missing;
    };
    if detect_snap_firefox(snap_list_shows_firefox, firefox_realpath) {
        return FirefoxStatus::Snap;
    }
    match parse_firefox_version(output) {
        Some(v) if v >= FIREFOX_MIN_VERSION => FirefoxStatus::Ok(v),
        Some(v) => FirefoxStatus::TooOld(v),
        None => FirefoxStatus::Missing,
    }
}

/// True when the TOML text already contains a live (non-commented)
/// `[browser]` header.
pub fn has_browser_section(toml_text: &str) -> bool {
    toml_text
        .lines()
        .map(str::trim)
        .any(|line| line == "[browser]")
}

/// New TOML text with the `[browser]` section appended, or `None` when
/// one is already present (idempotent: a re-run changes nothing).
/// Never duplicates the section; preserves the rest byte-for-byte.
pub fn append_browser_section(toml_text: &str) -> Option<String> {
    if has_browser_section(toml_text) {
        return None;
    }
    let mut out = toml_text.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.is_empty() && !out.ends_with("\n\n") {
        out.push('\n');
    }
    out.push_str(BROWSER_CONFIG_DEFAULTS);
    Some(out)
}

/// True when an APT sources file already contains the Mozilla repo line.
pub fn mozilla_repo_present(sources_text: &str) -> bool {
    sources_text
        .lines()
        .map(str::trim)
        .any(|line| line == MOZILLA_SOURCES_LINE)
}

/// Back up `path` to `<path>.bak-<unix-epoch>` before it gets overwritten.
/// No-op when the file does not exist yet. A failed backup only warns;
/// callers decide whether to proceed. (Mirrors the installer's own helper;
/// the lib module cannot import from the installer binary.)
fn backup_existing(path: &Path) {
    if !path.exists() {
        return;
    }
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = PathBuf::from(format!("{}.bak-{}", path.display(), epoch));
    match fs::copy(path, &backup) {
        Ok(_) => eprintln!(
            "browser: backed up {} -> {}",
            path.display(),
            backup.display()
        ),
        Err(e) => eprintln!(
            "browser: WARNING: could not back up {}: {} — proceeding with overwrite",
            path.display(),
            e
        ),
    }
}

/// Ensure the `[browser]` section exists in the live Helix config.
/// Returns true when the section was appended (false = already present).
/// The config is backed up first; an existing section is never touched.
pub fn ensure_browser_config(helix_config_path: &Path) -> Result<bool, String> {
    let text = fs::read_to_string(helix_config_path).map_err(|e| {
        format!(
            "browser: cannot read Helix config {}: {}",
            helix_config_path.display(),
            e
        )
    })?;
    match append_browser_section(&text) {
        None => Ok(false),
        Some(new_text) => {
            backup_existing(helix_config_path);
            fs::write(helix_config_path, new_text).map_err(|e| {
                format!(
                    "browser: cannot write {}: {}",
                    helix_config_path.display(),
                    e
                )
            })?;
            Ok(true)
        }
    }
}

/// Create the browser dirs (profile + manager cache). Ownership is the
/// installer binary's job (it chowns the reported paths); this module
/// only creates them. Returns the created dirs.
pub fn ensure_browser_dirs(layout: &BrowserLayout) -> Result<Vec<PathBuf>, String> {
    let mut created = Vec::new();
    for dir in [&layout.profile_dir, &layout.manager_cache_dir] {
        fs::create_dir_all(dir)
            .map_err(|e| format!("browser: cannot create {}: {}", dir.display(), e))?;
        created.push(dir.clone());
    }
    Ok(created)
}

// ── System operations (seam for tests) ─────────────────────────────────

/// The system commands the installer needs (Firefox check, APT, snap).
/// The real impl shells out; tests inject a stub. This keeps the
/// orchestrator unit-testable without touching the real system.
pub trait SystemOps {
    /// `firefox --version` stdout, or None when the binary is missing.
    fn firefox_version(&self) -> Option<String>;
    /// True when `snap list firefox` shows the snap installed.
    fn snap_list_firefox(&self) -> bool;
    /// Resolved path of the `firefox` binary ("" when missing).
    fn firefox_realpath(&self) -> String;
    /// Ensure the Mozilla APT repo (key + sources + pin + apt update).
    /// Returns true when the repo was added (false = already present).
    fn ensure_mozilla_repo(&self) -> Result<bool, String>;
    /// `apt-get install -y firefox`.
    fn install_firefox(&self) -> Result<(), String>;
}

/// The real system operations (shells out; runs as root in the installer).
pub struct RealSystemOps;

fn run_cmd(program: &str, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("browser: failed to execute {program}: {e}"))
}

fn output_snippet(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{stderr}\n{stdout}");
    combined
        .chars()
        .take(1000)
        .collect::<String>()
        .trim()
        .to_string()
}

impl SystemOps for RealSystemOps {
    fn firefox_version(&self) -> Option<String> {
        let out = Command::new("firefox").arg("--version").output().ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn snap_list_firefox(&self) -> bool {
        match Command::new("snap").args(["list", "firefox"]).output() {
            Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
                .lines()
                .any(|l| l.split_whitespace().next() == Some("firefox")),
            _ => false,
        }
    }

    fn firefox_realpath(&self) -> String {
        let out = match Command::new("readlink").args(["-f", "firefox"]).output() {
            Ok(o) if o.status.success() => o,
            _ => match Command::new("which").arg("firefox").output() {
                Ok(o) if o.status.success() => o,
                _ => return String::new(),
            },
        };
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn ensure_mozilla_repo(&self) -> Result<bool, String> {
        let sources_path = Path::new("/etc/apt/sources.list.d/mozilla.list");
        if let Ok(existing) = fs::read_to_string(sources_path)
            && mozilla_repo_present(&existing)
        {
            return Ok(false);
        }
        // Keyring dir (documented Mozilla procedure).
        fs::create_dir_all("/etc/apt/keyrings")
            .map_err(|e| format!("browser: cannot create /etc/apt/keyrings: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions("/etc/apt/keyrings", fs::Permissions::from_mode(0o755));
        }
        // Signing key: curl first, wget as fallback (both standard on
        // Ubuntu). The key file is written exactly as Mozilla distributes
        // it; the sources line references it via Signed-By.
        let key_path = "/etc/apt/keyrings/packages.mozilla.org.asc";
        let downloaded = run_cmd("curl", &["-fsSL", MOZILLA_KEY_URL, "-o", key_path])
            .map(|o| o.status.success())
            .unwrap_or(false)
            || run_cmd("wget", &["-q", MOZILLA_KEY_URL, "-O", key_path])
                .map(|o| o.status.success())
                .unwrap_or(false);
        if !downloaded {
            return Err(
                "browser: could not download the Mozilla APT signing key (curl and wget both failed — check network)"
                    .to_string(),
            );
        }
        fs::write(sources_path, format!("{MOZILLA_SOURCES_LINE}\n"))
            .map_err(|e| format!("browser: cannot write {}: {}", sources_path.display(), e))?;
        fs::write("/etc/apt/preferences.d/mozilla", MOZILLA_PIN)
            .map_err(|e| format!("browser: cannot write /etc/apt/preferences.d/mozilla: {e}"))?;
        let update = run_cmd("apt-get", &["update"])?;
        if !update.status.success() {
            return Err(format!(
                "browser: apt-get update failed: {}",
                output_snippet(&update)
            ));
        }
        Ok(true)
    }

    fn install_firefox(&self) -> Result<(), String> {
        let out = run_cmd("apt-get", &["install", "-y", "firefox"])?;
        if !out.status.success() {
            return Err(format!(
                "browser: apt-get install -y firefox failed: {}",
                output_snippet(&out)
            ));
        }
        Ok(())
    }
}

// ── Orchestrator ────────────────────────────────────────────────────────

/// Full hook run against an explicit layout and system ops (what tests
/// call, and what the installer wraps). Order: Firefox check -> install
/// if needed -> config section -> dirs. The Helix config is backed up
/// before any modification; a no-op run writes nothing and backs up
/// nothing. A Firefox install failure is reported as an error to the
/// caller (the installer binary treats it as a loud warning and
/// continues the rest of the install — fail-soft, task 295 pattern).
pub fn install_browser_deps_at(
    layout: &BrowserLayout,
    ops: &dyn SystemOps,
) -> Result<BrowserInstallReport, String> {
    let status = firefox_status_from(
        ops.firefox_version().as_deref(),
        ops.snap_list_firefox(),
        &ops.firefox_realpath(),
    );
    let mut firefox_installed = false;
    let mut repo_added = false;
    if status.action() == FirefoxAction::Install {
        repo_added = ops.ensure_mozilla_repo()?;
        ops.install_firefox()?;
        firefox_installed = true;
        // Re-check: the install must have produced a usable Firefox.
        let after = firefox_status_from(
            ops.firefox_version().as_deref(),
            ops.snap_list_firefox(),
            &ops.firefox_realpath(),
        );
        if after.action() == FirefoxAction::Install {
            return Err(format!(
                "browser: Firefox install ran but the check still reports {after:?} — not continuing silently"
            ));
        }
    }
    let browser_section_appended = ensure_browser_config(&layout.helix_config_path)?;
    let dirs_to_own = ensure_browser_dirs(layout)?;
    Ok(BrowserInstallReport {
        layout: layout.clone(),
        firefox_status: status,
        firefox_installed,
        repo_added,
        browser_section_appended,
        dirs_to_own,
    })
}

/// Hook entry point for the installer binary: default layout for `user`
/// with the live Helix config in `primary_runtime_dir`.
pub fn install_browser_deps(
    user: &str,
    primary_runtime_dir: &str,
) -> Result<BrowserInstallReport, String> {
    let layout = layout_for_user(user, primary_runtime_dir);
    install_browser_deps_at(&layout, &RealSystemOps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// Unique scratch dir per test under the package target dir (never
    /// /tmp: the bot suite keeps its test scratch under target/).
    fn unique_dir(tag: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-tmp-browser-install");
        let dir = root.join(format!("{}-{}-{}-{}", tag, std::process::id(), nanos, n));
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, content).expect("write fixture");
    }

    /// Stub SystemOps: canned Firefox state, records what the hook asked
    /// it to do. `installed_version` is what `firefox --version` reports
    /// AFTER a successful install (simulating the re-check).
    struct FakeSystemOps {
        version_before: Option<String>,
        version_after: Option<String>,
        snap: bool,
        calls: Mutex<Vec<&'static str>>,
    }

    impl FakeSystemOps {
        fn new(version: Option<&str>, snap: bool) -> Self {
            Self {
                version_before: version.map(str::to_string),
                version_after: Some("Mozilla Firefox 159.0".to_string()),
                snap,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<&'static str> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl SystemOps for FakeSystemOps {
        fn firefox_version(&self) -> Option<String> {
            let calls = self.calls.lock().unwrap();
            // Before install: the canned version; after an install call:
            // the post-install version.
            if calls.contains(&"install_firefox") {
                self.version_after.clone()
            } else {
                self.version_before.clone()
            }
        }

        fn snap_list_firefox(&self) -> bool {
            // After an install, the snap is gone (the Mozilla build
            // replaced it).
            let calls = self.calls.lock().unwrap();
            if calls.contains(&"install_firefox") {
                false
            } else {
                self.snap
            }
        }

        fn firefox_realpath(&self) -> String {
            // After an install, the binary is the Mozilla APT build, not
            // the snap path.
            let calls = self.calls.lock().unwrap();
            if calls.contains(&"install_firefox") {
                "/usr/bin/firefox".to_string()
            } else if self.snap {
                "/snap/bin/firefox".to_string()
            } else {
                "/usr/bin/firefox".to_string()
            }
        }

        fn ensure_mozilla_repo(&self) -> Result<bool, String> {
            self.calls.lock().unwrap().push("ensure_mozilla_repo");
            Ok(true)
        }

        fn install_firefox(&self) -> Result<(), String> {
            self.calls.lock().unwrap().push("install_firefox");
            Ok(())
        }
    }

    fn layout_in(root: &Path) -> BrowserLayout {
        BrowserLayout {
            helix_config_path: root.join("helix/config.toml"),
            profile_dir: root.join("home/.mozilla/firefox/helix"),
            manager_cache_dir: root.join("home/.cache/thirtyfour/drivers"),
        }
    }

    const HELIX_MINIMAL: &str = "[helix]\nname = \"Helix\"\n";

    #[test]
    fn version_parse_handles_real_output() {
        assert_eq!(parse_firefox_version("Mozilla Firefox 159.0"), Some(159));
        assert_eq!(parse_firefox_version("Mozilla Firefox 159.0.1"), Some(159));
        assert_eq!(parse_firefox_version("Mozilla Firefox 141.0"), Some(141));
        assert_eq!(parse_firefox_version(""), None);
        assert_eq!(parse_firefox_version("Firefox"), None);
    }

    #[test]
    fn status_decisions() {
        assert_eq!(firefox_status_from(None, false, ""), FirefoxStatus::Missing);
        assert_eq!(
            firefox_status_from(Some("Mozilla Firefox 159.0"), false, "/usr/bin/firefox"),
            FirefoxStatus::Ok(159)
        );
        assert_eq!(
            firefox_status_from(Some("Mozilla Firefox 140.0"), false, "/usr/bin/firefox"),
            FirefoxStatus::TooOld(140)
        );
        assert_eq!(
            firefox_status_from(Some("Mozilla Firefox 159.0"), true, "/usr/bin/firefox"),
            FirefoxStatus::Snap
        );
        assert_eq!(
            firefox_status_from(Some("Mozilla Firefox 159.0"), false, "/snap/bin/firefox"),
            FirefoxStatus::Snap
        );
        assert_eq!(FirefoxStatus::Ok(159).action(), FirefoxAction::AlreadyOk);
        assert_eq!(FirefoxStatus::Missing.action(), FirefoxAction::Install);
        assert_eq!(FirefoxStatus::TooOld(140).action(), FirefoxAction::Install);
    }

    #[test]
    fn browser_section_appended_when_missing() {
        let out = append_browser_section(HELIX_MINIMAL).expect("must append");
        assert!(has_browser_section(&out));
        assert!(
            out.contains("enabled = false"),
            "fail-closed default, got: {out}"
        );
        assert!(out.contains("headless = true"));
        assert_eq!(out.matches("[browser]").count(), 1, "never duplicated");
    }

    #[test]
    fn browser_section_noop_when_present() {
        let with_section = format!("{HELIX_MINIMAL}\n[BROWSER]\n");
        // Case-sensitive: [BROWSER] is not [browser].
        assert!(append_browser_section(&with_section).is_some());
        let with_section = format!("{HELIX_MINIMAL}\n[browser]\nenabled = true\n");
        assert!(append_browser_section(&with_section).is_none());
        let commented = format!("{HELIX_MINIMAL}\n# [browser]\n# enabled = true\n");
        assert!(append_browser_section(&commented).is_some());
    }

    #[test]
    fn mozilla_repo_present_detection() {
        assert!(mozilla_repo_present(&format!("{MOZILLA_SOURCES_LINE}\n")));
        assert!(!mozilla_repo_present(
            "deb http://archive.ubuntu.com/ubuntu noble main\n"
        ));
        assert!(!mozilla_repo_present(""));
    }

    #[test]
    fn orchestrator_missing_firefox_installs_and_appends_config() {
        let dir = unique_dir("orch-missing");
        let layout = layout_in(&dir);
        write(&layout.helix_config_path, HELIX_MINIMAL);
        let ops = FakeSystemOps::new(None, false);
        let report = install_browser_deps_at(&layout, &ops).expect("missing firefox must install");
        assert_eq!(report.firefox_status, FirefoxStatus::Missing);
        assert!(report.firefox_installed);
        assert!(report.repo_added);
        assert!(report.browser_section_appended);
        assert!(layout.profile_dir.exists());
        assert!(layout.manager_cache_dir.exists());
        assert_eq!(report.dirs_to_own.len(), 2);
        let helix = fs::read_to_string(&layout.helix_config_path).expect("read helix config");
        assert!(has_browser_section(&helix));
        assert!(ops.calls().contains(&"install_firefox"));
        cleanup(&dir);
    }

    #[test]
    fn orchestrator_snap_firefox_is_replaced() {
        let dir = unique_dir("orch-snap");
        let layout = layout_in(&dir);
        write(&layout.helix_config_path, HELIX_MINIMAL);
        let ops = FakeSystemOps::new(Some("Mozilla Firefox 159.0"), true);
        let report = install_browser_deps_at(&layout, &ops).expect("snap must be replaced");
        assert_eq!(report.firefox_status, FirefoxStatus::Snap);
        assert!(report.firefox_installed);
        assert!(ops.calls().contains(&"install_firefox"));
        cleanup(&dir);
    }

    #[test]
    fn orchestrator_ok_firefox_skips_install() {
        let dir = unique_dir("orch-ok");
        let layout = layout_in(&dir);
        write(&layout.helix_config_path, HELIX_MINIMAL);
        let ops = FakeSystemOps::new(Some("Mozilla Firefox 159.0"), false);
        let report = install_browser_deps_at(&layout, &ops).expect("ok firefox must not install");
        assert_eq!(report.firefox_status, FirefoxStatus::Ok(159));
        assert!(!report.firefox_installed);
        assert!(!report.repo_added);
        assert!(!ops.calls().contains(&"install_firefox"));
        assert!(!ops.calls().contains(&"ensure_mozilla_repo"));
        assert!(report.browser_section_appended);
        cleanup(&dir);
    }

    #[test]
    fn orchestrator_idempotent_rerun() {
        let dir = unique_dir("orch-rerun");
        let layout = layout_in(&dir);
        write(&layout.helix_config_path, HELIX_MINIMAL);
        let ops = FakeSystemOps::new(Some("Mozilla Firefox 159.0"), false);
        let first = install_browser_deps_at(&layout, &ops).expect("first run");
        assert!(first.browser_section_appended);
        let second = install_browser_deps_at(&layout, &ops).expect("second run");
        assert!(
            !second.browser_section_appended,
            "re-run must not duplicate the section"
        );
        let helix = fs::read_to_string(&layout.helix_config_path).expect("read helix config");
        assert_eq!(helix.matches("[browser]").count(), 1);
        cleanup(&dir);
    }

    #[test]
    fn orchestrator_missing_helix_config_errors() {
        let dir = unique_dir("orch-nohelix");
        let layout = layout_in(&dir);
        // No helix config written: the config step must error (the dirs
        // were already created; the Firefox decision is recorded in the
        // error path via the report being absent).
        let ops = FakeSystemOps::new(Some("Mozilla Firefox 159.0"), false);
        let err =
            install_browser_deps_at(&layout, &ops).expect_err("missing helix config must error");
        assert!(err.contains("Helix config"), "got: {err}");
        cleanup(&dir);
    }
}
