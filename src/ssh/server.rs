//! SSH listener for Helix (russh 0.63, aws-lc-rs).
//!
//! Startup policy (fail closed):
//! 1. `[ssh] enabled = false` (default) → the listener never starts.
//! 2. `[shell]` disabled → refuse to start: SSH sessions execute through
//!    the `run_shell` policy, and running SSH without that policy would be
//!    a second, unguarded shell path.
//! 3. No usable authorized keys → refuse to start (never accept connections
//!    in a degraded state).
//!
//! Bind policy: 127.0.0.1 by default; `0.0.0.0` requires an explicit config
//! choice and logs a loud warning. The bind address comes ONLY from the
//! config file — there is no tool/chat surface that can change it
//! (prompt-injection guard).

use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use log::{error, info, warn};
use russh::keys::PrivateKey;
use russh::keys::ssh_key::{public::KeyData, Algorithm, LineEnding, PublicKey};
use russh::server::Config as RusshConfig;

/// Infallible OS RNG for host-key generation, backed by `getrandom`.
/// Panics only if the OS RNG itself fails — at which point startup is
/// doomed anyway. (`rand_core::OsRng` only offers the fallible API;
/// `PrivateKey::random` needs the infallible `CryptoRng`.)
#[derive(Debug, Default)]
struct OsRng;

impl rand_core::TryRng for OsRng {
    type Error = core::convert::Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        let mut b = [0u8; 4];
        getrandom::fill(&mut b).expect("OS RNG failure");
        Ok(u32::from_ne_bytes(b))
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        let mut b = [0u8; 8];
        getrandom::fill(&mut b).expect("OS RNG failure");
        Ok(u64::from_ne_bytes(b))
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        getrandom::fill(dst).expect("OS RNG failure");
        Ok(())
    }
}

impl rand_core::TryCryptoRng for OsRng {}

use super::handler::{HelixSshHandler, SharedState};
use crate::config::ssh::SshConfig;

/// Directory (under the config dir) holding Helix-managed SSH material.
const SSH_DIR_NAME: &str = "ssh";
const AUTHORIZED_KEYS_NAME: &str = "authorized_keys";
const HOST_KEY_NAME: &str = "ssh_host_key";

/// Resolve `<config-dir>/ssh/<name>`, or the explicit config path.
/// Relative explicit paths resolve under `<config-dir>/ssh/` (keeps all
/// SSH material in one Helix-managed directory).
fn resolve_ssh_path(config_dir: &Path, explicit: Option<&str>, name: &str) -> PathBuf {
    let ssh_dir = config_dir.join(SSH_DIR_NAME);
    match explicit {
        Some(p) => {
            let pb = PathBuf::from(p);
            if pb.is_absolute() {
                pb
            } else {
                ssh_dir.join(pb)
            }
        }
        None => ssh_dir.join(name),
    }
}

/// Load the authorized-keys set. Fail closed: missing file, empty file, or
/// zero parseable keys is a hard error — never start degraded.
pub fn load_authorized_keys(path: &Path) -> Result<HashSet<KeyData>, String> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "SSH fail-closed: cannot read authorized_keys '{}': {}. \
             Create it with your public key(s), one per line.",
            path.display(),
            e
        )
    })?;
    let mut keys = HashSet::new();
    for (i, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match PublicKey::from_openssh(line) {
            Ok(k) => {
                // Key material only: comments must not affect auth.
                keys.insert(k.key_data().clone());
            }
            Err(e) => {
                warn!(
                    "[ssh] ignoring unparseable authorized_keys line {}: {}",
                    i + 1,
                    e
                );
            }
        }
    }
    if keys.is_empty() {
        return Err(format!(
            "SSH fail-closed: no usable keys in authorized_keys '{}'. \
             Refusing to start without authorized keys.",
            path.display()
        ));
    }
    Ok(keys)
}

/// Load the server host key, generating + persisting (0600) an Ed25519 key
/// on first start.
pub fn load_or_generate_host_key(path: &Path) -> Result<PrivateKey, String> {
    if let Ok(content) = std::fs::read_to_string(path) {
        return PrivateKey::from_openssh(content.trim()).map_err(|e| {
            format!(
                "cannot parse SSH host key '{}': {}",
                path.display(),
                e
            )
        });
    }
    // Generate.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create ssh dir '{}': {}", parent.display(), e))?;
    }
    // Generate with the OS RNG (panics only if the OS RNG itself fails —
    // at which point there are bigger problems than an SSH host key).
    let mut rng = OsRng;
    let key = PrivateKey::random(&mut rng, Algorithm::Ed25519)
        .map_err(|e| format!("cannot generate SSH host key: {}", e))?;
    let pem = key
        .to_openssh(LineEnding::LF)
        .map_err(|e| format!("cannot encode SSH host key: {}", e))?;
    std::fs::write(path, pem.as_str())
        .map_err(|e| format!("cannot write SSH host key '{}': {}", path.display(), e))?;
    // 0600 from birth: the service user's host key must not be world-readable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    info!("[ssh] generated new Ed25519 host key at '{}'", path.display());
    Ok(key)
}

/// Parse + validate the bind address. Returns the IP; logs the loud warning
/// for 0.0.0.0. Invalid addresses are a hard error (no silent fallback to
/// loopback — the operator must fix the config).
fn parse_bind(bind: &str) -> Result<IpAddr, String> {
    let ip: IpAddr = bind
        .parse()
        .map_err(|_| format!("invalid [ssh] bind address '{}'", bind))?;
    if ip.is_unspecified() {
        warn!(
            "[ssh] BINDING TO 0.0.0.0 — the Helix SSH server is reachable from \
             the whole network. This was an explicit config choice \
             ([ssh] bind = \"0.0.0.0\"). Ensure [ssh] authorized_keys is \
             correct and consider the Tailscale interface IP instead."
        );
    }
    Ok(ip)
}

/// Start the SSH server. Returns after the listener is up (runs forever);
/// `Err` only for fail-closed startup refusals.
pub async fn start_ssh_server(ssh_cfg: SshConfig, config_dir: PathBuf) -> Result<(), String> {
    if !ssh_cfg.is_enabled() {
        info!("[ssh] disabled ([ssh] enabled = false) — listener not started");
        return Ok(());
    }

    // Fail closed: no run_shell policy, no SSH. A second shell path without
    // the denylist/timeout/caps would silently widen the threat model.
    if !crate::tools::shell_tool::is_enabled() {
        error!(
            "[ssh] REFUSING TO START: [shell] is not enabled. SSH sessions \
             execute through the run_shell policy (task 191); enable it with \
             `[shell] enabled = true` or disable [ssh]."
        );
        return Err("SSH fail-closed: [shell] is not enabled".to_string());
    }

    let ak_path = resolve_ssh_path(&config_dir, ssh_cfg.authorized_keys.as_deref(), AUTHORIZED_KEYS_NAME);
    let keys = load_authorized_keys(&ak_path)?;
    info!(
        "[ssh] loaded {} authorized key(s) from '{}'",
        keys.len(),
        ak_path.display()
    );

    let hk_path = resolve_ssh_path(&config_dir, ssh_cfg.host_key.as_deref(), HOST_KEY_NAME);
    let host_key = load_or_generate_host_key(&hk_path)?;

    let bind_ip = parse_bind(&ssh_cfg.bind)?;
    let addr = SocketAddr::new(bind_ip, ssh_cfg.port);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("cannot bind SSH to {}: {}", addr, e))?;
    info!("[ssh] listening on {} (key-only auth)", addr);

    let mut russh_config = RusshConfig::default();
    russh_config.keys.push(host_key);
    russh_config.max_auth_attempts = 3;
    russh_config.auth_rejection_time = Duration::from_secs(1);
    russh_config.inactivity_timeout = Some(Duration::from_secs(ssh_cfg.idle_timeout_secs));
    let russh_config = Arc::new(russh_config);

    let shared = Arc::new(SharedState::new(
        keys,
        ssh_cfg.max_sessions,
        ssh_cfg.auth_rate_limit_per_minute,
    ));

    loop {
        let (stream, peer) = listener
            .accept()
            .await
            .map_err(|e| format!("SSH accept failed: {}", e))?;
        let peer_ip = peer.ip();
        info!("[ssh] incoming connection from {}", peer);
        let cfg = russh_config.clone();
        let shared = shared.clone();
        tokio::spawn(async move {
            let handler = HelixSshHandler::new(peer_ip, shared);
            if let Err(e) = russh::server::run_stream(cfg, stream, handler).await {
                warn!("[ssh] connection from {} ended: {}", peer, e);
            } else {
                info!("[ssh] connection from {} closed", peer);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bind_loopback_ok() {
        let ip = parse_bind("127.0.0.1").unwrap();
        assert!(ip.is_loopback());
    }

    #[test]
    fn parse_bind_invalid_is_hard_error() {
        assert!(parse_bind("not-an-ip").is_err());
        // No silent fallback: an invalid bind never becomes loopback.
    }

    #[test]
    fn parse_bind_unspecified_parses() {
        // 0.0.0.0 is accepted (with the loud warning) — explicit opt-in.
        let ip = parse_bind("0.0.0.0").unwrap();
        assert!(ip.is_unspecified());
    }

    #[test]
    fn load_authorized_keys_fail_closed_on_missing() {
        let err =
            load_authorized_keys(Path::new("/nonexistent/authorized_keys")).unwrap_err();
        assert!(err.contains("fail-closed"), "got: {}", err);
    }

    #[test]
    fn load_authorized_keys_fail_closed_on_empty() {
        let dir = std::env::temp_dir().join(format!("helix-ssh-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("authorized_keys");
        std::fs::write(&path, "# only a comment\n\n").unwrap();
        let err = load_authorized_keys(&path).unwrap_err();
        assert!(err.contains("fail-closed"), "got: {}", err);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_authorized_keys_parses_valid_skips_garbage() {
        let dir = std::env::temp_dir().join(format!("helix-ssh-test2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Real key via the host-key generator (public half only).
        let hk_path = dir.join("ssh_host_key");
        let privkey = load_or_generate_host_key(&hk_path).unwrap();
        let publine = privkey.public_key().to_openssh().unwrap();
        let path = dir.join("authorized_keys");
        std::fs::write(&path, format!("{}\ngarbage-line\n", publine.trim())).unwrap();
        let keys = load_authorized_keys(&path).unwrap();
        assert_eq!(keys.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn host_key_generate_and_reload() {
        let dir = std::env::temp_dir().join(format!("helix-ssh-test3-{}", std::process::id()));
        let path = dir.join("ssh_host_key");
        let k1 = load_or_generate_host_key(&path).unwrap();
        assert!(path.exists());
        let k2 = load_or_generate_host_key(&path).unwrap();
        assert_eq!(
            k1.public_key()
                .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
                .to_string(),
            k2.public_key()
                .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
                .to_string(),
            "reloaded key must match generated key"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_ssh_path_defaults_under_config_dir() {
        let config_dir = Path::new("/etc/helix");
        assert_eq!(
            resolve_ssh_path(config_dir, None, "authorized_keys"),
            PathBuf::from("/etc/helix/ssh/authorized_keys")
        );
        assert_eq!(
            resolve_ssh_path(config_dir, Some("custom_keys"), "authorized_keys"),
            PathBuf::from("/etc/helix/ssh/custom_keys")
        );
        assert_eq!(
            resolve_ssh_path(config_dir, Some("/abs/keys"), "authorized_keys"),
            PathBuf::from("/abs/keys")
        );
    }
}
