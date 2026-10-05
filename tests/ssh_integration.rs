//! Live SSH interop test: real OpenSSH client against the Helix russh server.
//!
//! Spins up `start_ssh_server` on 127.0.0.1:2229 with a temp authorized_keys,
//! then drives it with the system `ssh` binary:
//! 1. Authorized key → exec works, output + exit code come back.
//! 2. Denylisted command (`rm -rf /`) → blocked by the task-191 run_shell
//!    policy (proves ONE execution path, not two).
//! 3. Wrong key → authentication rejected.
//! 4. No authorized keys → `start_ssh_server` refuses (fail closed).

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use helix::config::ssh::SshConfig;

/// Generate an ed25519 keypair with ssh-keygen; returns (priv_path, pub_line).
fn gen_keypair(dir: &std::path::Path, name: &str) -> (PathBuf, String) {
    let priv_path = dir.join(name);
    let out = std::process::Command::new("ssh-keygen")
        .args([
            "-t",
            "ed25519",
            "-f",
            priv_path.to_str().unwrap(),
            "-N",
            "",
            "-q",
        ])
        .stdin(Stdio::null())
        .output()
        .expect("ssh-keygen must exist for this test");
    assert!(out.status.success(), "ssh-keygen failed");
    let pub_line =
        std::fs::read_to_string(dir.join(format!("{}.pub", name))).expect("pubkey written");
    (priv_path, pub_line)
}

fn ssh_config(dir: &std::path::Path, ak_rel: &str) -> (SshConfig, PathBuf) {
    let cfg = SshConfig {
        enabled: true,
        bind: "127.0.0.1".to_string(),
        port: 2229,
        authorized_keys: Some(ak_rel.to_string()),
        host_key: Some("ssh_host_key".to_string()),
        max_sessions: 4,
        idle_timeout_secs: 60,
        auth_rate_limit_per_minute: 100,
    };
    (cfg, dir.to_path_buf())
}

fn ssh_args<'a>(key: &'a PathBuf, port: &'a str, remote_cmd: Option<&'a str>) -> Vec<&'a str> {
    let mut args = vec![
        "-i",
        key.to_str().unwrap(),
        "-p",
        port,
        "-o",
        "StrictHostKeyChecking=no",
        "-o",
        "UserKnownHostsFile=/dev/null",
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=10",
        "helix@127.0.0.1",
    ];
    if let Some(cmd) = remote_cmd {
        args.push(cmd);
    }
    args
}

async fn wait_for_port(port: u16) {
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("SSH test server never came up on {}", port);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ssh_exec_authorized_key_works_and_policy_blocks() {
    // NOTE: one test function, sequential steps — the shell config OnceLock
    // is process-wide, so parallel tests would race its initialization.
    let dir = std::env::temp_dir().join(format!("helix-ssh-live-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // [shell] must be enabled or the SSH server fails closed. Point the
    // shell tool at a temp config first (fresh process => fresh OnceLock).
    let shell_cfg = dir.join("config.toml");
    std::fs::write(
        &shell_cfg,
        "[shell]\nenabled = true\ndefault_timeout_secs = 20\n",
    )
    .unwrap();
    helix::tools::shell_tool::set_config_path(shell_cfg);
    assert!(helix::tools::shell_tool::is_enabled());

    let (key_path, pub_line) = gen_keypair(&dir, "id_test");
    // Relative authorized_keys paths resolve under <config-dir>/ssh/.
    std::fs::create_dir_all(dir.join("ssh")).unwrap();
    std::fs::write(dir.join("ssh").join("test_authorized_keys"), &pub_line).unwrap();

    let (cfg, config_dir) = ssh_config(&dir, "test_authorized_keys");
    tokio::spawn(async move {
        let _ = helix::ssh::start_ssh_server(cfg, config_dir).await;
    });
    wait_for_port(2229).await;

    // 1. Authorized key: exec works.
    let out = std::process::Command::new("ssh")
        .args(ssh_args(&key_path, "2229", Some("echo hello-ssh-live")))
        .stdin(Stdio::null())
        .output()
        .expect("ssh run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "ssh exec failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("hello-ssh-live"),
        "unexpected stdout: {}",
        stdout
    );
    assert!(stdout.contains("[exit: code 0]"), "stdout: {}", stdout);

    // 2. Denylisted command is blocked by the run_shell policy (one path).
    let out = std::process::Command::new("ssh")
        .args(ssh_args(&key_path, "2229", Some("rm -rf /")))
        .stdin(Stdio::null())
        .output()
        .expect("ssh run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("blocked by security policy"),
        "denylist did not fire over SSH: {}",
        stdout
    );
    assert!(!out.status.success(), "blocked command must not exit 0");

    // 3. Wrong key is rejected.
    let (wrong_key, _) = gen_keypair(&dir, "id_wrong");
    let out = std::process::Command::new("ssh")
        .args(ssh_args(&wrong_key, "2229", Some("echo nope")))
        .stdin(Stdio::null())
        .output()
        .expect("ssh run");
    assert!(
        !out.status.success(),
        "wrong key must not authenticate"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Permission denied") || stderr.contains("denied"),
        "expected auth rejection, stderr: {}",
        stderr
    );

    // 4. Fail closed without keys: refuses to start AND does not bind.
    // (Shell is enabled here; the refusal comes from the keys check.)
    let cfg = SshConfig {
        enabled: true,
        bind: "127.0.0.1".to_string(),
        port: 2230,
        authorized_keys: Some("does-not-exist".to_string()),
        host_key: Some("ssh_host_key".to_string()),
        ..SshConfig::default()
    };
    let err = helix::ssh::start_ssh_server(cfg, dir.clone())
        .await
        .unwrap_err();
    assert!(err.contains("fail-closed"), "got: {}", err);
    assert!(
        tokio::net::TcpStream::connect(("127.0.0.1", 2230))
            .await
            .is_err(),
        "fail-closed server must not bind"
    );

    std::fs::remove_dir_all(&dir).ok();
}
