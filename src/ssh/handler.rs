//! russh [`Handler`] implementation for the Helix SSH server.
//!
//! THREAT MODEL (see `src/config/ssh.rs` and `docs/ssh.md`):
//! - Public-key auth ONLY. `auth_none` / `auth_password` / keyboard-interactive
//!   are never accepted — the defaults reject and we don't override them.
//! - Keys are checked against the dedicated Helix-managed `authorized_keys`
//!   set, never the service user's `~/.ssh/authorized_keys`.
//! - Every auth attempt is logged with user, peer IP, and key fingerprint
//!   (SHA256) — never key material.
//! - Per-IP failed-auth rate limiting (sliding 1-minute window) sits in front
//!   of the key check; russh's own `max_auth_attempts` + `auth_rejection_time`
//!   apply per connection underneath.
//! - Sessions execute commands through
//!   [`run_shell`](crate::tools::shell_tool::run_shell) — the task-191
//!   policy (denylist, timeouts, output caps, workdir confinement). There is
//!   exactly ONE shell execution path.
//! - v1 is exec-channel only: PTY/`shell_request` is rejected with a clear
//!   message (PTY is a follow-up, not a silent second shell path).

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use log::{info, warn};
use russh::keys::ssh_key::{public::KeyData, HashAlg, PublicKey};
use russh::server::{Auth, ChannelOpenHandle, Handler, Msg, Session};
use russh::{Channel, ChannelId, ChannelOpenFailure};

/// Cross-connection shared state for the SSH server.
pub struct SharedState {
    /// Key data from the Helix-managed authorized_keys file.
    /// Compared by key material only — OpenSSH comments are ignored, so a
    /// key stays authorized regardless of the comment on its line.
    pub authorized_keys: HashSet<KeyData>,
    /// Currently open session channels.
    pub open_sessions: AtomicUsize,
    /// Cap on concurrent session channels.
    pub max_sessions: usize,
    /// Failed auth attempts per IP (sliding window).
    pub auth_attempts: Mutex<HashMap<IpAddr, Vec<Instant>>>,
    /// Max failed attempts per IP per minute before throttling.
    pub rate_limit_per_minute: u32,
}

impl SharedState {
    pub fn new(
        authorized_keys: HashSet<KeyData>,
        max_sessions: usize,
        rate_limit_per_minute: u32,
    ) -> Self {
        Self {
            authorized_keys,
            open_sessions: AtomicUsize::new(0),
            max_sessions,
            auth_attempts: Mutex::new(HashMap::new()),
            rate_limit_per_minute,
        }
    }

    /// Record a failed auth attempt; true when the IP is now throttled.
    fn note_failed_auth(&self, ip: IpAddr) -> bool {
        let mut guard = self.auth_attempts.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let window = std::time::Duration::from_secs(60);
        let entry = guard.entry(ip).or_default();
        entry.retain(|t| now.duration_since(*t) < window);
        entry.push(now);
        entry.len() > self.rate_limit_per_minute as usize
    }
}

/// One SSH connection's handler.
pub struct HelixSshHandler {
    peer_ip: IpAddr,
    shared: Arc<SharedState>,
    authed_user: Option<String>,
}

impl HelixSshHandler {
    pub fn new(peer_ip: IpAddr, shared: Arc<SharedState>) -> Self {
        Self {
            peer_ip,
            shared,
            authed_user: None,
        }
    }

    fn fingerprint(key: &PublicKey) -> String {
        key.fingerprint(HashAlg::Sha256).to_string()
    }
}

#[derive(Debug)]
pub struct HandlerError(pub String);

impl From<russh::Error> for HandlerError {
    fn from(e: russh::Error) -> Self {
        HandlerError(e.to_string())
    }
}

impl std::fmt::Display for HandlerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ssh handler error: {}", self.0)
    }
}

impl std::error::Error for HandlerError {}

impl Handler for HelixSshHandler {
    type Error = HandlerError;

    /// Probe: is this key even in the authorized set? No counting, no logging
    /// beyond debug — ownership isn't proven yet.
    async fn auth_publickey_offered(
        &mut self,
        _user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        if self.shared.authorized_keys.contains(public_key.key_data()) {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::Reject {
                proceed_with_methods: None,
                partial_success: false,
            })
        }
    }

    /// Real public-key auth: rate-limit, then key check, logging everything
    /// except key material.
    async fn auth_publickey(
        &mut self,
        user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        let fp = Self::fingerprint(public_key);
        if self.shared.note_failed_auth(self.peer_ip) {
            warn!(
                "[ssh] auth throttled: peer={} user='{}' fp={} (rate limit)",
                self.peer_ip, user, fp
            );
            return Ok(Auth::Reject {
                proceed_with_methods: None,
                partial_success: false,
            });
        }
        if self.shared.authorized_keys.contains(public_key.key_data()) {
            info!(
                "[ssh] auth accepted: peer={} user='{}' fp={}",
                self.peer_ip, user, fp
            );
            self.authed_user = Some(user.to_string());
            Ok(Auth::Accept)
        } else {
            warn!(
                "[ssh] auth rejected: peer={} user='{}' fp={} (unknown key)",
                self.peer_ip, user, fp
            );
            Ok(Auth::Reject {
                proceed_with_methods: None,
                partial_success: false,
            })
        }
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let open = self.shared.open_sessions.fetch_add(1, Ordering::SeqCst) + 1;
        if open > self.shared.max_sessions {
            self.shared.open_sessions.fetch_sub(1, Ordering::SeqCst);
            warn!(
                "[ssh] session rejected: peer={} at cap ({}/{})",
                self.peer_ip, open, self.shared.max_sessions
            );
            reply.reject(ChannelOpenFailure::ResourceShortage).await;
            return Ok(());
        }
        info!(
            "[ssh] channel open: peer={} user={:?} ({}/{})",
            self.peer_ip,
            self.authed_user,
            open,
            self.shared.max_sessions
        );
        reply.accept().await;
        Ok(())
    }

    async fn channel_close(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.shared.open_sessions.fetch_sub(1, Ordering::SeqCst);
        Ok(())
    }

    /// Exec channel: run the command through the ONE shell execution path —
    /// `run_shell` (task 191 policy). Blocking work goes on a blocking
    /// thread so the russh runtime never stalls.
    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data).into_owned();
        let user = self.authed_user.clone().unwrap_or_else(|| "?".to_string());
        info!(
            "[ssh] exec: peer={} user='{}' command_len={}",
            self.peer_ip,
            user,
            command.len()
        );

        // ONE shell path: the task-191 policy (gate, denylist, timeout with
        // process-group kill, output caps, no TTY, workdir confinement).
        let report = tokio::task::spawn_blocking(move || {
            crate::tools::shell_tool::run_shell(&serde_json::json!({
                "command": command,
            }))
        })
        .await
        .map_err(|e| HandlerError(format!("shell task panicked: {}", e)))?;

        let code = exit_code_for_report(&report);
        info!(
            "[ssh] exec done: peer={} user='{}' exit={}",
            self.peer_ip, user, code
        );
        session.data(channel, report.into_bytes())?;
        let _ = session.exit_status_request(channel, code);
        session.eof(channel)?;
        session.close(channel)?;
        Ok(())
    }

    /// v1: no interactive shells. Accepting a PTY shell would be a second
    /// shell path outside the run_shell policy — reject with a message.
    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        warn!(
            "[ssh] shell_request rejected (exec-only v1): peer={}",
            self.peer_ip
        );
        session.channel_failure(channel)?;
        session.data(
            channel,
            "Helix SSH v1 supports exec channels only (e.g. `ssh -p 2222 helix@127.0.0.1 'command'`). Interactive shells are a follow-up.\r\n"
                .as_bytes()
                .to_vec(),
        )?;
        session.close(channel)?;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        _col_width: u32,
        _row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // PTY is a follow-up; reject so clients fail fast with a reason.
        session.channel_failure(channel)?;
        Ok(())
    }

    /// Stdin data on an exec channel: the command already ran via run_shell;
    /// there is no persistent shell to feed. Ignore.
    async fn data(
        &mut self,
        _channel: ChannelId,
        _data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Map a run_shell report to an SSH exit status.
/// 124 = timeout (like timeout(1)); 126 = policy/refusal; else the real code.
fn exit_code_for_report(report: &str) -> u32 {
    if report.contains("[TIMEOUT") {
        return 124;
    }
    if report.contains("blocked by security policy") || report.contains("is disabled") {
        return 126;
    }
    // Reports from spawn_and_wait carry "[exit: code N]".
    if let Some(idx) = report.find("[exit: code ") {
        let rest = &report[idx + "[exit: code ".len()..];
        if let Some(end) = rest.find(']') {
            if let Ok(n) = rest[..end].trim().parse::<i32>() {
                return n.max(0) as u32;
            }
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::ssh_key::PublicKey;

    fn shared_state() -> Arc<SharedState> {
        Arc::new(SharedState::new(HashSet::new(), 8, 10))
    }

    #[test]
    fn exit_code_mapping() {
        assert_eq!(exit_code_for_report("$ x\n[exit: code 0]\n"), 0);
        assert_eq!(exit_code_for_report("$ x\n[exit: code 3]\n"), 3);
        assert_eq!(
            exit_code_for_report("bla [TIMEOUT after 2s — process group killed]"),
            124
        );
        assert_eq!(
            exit_code_for_report("run_shell: command blocked by security policy: nope"),
            126
        );
        assert_eq!(exit_code_for_report("run_shell is disabled: ..."), 126);
        assert_eq!(exit_code_for_report("weird"), 1);
    }

    #[test]
    fn rate_limiter_throttles() {
        let shared = shared_state();
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        // 10 allowed, 11th throttles.
        for _ in 0..10 {
            assert!(!shared.note_failed_auth(ip));
        }
        assert!(shared.note_failed_auth(ip));
    }

    #[test]
    fn authorized_set_matches_parsed_key() {
        // Generate a real key via the server's own host-key function, then
        // check the authorized-set round trip on its public half.
        let dir = std::env::temp_dir().join(format!("helix-ssh-test4-{}", std::process::id()));
        let path = dir.join("ssh_host_key");
        let privkey = crate::ssh::server::load_or_generate_host_key(&path).unwrap();
        let line = privkey
            .public_key()
            .to_openssh()
            .expect("public key encodes");
        let key = PublicKey::from_openssh(line.trim()).expect("test key parses");
        let mut set = HashSet::new();
        set.insert(key.key_data().clone());
        assert!(set.contains(key.key_data()));
        // Fingerprint is a stable non-empty SHA256 string.
        let fp = key.fingerprint(HashAlg::Sha256).to_string();
        assert!(fp.starts_with("SHA256:"), "got: {}", fp);
        assert_eq!(fp, key.fingerprint(HashAlg::Sha256).to_string());
        std::fs::remove_dir_all(&dir).ok();
    }
}
