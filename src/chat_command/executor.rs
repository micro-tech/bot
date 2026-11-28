//! Read-only command execution (tasks 298/304).
//!
//! No shell, ever, on this path. The command line is split on
//! whitespace, every token is screened for metacharacters, argv[0]
//! must be allowlisted, and the process is spawned with
//! `std::process::Command` directly. `run_shell` is deliberately NOT
//! used here — it runs via a shell and allows writes, so it cannot
//! enforce read-only.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Characters that must never appear in any token. Covers pipes,
/// redirection, command substitution, chaining, globbing, tilde
/// expansion, quoting, and subshell/grouping syntax. A token with one
/// of these is rejected whole — there is no escaping or quoting in v1.
pub const METACHARS: &[char] = &[
    '|', '>', '<', ';', '&', '$', '`', '"', '\'', '*', '?', '~', '(', ')', '{', '}',
];

/// Hardcoded LOCAL allowlist (code, NOT config — a config edit must
/// never silently widen remote execution; allowlist changes go through
/// code review).
pub const LOCAL_ALLOWLIST: &[&str] = &[
    "cat",
    "tail",
    "head",
    "grep",
    "ls",
    "ps",
    "df",
    "free",
    "uptime",
    "whoami",
    "hostname",
    "systemctl",
];

/// `systemctl` subcommands that only read state. Everything else
/// (`restart`, `stop`, `enable`, ...) is rejected.
pub const SYSTEMCTL_READ_ONLY: &[&str] = &["status", "is-active", "show", "list-units"];

/// Commands that take file paths to read. Their path arguments are
/// confined to [`ALLOWED_READ_ROOTS`] (M5, task 395) — see
/// [`is_allowed_read_path`].
pub const READ_COMMANDS: &[&str] = &["cat", "head", "tail", "grep"];

/// Flags of the read commands whose following argument is a VALUE, not
/// a path (`head -n 5`, `tail -c 100`, `grep -m 3`). The value must be
/// skipped by the path check, not rejected as an unlisted path.
/// `grep -f` is deliberately NOT here: its value IS a pattern file and
/// must stay path-checked — see [`PATH_VALUE_FLAGS`]. Two of these
/// flags need extra handling for grep (see the loop): `-e`/`--regexp`
/// supply the pattern (see [`PATTERN_FLAGS`]), and `-n`/`-c`/`-s` are
/// BOOLEAN for grep (--line-number/--count/--no-messages) even though
/// they take a value for head/tail. Same for cat: `-n`/`-A`/`-e`/`-s`
/// are BOOLEAN for cat (--number/--show-all/--show-ends/
/// --squeeze-blank) even though they take a value for other commands.
const VALUE_FLAGS: &[&str] = &[
    "-n",
    "--lines",
    "-c",
    "--bytes",
    "-m",
    "--max-count",
    "-A",
    "--after-context",
    "-B",
    "--before-context",
    "-C",
    "--context",
    "-e",
    "--regexp",
    "--include",
    "--exclude",
    "-s",
    "--sleep-interval",
    "--pid",
];

/// Flags whose value IS grep's inline pattern (`grep -e root`,
/// `grep --regexp root`). Unlike the other [`VALUE_FLAGS`], the value
/// supplies the pattern, so once one is seen the later positionals
/// are files to search, not the pattern — otherwise the file is eaten
/// as the pattern and never path-checked (`grep -e root /etc/shadow`
/// passed the gate while the child printed the root hash line;
/// reviewer finding, M5 follow-up, BLOCKING). These stay in
/// `VALUE_FLAGS` too (the value itself is not a path and is skipped);
/// the loop sets `saw_pattern` for them on top of the skip.
const PATTERN_FLAGS: &[&str] = &["-e", "--regexp"];

/// Flags of the read commands whose following argument is a PATH to
/// read (`grep -f patterns.txt`). Unlike [`VALUE_FLAGS`], the value
/// must be path-checked like any other file argument, NOT skipped:
/// the old code ate the `-f` value as grep's inline pattern, so
/// `grep -f /etc/shadow -e root` passed the gate while the child
/// opened `/etc/shadow` (reviewer finding, M5 follow-up). The
/// pattern comes from this file, so once `-f` is seen the later
/// positionals are files to search, not the pattern.
const PATH_VALUE_FLAGS: &[&str] = &["-f", "--file"];

/// Directory roots under which file-reading commands may read. System
/// diagnostics live here (`/etc/hostname`, `/var/log/*`, `/proc/*`).
/// The Helix working directory itself is NOT an allowed root — its
/// `config.toml` and `.env` hold secrets — only its `logs/` subdir is
/// (resolved at call time from the current directory).
fn allowed_read_roots() -> Vec<std::path::PathBuf> {
    let mut roots = vec![
        std::path::PathBuf::from("/etc"),
        std::path::PathBuf::from("/var/log"),
        std::path::PathBuf::from("/proc"),
        std::path::PathBuf::from("/sys"),
    ];
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd.join("logs"));
    }
    roots
}

/// File names that are never readable via the executor, wherever they
/// live — they hold secrets (auth token, env credentials, password
/// hashes). `shadow`/`gshadow` are the system password-hash files:
/// `/etc` is an allowed root for diagnostics, but the hashes in it
/// are not diagnostics — `grep -f /etc/shadow` must be rejected.
fn is_sensitive_file_name(name: &str) -> bool {
    matches!(name, ".env" | "config.toml" | "shadow" | "gshadow")
}

/// Whether `path` may be read by a file-reading command (M5, task 395).
/// Fail-closed: a path is allowed only when it resolves (symlinks
/// followed, so a link inside an allowed root cannot smuggle out a
/// secret) under an allowed root, AND its file name is not sensitive,
/// AND no path component is `.ssh` (keys live there). A path that
/// cannot be resolved is checked lexically against the same rules.
pub fn is_allowed_read_path(path: &str) -> bool {
    let p = std::path::Path::new(path);
    // Sensitive names are rejected wherever they live.
    if let Some(name) = p.file_name().and_then(|n| n.to_str())
        && is_sensitive_file_name(name)
    {
        return false;
    }
    if p.components()
        .any(|c| matches!(c, std::path::Component::Normal(n) if n == ".ssh"))
    {
        return false;
    }
    // Resolve symlinks when the file exists; otherwise normalise
    // lexically against the current directory.
    let resolved: std::path::PathBuf = if let Ok(canon) = std::fs::canonicalize(p) {
        canon
    } else {
        let base = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let joined = if p.is_absolute() {
            p.to_path_buf()
        } else {
            base.join(p)
        };
        // Lexical normalisation (no symlink resolution): collapse `.`
        // and `..` without touching the filesystem.
        let mut out = std::path::PathBuf::new();
        for comp in joined.components() {
            match comp {
                std::path::Component::ParentDir => {
                    out.pop();
                }
                std::path::Component::CurDir => {}
                other => out.push(other.as_os_str()),
            }
        }
        out
    };
    // The resolved path must itself be clean (a symlink could have
    // pointed at a sensitive name or an .ssh dir).
    if let Some(name) = resolved.file_name().and_then(|n| n.to_str())
        && is_sensitive_file_name(name)
    {
        return false;
    }
    if resolved
        .components()
        .any(|c| matches!(c, std::path::Component::Normal(n) if n == ".ssh"))
    {
        return false;
    }
    allowed_read_roots()
        .iter()
        .any(|root| resolved.starts_with(root))
}

/// Whether `arg` is a `tail -f`-style follow flag (M5, task 395).
/// Covers `-f`, `--follow`, and short-flag clusters containing `f`
/// (`-fn`, `-nf`). A follow would hang the tick forever.
fn is_follow_flag(arg: &str) -> bool {
    if arg == "-f" || arg == "--follow" {
        return true;
    }
    if arg.starts_with('-') && !arg.starts_with("--") && arg.len() > 1 {
        return arg[1..].contains('f');
    }
    false
}

/// The path carried INSIDE a [`PATH_VALUE_FLAGS`] flag token itself:
/// `-fFILE` (short, attached) or `--file=FILE` (long, `=`-separated).
/// Returns the embedded path, or `None` when `arg` is not such a
/// form. The exact `-f` / `--file` forms (value in the next argument)
/// are handled by the loop via `PATH_VALUE_FLAGS`, not here — the
/// value must be path-checked either way, or `grep -f/etc/shadow`
/// would slip past the gate the same way `grep -f /etc/shadow` did.
fn attached_path_value(arg: &str) -> Option<&str> {
    if let Some(rest) = arg.strip_prefix("--file=") {
        return Some(rest);
    }
    if arg.starts_with("-f") && !arg.starts_with("--") && arg.len() > 2 {
        return Some(&arg[2..]);
    }
    None
}

/// Whether `arg` is an attached `-e` pattern form: `-ePATTERN` (short,
/// attached) or `--regexp=PATTERN` (long, `=`-separated). The pattern
/// rides inside the token itself, so (like `-f`) it supplies grep's
/// pattern — later positionals are files to search, not the pattern.
/// The exact `-e` / `--regexp` forms (value in the next argument) are
/// handled by the loop via [`PATTERN_FLAGS`], not here. Without this,
/// `-eroot` fell through as an inert flag and `/etc/shadow` was eaten
/// as the inline pattern (reviewer finding, M5 follow-up, BLOCKING).
/// Checking this BEFORE `cluster_f_value` also fixes the false
/// positive that fix introduced: in `-efoo` the `f` is part of the
/// `-e` VALUE `"foo"`, not a flag, and must not be read as `-f`.
fn is_attached_pattern(arg: &str) -> bool {
    if arg.starts_with("--regexp=") {
        return true;
    }
    arg.starts_with("-e") && !arg.starts_with("--") && arg.len() > 2
}

/// The path value carried by a short-flag CLUSTER containing `f`
/// (`-if FILE`, `-ifFILE`, `-nf /etc/shadow`): the remainder of the
/// token after `f` when attached, or `None` when `f` is the last char
/// and the value is the next argument. Returns `None` when the token
/// is not such a cluster. Mirrors `is_follow_flag`'s cluster walk.
/// A leading `f` (`-f`, `-fFILE`) is NOT handled here — the exact
/// `PATH_VALUE_FLAGS` match and `attached_path_value` own those —
/// so this only fires when `f` is buried in the cluster. Only
/// `PATH_VALUE_FLAGS` (`-f`) needs this; `VALUE_FLAGS` values aren't
/// paths. Without it, `-if` matched no exact entry, fell through as
/// an inert flag, and its value was eaten as grep's inline pattern,
/// so `grep -if /etc/shadow` passed the gate while the child opened
/// `/etc/shadow` (reviewer finding, M5 follow-up, BLOCKING).
///
/// The walk stops at the first short flag that takes a value FOR GREP
/// (`m`, `A`, `B`, `C`, `e`): once one is hit, the rest of the token
/// is its value, not more flags (getopt rule) — so an `f` inside it
/// is not the `-f` flag. NOTE: `n`, `c`, `s` are deliberately NOT in
/// that list — for grep they are boolean (--line-number, --count,
/// --no-messages), so `-nf` really does contain the `-f` flag (the
/// cluster test requires `-nf /etc/shadow` to be rejected); stopping
/// at `n` would re-open that bypass. A cluster like `-iefoo` still
/// misreads (the walk stops at `e`, the token falls through as inert,
/// and the file is eaten as the pattern) — exotic, noted, not fixed
/// here: closing it needs the walk to report a pattern-supplying `e`,
/// which is a bigger change to this helper's return type.
fn cluster_f_value(arg: &str) -> Option<Option<&str>> {
    if !arg.starts_with('-') || arg.starts_with("--") {
        return None;
    }
    // Walk the chars after the dash (char_indices, so a multi-byte
    // flag char before `f` can't misalign the slice); skip the first
    // flag char, which the exact/attached forms own.
    const VALUE_TAKING: &[char] = &['m', 'A', 'B', 'C', 'e'];
    let mut chars = arg[1..].char_indices();
    let first = chars.next()?.1;
    if first == 'f' {
        return None;
    }
    if VALUE_TAKING.contains(&first) {
        return None;
    }
    for (i, c) in chars {
        if VALUE_TAKING.contains(&c) {
            return None;
        }
        if c == 'f' {
            let after = &arg[1 + i + 1..];
            return Some(if after.is_empty() { None } else { Some(after) });
        }
    }
    None
}

/// What a run produced. `stdout`/`stderr` are lossy-UTF-8 decodes of
/// the bytes captured (capped per stream at the caller's cap).
#[derive(Debug, Clone)]
pub struct ExecutionOutcome {
    pub stdout: String,
    pub stderr: String,
    /// Process exit code (`None` when killed by a signal).
    pub exit_code: Option<i32>,
    /// Unix signal number when killed by a signal (`None` otherwise).
    #[allow(dead_code)]
    pub signal: Option<i32>,
    pub timed_out: bool,
    pub duration_ms: u128,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

/// Split a command line into argv. No quoting in v1: any token
/// containing a quote char is rejected by the metachar screen (quotes
/// are in `METACHARS`), so `split_whitespace` is the whole parser.
pub fn parse_command(line: &str) -> Result<Vec<String>, String> {
    let argv: Vec<String> = line.split_whitespace().map(str::to_string).collect();
    if argv.is_empty() {
        return Err("empty command".to_string());
    }
    for tok in &argv {
        if tok
            .chars()
            .any(|c| METACHARS.contains(&c) || c.is_control())
        {
            return Err(format!(
                "rejected: token {tok:?} contains a forbidden character"
            ));
        }
    }
    Ok(argv)
}

/// Local allowlist check, including the `systemctl` subcommand
/// restriction. `systemctl` with no subcommand is rejected (there is
/// no read-only bare form worth allowing).
pub fn check_local_command(argv: &[String]) -> Result<(), String> {
    let Some(first) = argv.first() else {
        return Err("empty command".to_string());
    };
    if !LOCAL_ALLOWLIST.contains(&first.as_str()) {
        return Err(format!(
            "rejected: `{first}` is not in the read-only allowlist"
        ));
    }
    if first == "systemctl" {
        match argv.get(1).map(String::as_str) {
            Some(sub) if SYSTEMCTL_READ_ONLY.contains(&sub) => {}
            Some(sub) => {
                return Err(format!(
                    "rejected: `systemctl {sub}` is not a read-only subcommand"
                ));
            }
            None => return Err("rejected: `systemctl` needs a read-only subcommand".to_string()),
        }
    }
    // M5 (task 395): `hostname` with an argument SETS the hostname
    // (as root) — the read-only form takes no arguments.
    if first == "hostname" && argv.len() > 1 {
        return Err(
            "rejected: `hostname` takes no arguments (with an argument it sets the hostname)"
                .to_string(),
        );
    }
    // M5 (task 395): `tail -f` follows forever and hangs the tick.
    if first == "tail" && argv[1..].iter().any(|a| is_follow_flag(a)) {
        return Err("rejected: `tail -f` (follow) would hang the tick".to_string());
    }
    // M5 (task 395): file-reading commands are confined to the
    // directory allowlist, so they cannot read .env, config.toml, or
    // ~/.ssh. For grep the first non-flag argument is the pattern,
    // not a file; for cat/head/tail every non-flag argument is a file.
    // The value after a flag that takes one (`head -n 5`, `tail -c 100`)
    // is NOT a path and is skipped, as is any bare number (a flag value
    // after a short-flag cluster, e.g. `head -qn 5`). EXCEPTIONS: the
    // value after a `PATH_VALUE_FLAGS` flag (`grep -f patterns.txt`)
    // IS a path and is checked — and once `-f` is seen the pattern
    // comes from that file, so later positionals are files to search,
    // not the pattern. The same holds for a short-flag cluster
    // containing `f` (`grep -if patterns.txt`, `grep -ifpatterns.txt`)
    // — see `cluster_f_value` — and for a pattern-supplying flag
    // (`grep -e root`, `grep --regexp root`, `grep -eroot`,
    // `grep --regexp=root`): `-e`/`--regexp` supply the inline
    // pattern, so later positionals are files too — see
    // `PATTERN_FLAGS` and `is_attached_pattern`. And for grep,
    // `-n`/`-c`/`-s` are boolean (--line-number/--count/
    // --no-messages), so they do NOT skip the next argument — the
    // next argument is the pattern. Same for cat: `-n`/`-A`/`-e`/`-s`
    // are boolean (--number/--show-all/--show-ends/--squeeze-blank),
    // so they do NOT skip the next argument — the next argument is a
    // file to read. `check_path_value` is tested
    // before the flag screen so a value that itself starts with `-`
    // (e.g. `-f -e`) is still path-checked, not mistaken for a flag.
    if READ_COMMANDS.contains(&first.as_str()) {
        let mut saw_pattern = false;
        let mut skip_value = false;
        let mut check_path_value = false;
        for arg in &argv[1..] {
            if skip_value {
                skip_value = false;
                continue;
            }
            if check_path_value {
                check_path_value = false;
                if !is_allowed_read_path(arg) {
                    return Err(format!(
                        "rejected: `{first}` may not read `{arg}` (outside the read allowlist)"
                    ));
                }
                continue;
            }
            if arg.starts_with('-') {
                if VALUE_FLAGS.contains(&arg.as_str()) {
                    // For grep, `-n`/`-c`/`-s` are BOOLEAN
                    // (--line-number/--count/--no-messages) — they
                    // take no value, so the next argument is the
                    // pattern, not a value to skip. Skipping it would
                    // eat the pattern and let the file be consumed as
                    // the "pattern" (`grep -n root /etc/shadow`
                    // bypass, same class as the `-e` finding). For
                    // head/tail they DO take a value, so the skip
                    // only lifts for grep. Same for cat: `-n`/`-A`/
                    // `-e`/`-s` are BOOLEAN (--number/--show-all/
                    // --show-ends/--squeeze-blank), so the next
                    // argument is a file, not a value to skip
                    // (`cat -n /etc/shadow` bypass, reviewer
                    // finding, M5 follow-up, BLOCKING).
                    if !(first == "grep" && matches!(arg.as_str(), "-n" | "-c" | "-s"))
                        && !(first == "cat" && matches!(arg.as_str(), "-n" | "-A" | "-e" | "-s"))
                    {
                        skip_value = true;
                    }
                    if first == "grep" && PATTERN_FLAGS.contains(&arg.as_str()) {
                        saw_pattern = true;
                    }
                } else if PATH_VALUE_FLAGS.contains(&arg.as_str()) {
                    check_path_value = true;
                    if first == "grep" {
                        saw_pattern = true;
                    }
                } else if let Some(value) = attached_path_value(arg) {
                    if !is_allowed_read_path(value) {
                        return Err(format!(
                            "rejected: `{first}` may not read `{value}` (outside the read allowlist)"
                        ));
                    }
                    if first == "grep" {
                        saw_pattern = true;
                    }
                } else if is_attached_pattern(arg) {
                    // `-ePATTERN` / `--regexp=PATTERN`: the pattern
                    // rides inside the token, so later positionals
                    // are files to search. Checked before
                    // `cluster_f_value` — otherwise the `f` in
                    // `-efoo` would be misread as the `-f` flag.
                    if first == "grep" {
                        saw_pattern = true;
                    }
                } else if let Some(clustered) = cluster_f_value(arg) {
                    // A short-flag cluster containing `f` (`-if FILE`,
                    // `-ifFILE`): the `f` carries the pattern file just
                    // like a bare `-f` — attached remainder is checked
                    // now, a trailing `f` takes the next argument.
                    match clustered {
                        Some(value) => {
                            if !is_allowed_read_path(value) {
                                return Err(format!(
                                    "rejected: `{first}` may not read `{value}` (outside the read allowlist)"
                                ));
                            }
                        }
                        None => check_path_value = true,
                    }
                    if first == "grep" {
                        saw_pattern = true;
                    }
                }
                continue;
            }
            if !arg.is_empty() && arg.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            if first == "grep" && !saw_pattern {
                saw_pattern = true;
                continue;
            }
            if !is_allowed_read_path(arg) {
                return Err(format!(
                    "rejected: `{first}` may not read `{arg}` (outside the read allowlist)"
                ));
            }
        }
    }
    Ok(())
}

/// Drain a pipe to end, keeping at most `cap` bytes. Reading never
/// stops early: a chatty child must never block on a full pipe, so
/// bytes past the cap are read and discarded (and `truncated` is set).
fn drain_capped<R: Read>(mut reader: R, cap: usize) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut truncated = false;
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if kept.len() < cap {
                    let room = cap - kept.len();
                    let take = n.min(room);
                    kept.extend_from_slice(&buf[..take]);
                    if n > take {
                        truncated = true;
                    }
                } else {
                    truncated = true;
                }
            }
            Err(_) => break,
        }
    }
    (kept, truncated)
}

#[cfg(unix)]
fn kill_tree(pid: u32) {
    // The child runs in its own process group (see pre_exec below),
    // so SIGKILL to the group reaps backgrounded grandchildren too.
    unsafe {
        libc::killpg(pid as i32, libc::SIGKILL);
    }
}

/// Grace period for the drain threads to finish after the child is
/// reaped. A well-behaved child closes its pipes on exit, so EOF is
/// immediate; the grace only bounds the pathological case (H9): a
/// background child that escaped the process-group kill (e.g. via
/// `setsid`) keeps a pipe open forever, and an unbounded `join()`
/// would hang the caller past the timeout.
const DRAIN_JOIN_GRACE: Duration = Duration::from_secs(2);

/// Join a drain thread, giving up after [`DRAIN_JOIN_GRACE`].
/// Returns `(bytes, truncated, abandoned)`. When the thread is still
/// blocked on a pipe held open by an escaped background child, the
/// join is abandoned and empty output is returned — the thread is
/// detached (its `JoinHandle` is dropped) and stays blocked on read
/// until the escaped child exits, then finishes harmlessly. Same
/// bounded-drain shape as the evaluator's `recv_timeout` (task 377,
/// H10); partial output already read into the thread's local buffer
/// is dropped on abandon, matching that precedent.
fn join_drain(handle: std::thread::JoinHandle<(Vec<u8>, bool)>) -> (Vec<u8>, bool, bool) {
    let start = Instant::now();
    loop {
        if handle.is_finished() {
            let (bytes, truncated) = handle.join().unwrap_or_default();
            return (bytes, truncated, false);
        }
        if start.elapsed() >= DRAIN_JOIN_GRACE {
            return (Vec::new(), false, true);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Run argv directly (NO shell) with a wall-clock timeout and a
/// per-stream output cap. stdin is /dev/null (no TTY — interactive
/// prompts fail fast). On timeout the process (group) is killed and
/// `timed_out` is set; partial output captured so far is returned.
pub fn run_command_with_timeout(
    argv: &[String],
    timeout: Duration,
    cap_per_stream: usize,
) -> std::io::Result<ExecutionOutcome> {
    let Some((program, args)) = argv.split_first() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty argv",
        ));
    };
    let started = Instant::now();
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            // New process group: on timeout we SIGKILL the group, not
            // just the child, so backgrounded grandchildren die too.
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = cmd.spawn()?;
    // `pid` is only read by the Unix `kill_tree` call below; on
    // Windows the timeout path uses `child.kill()` instead, so
    // binding it unconditionally trips `-D unused-variables`.
    #[cfg(unix)]
    let pid = child.id();
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_handle = std::thread::spawn(move || {
        out_pipe
            .as_mut()
            .map(|p| drain_capped(p, cap_per_stream))
            .unwrap_or_default()
    });
    let err_handle = std::thread::spawn(move || {
        err_pipe
            .as_mut()
            .map(|p| drain_capped(p, cap_per_stream))
            .unwrap_or_default()
    });

    let mut timed_out = false;
    let status;
    loop {
        match child.try_wait()? {
            Some(s) => {
                status = Some(s);
                break;
            }
            None => {
                if started.elapsed() >= timeout {
                    timed_out = true;
                    #[cfg(unix)]
                    kill_tree(pid);
                    #[cfg(not(unix))]
                    let _ = child.kill();
                    // Reap: closes the pipes so the drain threads see EOF.
                    status = child.wait().ok();
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }

    // Bounded join (H9): a background child that escaped the group
    // kill (e.g. via `setsid`) holds a pipe open past the timeout,
    // and an unbounded join() would hang the caller forever. An
    // abandoned drain means the stream's output is incomplete, so
    // it folds into the truncation flags — the reply renders
    // `[truncated]`, the same signal as a cap cut. No struct change.
    let (stdout_bytes, stdout_truncated, stdout_abandoned) = join_drain(out_handle);
    let (stderr_bytes, stderr_truncated, stderr_abandoned) = join_drain(err_handle);
    let stdout_truncated = stdout_truncated || stdout_abandoned;
    let stderr_truncated = stderr_truncated || stderr_abandoned;
    Ok(ExecutionOutcome {
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        exit_code: status.and_then(|s| s.code()),
        #[cfg(unix)]
        signal: {
            use std::os::unix::process::ExitStatusExt;
            status.and_then(|s| s.signal())
        },
        #[cfg(not(unix))]
        signal: None,
        timed_out,
        duration_ms: started.elapsed().as_millis(),
        stdout_truncated,
        stderr_truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn parse_splits_on_whitespace() {
        assert_eq!(parse_command("ps aux").unwrap(), argv(&["ps", "aux"]));
        assert_eq!(parse_command("  ls   -la  ").unwrap(), argv(&["ls", "-la"]));
    }

    #[test]
    fn parse_rejects_empty() {
        assert!(parse_command("").is_err());
        assert!(parse_command("   ").is_err());
    }

    #[test]
    fn parse_rejects_every_metachar_class() {
        // Pipe, redirect, chain, substitution, glob, quote, tilde, group.
        for cmd in [
            "ps aux | grep x",
            "ls > /tmp/x",
            "ls 2> /tmp/x",
            "ps; ls",
            "ps && ls",
            "echo $(whoami)",
            "echo `whoami`",
            "ls *.rs",
            "ls ~/x",
            "echo \"hi\"",
            "echo 'hi'",
            "ps (aux)",
            "echo {a,b}",
            "echo a?b",
        ] {
            assert!(parse_command(cmd).is_err(), "must reject: {cmd}");
        }
    }

    #[test]
    fn parse_rejects_control_chars() {
        // Whitespace controls (\n, \t) are separators by construction
        // (split_whitespace) — the screen's job is NON-whitespace
        // controls smuggled inside a token (escape, NUL, ...).
        assert!(parse_command("ls\x1b/tmp").is_err());
        assert!(parse_command("ls\x00x").is_err());
        assert!(parse_command("ls\x07x").is_err());
    }

    #[test]
    fn local_allowlist_accepts_each_command() {
        for cmd in [
            "cat", "tail", "head", "grep", "ls", "ps", "df", "free", "uptime", "whoami", "hostname",
        ] {
            assert!(
                check_local_command(&argv(&[cmd])).is_ok(),
                "{cmd} must be allowed"
            );
        }
        assert!(check_local_command(&argv(&["systemctl", "status"])).is_ok());
        assert!(check_local_command(&argv(&["systemctl", "is-active", "helix"])).is_ok());
        assert!(check_local_command(&argv(&["systemctl", "show", "helix"])).is_ok());
        assert!(check_local_command(&argv(&["systemctl", "list-units"])).is_ok());
    }

    #[test]
    fn local_allowlist_rejects_writes_and_bad_systemctl() {
        for words in [
            vec!["rm", "-rf", "/"],
            vec!["cp", "a", "b"],
            vec!["mv", "a", "b"],
            vec!["touch", "x"],
            vec!["systemctl", "restart", "helix"],
            vec!["systemctl", "stop", "helix"],
            vec!["systemctl", "enable", "helix"],
            vec!["systemctl"],
            vec!["chmod", "+x", "x"],
            vec!["curl", "http://example.com"],
        ] {
            assert!(
                check_local_command(&argv(&words)).is_err(),
                "{words:?} must be rejected"
            );
        }
    }

    // ── M5 (task 395): executor arg validation ───────────────────────────

    #[test]
    fn hostname_with_argument_is_rejected() {
        // `hostname newname` SETS the hostname (as root) — the read-only
        // form takes no arguments.
        assert!(check_local_command(&argv(&["hostname"])).is_ok());
        assert!(check_local_command(&argv(&["hostname", "newname"])).is_err());
    }

    #[test]
    fn tail_follow_is_rejected() {
        // `tail -f` follows forever and hangs the tick.
        for words in [
            vec!["tail", "-f", "/etc/hostname"],
            vec!["tail", "--follow", "/etc/hostname"],
            vec!["tail", "-fn", "5", "/etc/hostname"],
        ] {
            assert!(
                check_local_command(&argv(&words)).is_err(),
                "{words:?} must be rejected"
            );
        }
        // Plain tail of an allowed file is fine.
        assert!(check_local_command(&argv(&["tail", "/etc/hostname"])).is_ok());
    }

    #[test]
    fn read_commands_cannot_read_secrets() {
        // .env, config.toml, and anything under .ssh are off-limits,
        // however the path is spelled (absolute, relative, via ..).
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        for words in [
            vec!["cat", ".env"],
            vec!["cat", "config.toml"],
            vec!["cat", "/home/cobble/helix/.env"],
            vec!["cat", "/home/cobble/helix/config.toml"],
            vec!["cat", "~/.ssh/id_rsa"],
            vec!["cat", &format!("{home}/.ssh/id_rsa")],
            vec!["grep", "token", ".env"],
            vec!["grep", "token", "config.toml"],
            vec!["tail", "/home/cobble/.ssh/known_hosts"],
            vec!["head", "logs/../config.toml"],
        ] {
            assert!(
                check_local_command(&argv(&words)).is_err(),
                "{words:?} must be rejected"
            );
        }
    }

    #[test]
    fn read_commands_allow_system_files() {
        // System diagnostics stay readable: /etc/hostname is the
        // existing test's path, and /var/log is the log root.
        assert!(check_local_command(&argv(&["cat", "/etc/hostname"])).is_ok());
        assert!(check_local_command(&argv(&["grep", "root", "/etc/hostname"])).is_ok());
        // But paths outside the allowlist (e.g. /tmp) are not.
        assert!(check_local_command(&argv(&["cat", "/tmp/somefile"])).is_err());
    }

    #[test]
    fn read_commands_allow_flag_values() {
        // M5 (task 395) fix: the value after a flag that takes one is
        // not a path — the most common diagnostic invocations must pass.
        assert!(check_local_command(&argv(&["head", "-n", "5", "/etc/hostname"])).is_ok());
        assert!(check_local_command(&argv(&["tail", "-n", "20", "/var/log/syslog"])).is_ok());
        assert!(check_local_command(&argv(&["head", "-c", "100", "/etc/hostname"])).is_ok());
        assert!(check_local_command(&argv(&["grep", "-m", "3", "root", "/etc/hostname"])).is_ok());
        // A disallowed path after the flag value is still rejected.
        assert!(check_local_command(&argv(&["head", "-n", "5", "/tmp/somefile"])).is_err());
    }

    #[test]
    fn grep_pattern_file_is_path_checked() {
        // Reviewer finding (M5 follow-up): `grep -f` takes a pattern
        // FILE, and the old code ate its value as the inline pattern,
        // so `grep -f /etc/shadow -e root` passed the gate while the
        // child opened /etc/shadow. The -f value is a path and is
        // checked like any other file argument.
        assert!(
            check_local_command(&argv(&["grep", "-f", "/etc/shadow", "-e", "root"])).is_err(),
            "the reported bypass must be rejected"
        );
        assert!(
            check_local_command(&argv(&[
                "grep",
                "--file",
                "/etc/shadow",
                "root",
                "/etc/hostname"
            ]))
            .is_err()
        );
        // An allowlisted pattern file passes — and with -f the pattern
        // comes from the file, so a positional is a file to search,
        // not a pattern, and is checked too.
        assert!(
            check_local_command(&argv(&["grep", "-f", "/etc/hostname", "/etc/hostname"])).is_ok()
        );
        assert!(
            check_local_command(&argv(&[
                "grep",
                "-f",
                "/etc/hostname",
                "-e",
                "root",
                "/etc/hostname"
            ]))
            .is_ok()
        );
        assert!(
            check_local_command(&argv(&["grep", "-f", "/etc/hostname", "config.toml"])).is_err(),
            "a file searched via -f is not a pattern"
        );
        assert!(
            check_local_command(&argv(&["grep", "-f", "/etc/hostname", "/tmp/somefile"])).is_err()
        );
        // Attached forms carry the path inside the flag token itself.
        assert!(check_local_command(&argv(&["grep", "-f/etc/shadow"])).is_err());
        assert!(check_local_command(&argv(&["grep", "--file=/etc/shadow"])).is_err());
        assert!(check_local_command(&argv(&["grep", "-f/etc/hostname", "/etc/hostname"])).is_ok());
    }

    #[test]
    fn grep_pattern_file_in_flag_cluster_is_path_checked() {
        // Reviewer finding (M5 follow-up, BLOCKING): a short-flag
        // cluster containing `f` (`-if`, `-nf`, `-qf`) slipped past
        // the gate — the token matched no exact flag entry and
        // `attached_path_value` only handles a leading `-f`, so
        // `/etc/shadow` was eaten as the inline pattern and never
        // path-checked. The cluster's `f` carries the pattern file
        // just like a bare `-f`.
        for words in [
            vec!["grep", "-if", "/etc/shadow"],
            vec!["grep", "-nf", "/etc/shadow"],
            vec!["grep", "-qf", "/etc/shadow"],
            vec!["grep", "-if/etc/shadow"],
            vec!["grep", "-nf", "/etc/shadow", "/etc/hostname"],
        ] {
            assert!(
                check_local_command(&argv(&words)).is_err(),
                "{words:?} must be rejected"
            );
        }
        // An allowlisted pattern file via a cluster passes, and with
        // -f the pattern comes from the file, so a positional is a
        // file to search and is checked too.
        assert!(
            check_local_command(&argv(&["grep", "-if", "/etc/hostname", "/etc/hostname"])).is_ok()
        );
        assert!(check_local_command(&argv(&["grep", "-if/etc/hostname", "/etc/hostname"])).is_ok());
        assert!(
            check_local_command(&argv(&["grep", "-if", "/etc/hostname", "config.toml"])).is_err(),
            "a file searched via a -f cluster is not a pattern"
        );
        // A legitimate cluster without `f` still works.
        assert!(check_local_command(&argv(&["grep", "-in", "root", "/etc/hostname"])).is_ok());
    }

    #[test]
    fn grep_pattern_flag_supplies_pattern() {
        // Reviewer finding (M5 follow-up, BLOCKING): `-e`/`--regexp`
        // supply grep's inline pattern, just like `-f` supplies it
        // from a file — but the VALUE_FLAGS branch only skipped the
        // value and never set `saw_pattern`, so the file was eaten
        // as the inline pattern and never path-checked. `grep -e root
        // /etc/shadow` passed the gate while the child printed the
        // root hash line. The same hole covers the attached forms
        // (`-eroot`, `--regexp=root`) and the empty pattern (`-e ""`
        // matches every line — a full file dump).
        for words in [
            vec!["grep", "-e", "root", "/etc/shadow"],
            vec!["grep", "--regexp", "root", "/etc/shadow"],
            vec!["grep", "-eroot", "/etc/shadow"],
            vec!["grep", "--regexp=root", "/etc/shadow"],
            vec!["grep", "-e", "", "/etc/shadow"],
        ] {
            assert!(
                check_local_command(&argv(&words)).is_err(),
                "{words:?} must be rejected"
            );
        }
        // An allowlisted file passes with `-e`, in every form — and
        // with the pattern supplied, a positional is a file to search.
        for words in [
            vec!["grep", "-e", "root", "/etc/hostname"],
            vec!["grep", "--regexp", "root", "/etc/hostname"],
            vec!["grep", "-eroot", "/etc/hostname"],
            vec!["grep", "--regexp=root", "/etc/hostname"],
            vec!["grep", "-e", "", "/etc/hostname"],
        ] {
            assert!(
                check_local_command(&argv(&words)).is_ok(),
                "{words:?} must be allowed"
            );
        }
        // Same class, same branch: `-n`/`-c`/`-s` are BOOLEAN for
        // grep (--line-number/--count/--no-messages) but the shared
        // VALUE_FLAGS list treats them as value-taking, so the real
        // pattern was skipped and the file eaten as the "pattern".
        for words in [
            vec!["grep", "-n", "root", "/etc/shadow"],
            vec!["grep", "-c", "root", "/etc/shadow"],
            vec!["grep", "-s", "root", "/etc/shadow"],
        ] {
            assert!(
                check_local_command(&argv(&words)).is_err(),
                "{words:?} must be rejected"
            );
        }
        assert!(check_local_command(&argv(&["grep", "-n", "root", "/etc/hostname"])).is_ok());
        assert!(check_local_command(&argv(&["grep", "-c", "root", "/etc/hostname"])).is_ok());
        assert!(check_local_command(&argv(&["grep", "-s", "root", "/etc/hostname"])).is_ok());
        // False positive from the cluster fix (49af476):
        // `cluster_f_value` scanned for `f` anywhere after the dash,
        // so in `-efoo` the `f` of the `-e` VALUE `"foo"` was read as
        // the `-f` flag and `"oo"` path-checked. `-efoo` is `-e`
        // with pattern `"foo"`.
        assert!(check_local_command(&argv(&["grep", "-efoo", "/etc/hostname"])).is_ok());
        assert!(check_local_command(&argv(&["grep", "-efailed", "/var/log/syslog"])).is_ok());
    }

    #[test]
    fn cat_boolean_flags_do_not_skip_path() {
        // Reviewer finding (M5 follow-up, BLOCKING): `-n`/`-A`/`-e`/`-s`
        // are BOOLEAN for cat (--number/--show-all/--show-ends/
        // --squeeze-blank) but the shared VALUE_FLAGS list treats them
        // as value-taking, so the file was skipped as the flag's value
        // and never path-checked. `cat -n /etc/shadow` passed the gate
        // while real cat printed the file with line numbers — the same
        // class as the grep `-n`/`-c`/`-s` carve-out, and `cat -n` is
        // a form people actually type.
        for words in [
            vec!["cat", "-n", "/etc/shadow"],
            vec!["cat", "-e", "/etc/shadow"],
            vec!["cat", "-s", "/etc/shadow"],
            vec!["cat", "-A", "/etc/shadow"],
        ] {
            assert!(
                check_local_command(&argv(&words)).is_err(),
                "{words:?} must be rejected"
            );
        }
        // An allowlisted file passes with each boolean flag.
        for words in [
            vec!["cat", "-n", "/etc/hostname"],
            vec!["cat", "-e", "/etc/hostname"],
            vec!["cat", "-s", "/etc/hostname"],
            vec!["cat", "-A", "/etc/hostname"],
        ] {
            assert!(
                check_local_command(&argv(&words)).is_ok(),
                "{words:?} must be allowed"
            );
        }
    }

    #[test]
    fn run_executes_allowlisted_command() {
        let out = run_command_with_timeout(&argv(&["hostname"]), Duration::from_secs(10), 4096)
            .expect("hostname must run");
        assert!(!out.timed_out);
        assert_eq!(out.exit_code, Some(0));
        assert!(!out.stdout.trim().is_empty(), "hostname prints something");
    }

    #[test]
    fn run_reports_nonzero_exit() {
        // `grep` with no match exits 1 — a real non-zero path, not a stub.
        let out = run_command_with_timeout(
            &argv(&["grep", "definitely-not-present-zzz", "/etc/hostname"]),
            Duration::from_secs(10),
            4096,
        )
        .expect("grep must run");
        assert!(!out.timed_out);
        assert_eq!(out.exit_code, Some(1));
    }

    #[test]
    fn run_times_out_without_hanging() {
        let start = Instant::now();
        let out = run_command_with_timeout(&argv(&["sleep", "30"]), Duration::from_secs(1), 4096)
            .expect("sleep must spawn");
        assert!(out.timed_out, "30s sleep must time out at 1s");
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timeout must bound the run, took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn run_setsid_child_cannot_hold_pipe_open_forever() {
        // H9: `setsid` forks and its parent exits at once, but the
        // orphaned sleep (new session, escapes the group kill)
        // inherits stdout, so the drain thread never sees EOF and the
        // unbounded join() hangs forever. Must return within the
        // drain grace, with the setsid parent's exit code 0.
        let start = Instant::now();
        let out = run_command_with_timeout(
            &argv(&["setsid", "sleep", "30"]),
            Duration::from_secs(10),
            4096,
        )
        .expect("setsid must spawn");
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "setsid child held the pipe open past the deadline: {:?}",
            start.elapsed()
        );
        assert!(!out.timed_out);
        assert_eq!(out.exit_code, Some(0));
    }

    #[test]
    fn run_timeout_with_setsid_grandchild_cannot_hold_pipe_open() {
        // H9, timeout path: the direct child (sh) sleeps while a
        // setsid'd grandchild escapes the group kill and holds the
        // pipe open past the timeout.
        let start = Instant::now();
        let argv = vec![
            "sh".to_string(),
            "-c".to_string(),
            "setsid sleep 30 & sleep 30".to_string(),
        ];
        let out =
            run_command_with_timeout(&argv, Duration::from_secs(2), 4096).expect("sh must spawn");
        assert!(out.timed_out, "30s sleep must time out at 2s");
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "setsid grandchild held the pipe open past the deadline: {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn run_caps_output_and_marks_truncation() {
        // `seq` is not allowlisted, but the runner itself takes argv
        // directly — the cap behavior is independent of the allowlist.
        let out = run_command_with_timeout(
            &argv(&["seq", "1", "100000"]),
            Duration::from_secs(15),
            1024,
        )
        .expect("seq must run");
        assert!(out.stdout_truncated, "100k lines must exceed a 1K cap");
        assert!(out.stdout.len() <= 1024);
    }
}
