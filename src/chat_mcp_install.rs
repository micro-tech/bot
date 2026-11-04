//! chat-mcp installer hook (task 295).
//!
//! Runs during a Helix install and installs/updates chat-mcp, the
//! standalone MCP server that lets Helix chat through the hub:
//!   1. the `chat-mcp` binary into `<home>/.local/bin/chat-mcp`, built from
//!      the micro-tech/chat-mcp repo source (clone on first install, fast-
//!      forward pull + rebuild on later installs);
//!   2. the config dir `<home>/.config/chat-mcp`;
//!   3. the `[[mcp.servers]]` chat block in the Helix `config.toml`, if
//!      missing (the live config is backed up first, and the block and the
//!      `[mcp]` header are never duplicated).
//!
//! Token safety (hard rule): this module never embeds, prints, or requires
//! a token. An existing chat-mcp config is preserved byte-identical. A
//! missing config is created with `hub_url` and `watch_timeout_secs` only
//! and NO token key, perms 0600; the operator adds the existing helix
//! token on the machine, never in chat.
//!
//! Task 262 independence: this module takes plain paths and edits only the
//! live config file. It never touches the `include_str!` default-config /
//! manifest paths that task 262 covers, so it compiles and tests cleanly
//! whether or not 262.4 is decided. File ownership (chown to the runtime
//! user) is the installer binary's job, not this module's, and a failure
//! here must not fail the Helix install: the installer call site treats
//! any error as a warning and continues.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Upstream chat-mcp repo. Building from source (rather than shipping a
/// binary) keeps the installer working as chat-mcp evolves and avoids
/// vendoring a multi-megabyte binary into the bot repo.
pub const CHAT_MCP_REPO_URL: &str = "https://github.com/micro-tech/chat-mcp";

/// Hub the chat-mcp config points at. Matches the live R630 config.
pub const HUB_URL: &str = "https://chat.micro-tech.xyz";

/// Long-poll watch timeout. Matches the live R630 config.
pub const WATCH_TIMEOUT_SECS: u32 = 30;

/// What `ensure_chat_mcp_config` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigOutcome {
    /// The config already existed and was left completely untouched.
    Preserved,
    /// The config was created fresh (no token key; see module docs).
    Created,
}

/// All paths one chat-mcp install touches.
#[derive(Debug, Clone)]
pub struct ChatMcpLayout {
    /// `<home>/.local/bin/chat-mcp`.
    pub bin_path: PathBuf,
    /// `<home>/.config/chat-mcp`.
    pub config_dir: PathBuf,
    /// `<home>/.config/chat-mcp/config.toml`.
    pub config_path: PathBuf,
    /// `<primary_runtime_dir>/config.toml` (the live Helix config).
    pub helix_config_path: PathBuf,
    /// Checkout dir for the chat-mcp source (default `<home>/chat-mcp`).
    pub source_dir: PathBuf,
}

/// Report for one hook run, for installer logging.
#[derive(Debug, Clone)]
pub struct InstallReport {
    pub layout: ChatMcpLayout,
    pub binary_installed: bool,
    pub config: ConfigOutcome,
    pub block_appended: bool,
}

/// Home dir for a runtime user. Root installs land in `/root`; every other
/// user lands in `/home/<user>`. (The `helix` system user has no dedicated
/// home convention in this installer, so `/home/helix` is the default.)
pub fn home_dir_for_user(user: &str) -> PathBuf {
    if user == "root" {
        PathBuf::from("/root")
    } else {
        PathBuf::from(format!("/home/{user}"))
    }
}

/// Layout for a Helix install owned by `user` with live config in
/// `primary_runtime_dir` (e.g. `/home/cobble/helix`).
pub fn layout_for_user(user: &str, primary_runtime_dir: &str) -> ChatMcpLayout {
    let home_dir = home_dir_for_user(user);
    ChatMcpLayout {
        bin_path: home_dir.join(".local/bin/chat-mcp"),
        config_dir: home_dir.join(".config/chat-mcp"),
        config_path: home_dir.join(".config/chat-mcp/config.toml"),
        helix_config_path: Path::new(primary_runtime_dir).join("config.toml"),
        source_dir: home_dir.join("chat-mcp"),
    }
}

/// The `[[mcp.servers]]` block for chat-mcp, matching the schema in
/// `src/config/mcp.rs` (`name` + `command`; chat-mcp takes no args and
/// reads its config file over stdio).
pub fn chat_server_block(bin_path: &Path) -> String {
    format!(
        "[[mcp.servers]]\nname = \"chat\"\ncommand = \"{}\"\n",
        bin_path.display()
    )
}

/// True when the TOML text already contains a live (non-commented)
/// `[[mcp.servers]]` entry named `chat`.
pub fn has_chat_server(toml_text: &str) -> bool {
    let mut in_server_block = false;
    for raw_line in toml_text.lines() {
        let line = raw_line.trim();
        if line.starts_with('#') {
            continue;
        }
        if line == "[[mcp.servers]]" {
            in_server_block = true;
            continue;
        }
        if line.starts_with('[') {
            in_server_block = false;
            continue;
        }
        if in_server_block && line_is_name(line, "chat") {
            return true;
        }
    }
    false
}

/// True when the TOML text contains a live (non-commented) `[mcp]` header.
/// A `[[mcp.servers]]` line alone does not count.
pub fn has_mcp_header(toml_text: &str) -> bool {
    toml_text
        .lines()
        .map(str::trim)
        .any(|line| line == "[mcp]")
}

/// True when `line` is a `name = <value>` assignment for `value`, accepting
/// single or double quotes and trailing comments.
fn line_is_name(line: &str, value: &str) -> bool {
    let Some((key, raw_val)) = line.split_once('=') else {
        return false;
    };
    if key.trim() != "name" {
        return false;
    }
    let val = raw_val.trim();
    let unquoted = val
        .strip_prefix('"')
        .and_then(|v| v.split('"').next())
        .or_else(|| val.strip_prefix('\'').and_then(|v| v.split('\'').next()));
    unquoted == Some(value)
}

/// New TOML text with the chat block appended, or `None` when the block is
/// already present (idempotent: a re-run changes nothing). Adds the `[mcp]`
/// header first when the config has no live one, and never duplicates it.
pub fn append_chat_server(toml_text: &str, bin_path: &Path) -> Option<String> {
    if has_chat_server(toml_text) {
        return None;
    }
    let mut out = toml_text.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !has_mcp_header(toml_text) {
        out.push_str("[mcp]\nenabled = true\n");
    }
    if !out.is_empty() && !out.ends_with("\n\n") {
        out.push('\n');
    }
    out.push_str(&chat_server_block(bin_path));
    Some(out)
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
        Ok(_) => eprintln!("chat-mcp: backed up {} -> {}", path.display(), backup.display()),
        Err(e) => eprintln!(
            "chat-mcp: WARNING: could not back up {}: {} — proceeding with overwrite",
            path.display(),
            e
        ),
    }
}

/// Ensure the chat-mcp config dir and file exist. An existing file is
/// preserved byte-identical (its token, if any, is never read or rewritten).
/// A missing file is created with `hub_url` and `watch_timeout_secs` only
/// and NO token key, perms 0600 on Unix.
pub fn ensure_chat_mcp_config(
    config_dir: &Path,
    config_path: &Path,
) -> Result<ConfigOutcome, String> {
    fs::create_dir_all(config_dir)
        .map_err(|e| format!("chat-mcp: cannot create {}: {}", config_dir.display(), e))?;
    if config_path.exists() {
        return Ok(ConfigOutcome::Preserved);
    }
    let template = format!(
        "# chat-mcp configuration, created by the Helix installer (task 295).\n\
         # No token is written here by the installer, ever. The operator adds\n\
         # the token ON THE MACHINE (never in chat) with the existing helix token:\n\
         #   token = \"<existing-helix-token-entered-on-machine>\"\n\
         # Until then chat-mcp has no credential and its tools stay unusable.\n\
         hub_url = \"{HUB_URL}\"\n\
         watch_timeout_secs = {WATCH_TIMEOUT_SECS}\n"
    );
    fs::write(config_path, template.as_bytes())
        .map_err(|e| format!("chat-mcp: cannot write {}: {}", config_path.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(config_path, fs::Permissions::from_mode(0o600)).map_err(|e| {
            format!(
                "chat-mcp: cannot set 0600 on {}: {}",
                config_path.display(),
                e
            )
        })?;
    }
    Ok(ConfigOutcome::Created)
}

/// Resolve the chat-mcp source checkout. An existing checkout (a dir with a
/// `Cargo.toml`) is reused; when it is a git checkout, a fast-forward pull
/// is attempted but a pull failure only warns and the existing source is
/// still used. Otherwise the repo is cloned. The clone path is the one
/// part of this module with no unit test (it needs network + git); the
/// build, config, and block steps below are all tested on local fixtures.
pub fn resolve_source_dir(preferred: &Path, repo_url: &str) -> Result<PathBuf, String> {
    if preferred.join("Cargo.toml").exists() {
        if preferred.join(".git").exists() {
            let out = Command::new("git")
                .args(["-C", &preferred.to_string_lossy(), "pull", "--ff-only"])
                .output();
            match out {
                Ok(o) if o.status.success() => {}
                Ok(o) => eprintln!(
                    "chat-mcp: WARNING: git pull in {} failed ({}), using existing source",
                    preferred.display(),
                    String::from_utf8_lossy(&o.stderr).trim()
                ),
                Err(e) => eprintln!(
                    "chat-mcp: WARNING: could not run git pull in {}: {}, using existing source",
                    preferred.display(),
                    e
                ),
            }
        }
        return Ok(preferred.to_path_buf());
    }
    if let Some(parent) = preferred.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "chat-mcp: cannot create {}: {}",
                parent.display(),
                e
            )
        })?;
    }
    let status = Command::new("git")
        .args(["clone", repo_url, &preferred.to_string_lossy()])
        .status()
        .map_err(|e| format!("chat-mcp: failed to execute git clone: {e}"))?;
    if !status.success() {
        return Err(format!(
            "chat-mcp: git clone {repo_url} into {} failed (exit {})",
            preferred.display(),
            status.code().unwrap_or(-1)
        ));
    }
    if !preferred.join("Cargo.toml").exists() {
        return Err(format!(
            "chat-mcp: cloned {} has no Cargo.toml, refusing to build",
            preferred.display()
        ));
    }
    Ok(preferred.to_path_buf())
}

/// Build chat-mcp from `source_dir` and install the release binary to
/// `bin_path` (executable bit set on Unix). The target dir lives under the
/// source checkout, never under /tmp.
pub fn build_and_install_binary(source_dir: &Path, bin_path: &Path) -> Result<(), String> {
    let manifest = source_dir.join("Cargo.toml");
    if !manifest.exists() {
        return Err(format!(
            "chat-mcp: no Cargo.toml in {}, refusing to build",
            source_dir.display()
        ));
    }
    let out = Command::new("cargo")
        .args([
            "build",
            "--release",
            "--manifest-path",
            &manifest.to_string_lossy(),
        ])
        .output()
        .map_err(|e| format!("chat-mcp: failed to execute cargo build: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let snippet: String = stderr.chars().take(2000).collect();
        return Err(format!(
            "chat-mcp: cargo build --release failed for {}: {}",
            source_dir.display(),
            snippet.trim()
        ));
    }
    let built = source_dir.join("target/release/chat-mcp");
    if !built.exists() {
        return Err(format!(
            "chat-mcp: build succeeded but {} is missing",
            built.display()
        ));
    }
    if let Some(parent) = bin_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "chat-mcp: cannot create {}: {}",
                parent.display(),
                e
            )
        })?;
    }
    fs::copy(&built, bin_path).map_err(|e| {
        format!(
            "chat-mcp: cannot install {} -> {}: {}",
            built.display(),
            bin_path.display(),
            e
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(bin_path, fs::Permissions::from_mode(0o755)).map_err(|e| {
            format!(
                "chat-mcp: cannot set 0755 on {}: {}",
                bin_path.display(),
                e
            )
        })?;
    }
    Ok(())
}

/// Full hook run against an explicit layout (what tests call, and what the
/// installer wraps). Order: source -> binary -> chat-mcp config -> Helix
/// config block. The Helix config is backed up before any modification; a
/// no-op run (block already present) writes nothing and backs up nothing.
pub fn install_chat_mcp_at(layout: &ChatMcpLayout) -> Result<InstallReport, String> {
    let source = resolve_source_dir(&layout.source_dir, CHAT_MCP_REPO_URL)?;
    build_and_install_binary(&source, &layout.bin_path)?;
    let config = ensure_chat_mcp_config(&layout.config_dir, &layout.config_path)?;
    let helix_text = fs::read_to_string(&layout.helix_config_path).map_err(|e| {
        format!(
            "chat-mcp: cannot read Helix config {}: {} (binary and chat-mcp config are already in place)",
            layout.helix_config_path.display(),
            e
        )
    })?;
    let block_appended = match append_chat_server(&helix_text, &layout.bin_path) {
        None => false,
        Some(new_text) => {
            backup_existing(&layout.helix_config_path);
            fs::write(&layout.helix_config_path, new_text).map_err(|e| {
                format!(
                    "chat-mcp: cannot write {}: {}",
                    layout.helix_config_path.display(),
                    e
                )
            })?;
            true
        }
    };
    Ok(InstallReport {
        layout: layout.clone(),
        binary_installed: true,
        config,
        block_appended,
    })
}

/// Hook entry point for the installer binary: default layout for `user`
/// with the live Helix config in `primary_runtime_dir`.
pub fn install_chat_mcp(user: &str, primary_runtime_dir: &str) -> Result<InstallReport, String> {
    let layout = layout_for_user(user, primary_runtime_dir);
    install_chat_mcp_at(&layout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// Serializes the tests that invoke `cargo build` so concurrent fixture
    /// builds never stack up (the box rule is 2 concurrent cargo builds max;
    /// the bot suite itself is the other one when run together).
    static BUILD_LOCK: Mutex<()> = Mutex::new(());
    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn lock_build() -> std::sync::MutexGuard<'static, ()> {
        BUILD_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Unique scratch dir per test under the package target dir (never /tmp:
    /// fixture cargo builds write their own target dirs here).
    fn unique_dir(tag: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/test-tmp-chat-mcp");
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

    /// Minimal cargo project whose release binary is named `chat-mcp`.
    fn fixture_crate(dir: &Path) {
        write(
            &dir.join("Cargo.toml"),
            "[package]\nname = \"chat-mcp\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[[bin]]\nname = \"chat-mcp\"\npath = \"src/main.rs\"\n",
        );
        write(
            &dir.join("src/main.rs"),
            "fn main() { println!(\"fixture chat-mcp\"); }\n",
        );
    }

    fn layout_in(root: &Path, source: &Path) -> ChatMcpLayout {
        ChatMcpLayout {
            bin_path: root.join("home/.local/bin/chat-mcp"),
            config_dir: root.join("home/.config/chat-mcp"),
            config_path: root.join("home/.config/chat-mcp/config.toml"),
            helix_config_path: root.join("helix/config.toml"),
            source_dir: source.to_path_buf(),
        }
    }

    fn count_occurrences(haystack: &str, needle: &str) -> usize {
        haystack.matches(needle).count()
    }

    const HELIX_WITH_GOOGLE: &str = "[helix]\nname = \"Helix\"\n\n[mcp]\nenabled = true\n\n[[mcp.servers]]\nname = \"google\"\ncommand = \"/home/cobble/.local/bin/google-mcp\"\n";

    #[test]
    fn block_appended_when_missing() {
        let bin = Path::new("/home/cobble/.local/bin/chat-mcp");
        let out = append_chat_server(HELIX_WITH_GOOGLE, bin).expect("must append");
        assert_eq!(count_occurrences(&out, "[mcp]"), 1, "header not duplicated");
        assert_eq!(count_occurrences(&out, "name = \"chat\""), 1);
        assert!(out.contains("command = \"/home/cobble/.local/bin/chat-mcp\""));
        assert!(has_chat_server(&out));
    }

    #[test]
    fn block_noop_when_present() {
        let bin = Path::new("/home/cobble/.local/bin/chat-mcp");
        let with_chat = format!("{HELIX_WITH_GOOGLE}\n{}", chat_server_block(bin));
        assert!(append_chat_server(&with_chat, bin).is_none());
    }

    #[test]
    fn block_ignores_commented_chat() {
        let bin = Path::new("/home/cobble/.local/bin/chat-mcp");
        let commented = format!("{HELIX_WITH_GOOGLE}\n# [[mcp.servers]]\n# name = \"chat\"\n# command = \"/nowhere/chat-mcp\"\n");
        assert!(!has_chat_server(&commented));
        let out = append_chat_server(&commented, bin).expect("must append");
        // Count live blocks, not substrings: the fixture's commented
        // `# name = "chat"` line still contains the substring.
        assert_eq!(count_occurrences(&out, "[[mcp.servers]]\nname = \"chat\""), 1);
    }

    #[test]
    fn block_adds_mcp_header_when_missing() {
        let bin = Path::new("/home/cobble/.local/bin/chat-mcp");
        let bare = "[helix]\nname = \"Helix\"\n";
        let out = append_chat_server(bare, bin).expect("must append");
        assert!(has_mcp_header(&out));
        assert!(has_chat_server(&out));
    }

    #[test]
    fn block_handles_missing_trailing_newline() {
        let bin = Path::new("/home/cobble/.local/bin/chat-mcp");
        let no_newline = HELIX_WITH_GOOGLE.trim_end();
        let out = append_chat_server(no_newline, bin).expect("must append");
        assert!(has_chat_server(&out));
        assert!(!out.contains("google-mcp\"[[mcp.servers]]"), "blocks must not run together");
    }

    #[test]
    fn has_chat_server_accepts_single_quotes() {
        let text = "[mcp]\nenabled = true\n\n[[mcp.servers]]\nname = 'chat'\ncommand = \"/bin/chat-mcp\"\n";
        assert!(has_chat_server(text));
    }

    #[test]
    fn config_created_with_perms_and_no_token() {
        let dir = unique_dir("config-fresh");
        let layout = layout_in(&dir, &dir.join("src"));
        let outcome = ensure_chat_mcp_config(&layout.config_dir, &layout.config_path)
            .expect("create config");
        assert_eq!(outcome, ConfigOutcome::Created);
        let text = fs::read_to_string(&layout.config_path).expect("read config");
        assert!(text.contains("hub_url = \"https://chat.micro-tech.xyz\""));
        assert!(text.contains("watch_timeout_secs = 30"));
        assert!(
            !text.lines().any(|l| l.trim_start().starts_with("token =")),
            "installer must never write a token key"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&layout.config_path).expect("stat").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "config must be owner-only");
        }
        cleanup(&dir);
    }

    #[test]
    fn config_preserved_untouched() {
        let dir = unique_dir("config-preserved");
        let layout = layout_in(&dir, &dir.join("src"));
        let original = "hub_url = \"https://chat.micro-tech.xyz\"\nwatch_timeout_secs = 30\ntoken = \"fixture-only-not-a-real-token\"\n";
        write(&layout.config_path, original);
        let outcome = ensure_chat_mcp_config(&layout.config_dir, &layout.config_path)
            .expect("preserve config");
        assert_eq!(outcome, ConfigOutcome::Preserved);
        assert_eq!(fs::read_to_string(&layout.config_path).expect("read"), original);
        cleanup(&dir);
    }

    #[test]
    fn binary_build_and_install_fixture() {
        let _guard = lock_build();
        let dir = unique_dir("bin-fixture");
        let src = dir.join("src");
        fixture_crate(&src);
        let dest = dir.join("bin/chat-mcp");
        build_and_install_binary(&src, &dest).expect("build + install fixture");
        assert!(dest.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&dest).expect("stat").permissions().mode() & 0o777;
            assert_ne!(mode & 0o111, 0, "installed binary must be executable");
        }
        cleanup(&dir);
    }

    #[test]
    fn binary_missing_manifest_errors() {
        let dir = unique_dir("bin-nomanifest");
        let err = build_and_install_binary(&dir.join("empty"), &dir.join("bin/chat-mcp"))
            .expect_err("must refuse a dir without Cargo.toml");
        assert!(err.contains("no Cargo.toml"));
        cleanup(&dir);
    }

    #[test]
    fn orchestrator_fresh_install() {
        let _guard = lock_build();
        let dir = unique_dir("orch-fresh");
        let src = dir.join("src");
        fixture_crate(&src);
        let layout = layout_in(&dir, &src);
        write(&layout.helix_config_path, HELIX_WITH_GOOGLE);
        let report = install_chat_mcp_at(&layout).expect("fresh install");
        assert!(report.binary_installed);
        assert_eq!(report.config, ConfigOutcome::Created);
        assert!(report.block_appended);
        assert!(layout.bin_path.exists());
        assert!(layout.config_path.exists());
        let helix = fs::read_to_string(&layout.helix_config_path).expect("read helix config");
        assert_eq!(count_occurrences(&helix, "name = \"chat\""), 1);
        cleanup(&dir);
    }

    #[test]
    fn orchestrator_idempotent_rerun() {
        let _guard = lock_build();
        let dir = unique_dir("orch-rerun");
        let src = dir.join("src");
        fixture_crate(&src);
        let layout = layout_in(&dir, &src);
        write(&layout.helix_config_path, HELIX_WITH_GOOGLE);
        let first = install_chat_mcp_at(&layout).expect("first run");
        assert!(first.block_appended);
        let baks_after_first = count_baks(&dir);
        let second = install_chat_mcp_at(&layout).expect("second run");
        assert!(second.binary_installed);
        assert_eq!(second.config, ConfigOutcome::Preserved);
        assert!(!second.block_appended, "re-run must not duplicate the block");
        assert_eq!(count_baks(&dir), baks_after_first, "no-op run backs up nothing");
        let helix = fs::read_to_string(&layout.helix_config_path).expect("read helix config");
        assert_eq!(count_occurrences(&helix, "name = \"chat\""), 1);
        cleanup(&dir);
    }

    fn count_baks(root: &Path) -> usize {
        let mut count = 0;
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            if let Ok(entries) = fs::read_dir(&d) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if p.to_string_lossy().contains(".bak-") {
                        count += 1;
                    }
                }
            }
        }
        count
    }

    #[test]
    fn orchestrator_existing_config_preserved() {
        let _guard = lock_build();
        let dir = unique_dir("orch-preserved");
        let src = dir.join("src");
        fixture_crate(&src);
        let layout = layout_in(&dir, &src);
        let original = "hub_url = \"https://chat.micro-tech.xyz\"\nwatch_timeout_secs = 30\ntoken = \"fixture-only-not-a-real-token\"\n";
        write(&layout.config_path, original);
        write(&layout.helix_config_path, HELIX_WITH_GOOGLE);
        let report = install_chat_mcp_at(&layout).expect("install with existing config");
        assert!(report.binary_installed);
        assert_eq!(report.config, ConfigOutcome::Preserved);
        assert!(report.block_appended);
        assert_eq!(fs::read_to_string(&layout.config_path).expect("read"), original);
        cleanup(&dir);
    }

    #[test]
    fn orchestrator_missing_helix_config_errors_after_partial_progress() {
        let _guard = lock_build();
        let dir = unique_dir("orch-nohelix");
        let src = dir.join("src");
        fixture_crate(&src);
        let layout = layout_in(&dir, &src);
        let err = install_chat_mcp_at(&layout).expect_err("missing helix config must error");
        assert!(err.contains("Helix config"));
        assert!(layout.bin_path.exists(), "binary stays installed on later failure");
        assert!(layout.config_path.exists(), "chat config stays created on later failure");
        cleanup(&dir);
    }
}
