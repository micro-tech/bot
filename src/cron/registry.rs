//! Routine registry + scheduler (task 201).
//!
//! Named, scheduled, enable-able jobs persisted as JSON under the runtime
//! dir. This replaces the dead `cron_handler.rs` stub, which was never
//! compiled (`mod cron` was never declared) and had real bugs anyway: a
//! missing chrono import, `Utc::now().hour() % 1 == 0` (always true), and a
//! 5-second `agent_run` spam loop. Cadence here comes from each job's own
//! schedule — there is no global tick.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::bus::{Bus, Message};

// ── Routine ─────────────────────────────────────────────────────────────────

/// A named scheduled job. `schedule` is the canonical machine form
/// ("daily 02:00", "every 6h"); the WS API serves the human form.
///
/// Task 216: routines can carry an agent job payload. When `task` is
/// non-empty, firing the routine dispatches an agent run (via the same path
/// as the `spawn_subagent` tool) instead of the legacy hardcoded handlers.
/// All payload fields are serde-defaulted so pre-216 routine files keep
/// working unchanged.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Routine {
    pub id: String,
    pub name: String,
    pub schedule: String,
    pub enabled: bool,
    #[serde(default)]
    pub last_run_ms: Option<u64>,
    #[serde(default)]
    pub next_run_ms: Option<u64>,
    /// Agent role hint for the job (e.g. "quartermaster"); free text.
    #[serde(default)]
    pub agent_role: String,
    /// The marching orders. Empty = legacy routine (hardcoded handler by id).
    #[serde(default)]
    pub task: String,
    /// Backend for the agent run: local_ollama | lan_ollama | gemini | grok | auto.
    #[serde(default = "default_routine_backend")]
    pub backend: String,
}

/// True when this routine carries an agent job (vs. a legacy hardcoded id).
impl Routine {
    pub fn has_agent_job(&self) -> bool {
        !self.task.trim().is_empty()
    }
}

fn default_routine_backend() -> String {
    "auto".to_string()
}

// ── Schedules ───────────────────────────────────────────────────────────────

/// Parsed schedule. Two forms, both unambiguous and testable:
/// `daily HH:MM` (local time) and `every <n>s|m|h|d`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Schedule {
    Daily { hour: u32, min: u32 },
    Every { secs: u64 },
}

pub fn parse_schedule(s: &str) -> Result<Schedule, String> {
    let lower = s.trim().to_lowercase();
    if let Some(rest) = lower.strip_prefix("daily") {
        let rest = rest.trim();
        let (h, m) = rest
            .split_once(':')
            .ok_or_else(|| format!("bad daily schedule {:?}: want \"daily HH:MM\"", s))?;
        let hour: u32 = h
            .trim()
            .parse()
            .map_err(|_| format!("bad hour in {:?}", s))?;
        let min: u32 = m
            .trim()
            .parse()
            .map_err(|_| format!("bad minute in {:?}", s))?;
        if hour > 23 || min > 59 {
            return Err(format!("hour/min out of range in {:?}", s));
        }
        Ok(Schedule::Daily { hour, min })
    } else if let Some(rest) = lower.strip_prefix("every") {
        let rest = rest.trim();
        let (num, unit) = match rest.find(|c: char| !c.is_ascii_digit()) {
            Some(i) => rest.split_at(i),
            None => return Err(format!("bad every schedule {:?}: want \"every <n>s|m|h|d\"", s)),
        };
        let n: u64 = num
            .parse()
            .map_err(|_| format!("bad count in {:?}", s))?;
        if n == 0 {
            return Err(format!("zero interval in {:?}", s));
        }
        let secs = match unit.trim() {
            "s" | "sec" | "secs" | "second" | "seconds" => n,
            "m" | "min" | "mins" | "minute" | "minutes" => n * 60,
            "h" | "hr" | "hrs" | "hour" | "hours" => n * 3600,
            "d" | "day" | "days" => n * 86400,
            u => return Err(format!("bad unit {:?} in {:?}", u, s)),
        };
        Ok(Schedule::Every { secs })
    } else {
        Err(format!(
            "unknown schedule {:?}: want \"daily HH:MM\" or \"every <n>s|m|h|d\"",
            s
        ))
    }
}

/// Human form for the UI, e.g. "Every day at 2:00 AM", "Every 6 hours".
/// Task 197's contract shows `schedule` as this text.
pub fn schedule_display(sched: Schedule) -> String {
    match sched {
        Schedule::Daily { hour, min } => {
            let (h12, ampm) = match hour {
                0 => (12, "AM"),
                1..=11 => (hour, "AM"),
                12 => (12, "PM"),
                _ => (hour - 12, "PM"),
            };
            format!("Every day at {}:{:02} {}", h12, min, ampm)
        }
        Schedule::Every { secs } => {
            if secs % 86400 == 0 {
                let d = secs / 86400;
                format!("Every {} day{}", d, if d == 1 { "" } else { "s" })
            } else if secs % 3600 == 0 {
                let h = secs / 3600;
                format!("Every {} hour{}", h, if h == 1 { "" } else { "s" })
            } else if secs % 60 == 0 {
                let m = secs / 60;
                format!("Every {} minute{}", m, if m == 1 { "" } else { "s" })
            } else {
                format!("Every {} second{}", secs, if secs == 1 { "" } else { "s" })
            }
        }
    }
}

/// Next occurrence of `sched` strictly after `from_ms` (epoch millis).
/// Daily jobs use the machine's local timezone.
pub fn next_run_after(sched: Schedule, from_ms: u64) -> u64 {
    match sched {
        Schedule::Every { secs } => from_ms.saturating_add(secs.saturating_mul(1000)),
        Schedule::Daily { hour, min } => {
            use chrono::{Local, TimeZone};
            let from = Local
                .timestamp_millis_opt(from_ms as i64)
                .single()
                .unwrap_or_else(Local::now);
            let today = from
                .date_naive()
                .and_hms_opt(hour, min, 0)
                .map(|naive| {
                    Local
                        .from_local_datetime(&naive)
                        .single()
                        .map(|dt| dt.timestamp_millis() as u64)
                })
                .flatten();
            match today {
                Some(t) if t > from_ms => t,
                _ => {
                    // Tomorrow at HH:MM.
                    let tomorrow = from.date_naive() + chrono::Duration::days(1);
                    tomorrow
                        .and_hms_opt(hour, min, 0)
                        .and_then(|naive| {
                            Local
                                .from_local_datetime(&naive)
                                .single()
                                .map(|dt| dt.timestamp_millis() as u64)
                        })
                        .unwrap_or_else(|| from_ms.saturating_add(86_400_000))
                }
            }
        }
    }
}

// ── Registry ────────────────────────────────────────────────────────────────

/// The persisted routine set. Owns the JSON file; all mutations save.
pub struct RoutineRegistry {
    path: PathBuf,
    routines: Vec<Routine>,
}

fn default_routines() -> Vec<Routine> {
    vec![
        Routine {
            id: "nightly_memory_consolidation".to_string(),
            name: "Nightly memory consolidation".to_string(),
            // hartbeat.md's 1–7am maintenance window.
            schedule: "daily 02:00".to_string(),
            enabled: true,
            last_run_ms: None,
            next_run_ms: None,
            agent_role: String::new(),
            task: String::new(),
            backend: default_routine_backend(),
        },
        Routine {
            id: "error_log_review".to_string(),
            name: "Error log review".to_string(),
            schedule: "daily 06:30".to_string(),
            enabled: true,
            last_run_ms: None,
            next_run_ms: None,
            agent_role: String::new(),
            task: String::new(),
            backend: default_routine_backend(),
        },
        // Task 216: John's twice-daily mail check as a payload routine —
        // the template for "agent starts, does a task, stops".
        Routine {
            id: "mail_check".to_string(),
            name: "Inbox check (twice daily)".to_string(),
            schedule: "every 12h".to_string(),
            enabled: false, // John enables it when he wants the agent watching mail
            last_run_ms: None,
            next_run_ms: None,
            agent_role: "quartermaster".to_string(),
            task: "Check the Gmail inbox for urgent or actionable mail (bills,                    township business, customer issues). Summarize anything needing                    John's attention; do not send replies without his approval."
                .to_string(),
            backend: "auto".to_string(),
        },
    ]
}

impl RoutineRegistry {
    /// Load from `path`, seeding John's hartbeat.md jobs when the file is
    /// missing or unreadable. Jobs due while the bot was offline are skipped
    /// (next run is the next future occurrence) — no catch-up in v1.
    pub fn load_or_seed(path: PathBuf) -> Self {
        let routines = std::fs::read_to_string(&path)
            .ok()
            .and_then(|data| serde_json::from_str::<Vec<Routine>>(&data).ok());
        let mut reg = match routines {
            Some(rs) => {
                println!("cron: loaded {} routines from {}", rs.len(), path.display());
                Self { path, routines: rs }
            }
            None => {
                println!("cron: seeding default routines at {}", path.display());
                Self {
                    path,
                    routines: default_routines(),
                }
            }
        };
        reg.refresh_next_runs(crate::utils::now_ms());
        if let Err(e) = reg.save() {
            eprintln!("cron: failed to persist routines: {}", e);
        }
        reg
    }

    /// Recompute next_run for every routine from `now_ms`, leaving already-
    /// scheduled future runs alone.
    fn refresh_next_runs(&mut self, now_ms: u64) {
        for r in &mut self.routines {
            let needs = match r.next_run_ms {
                Some(n) => n <= now_ms,
                None => true,
            };
            if !needs {
                continue;
            }
            match parse_schedule(&r.schedule) {
                Ok(sched) => r.next_run_ms = Some(next_run_after(sched, now_ms)),
                Err(e) => {
                    eprintln!("cron: routine '{}' has {}", r.id, e);
                    r.next_run_ms = None;
                }
            }
        }
    }

    pub fn list(&self) -> &[Routine] {
        &self.routines
    }

    /// Enable/disable a routine by id; persists. Returns the updated routine.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Option<Routine> {
        let idx = self.routines.iter().position(|r| r.id == id)?;
        self.routines[idx].enabled = enabled;
        if enabled {
            self.refresh_next_runs(crate::utils::now_ms());
        }
        let updated = self.routines[idx].clone();
        if let Err(e) = self.save() {
            eprintln!("cron: failed to persist toggle: {}", e);
        }
        Some(updated)
    }

    // ── Task 220: CRUD for the routines editor UI ─────────────────────────

    /// Validate a backend name the way the UI select sends it (canonical
    /// form). Returns the normalized value or an error string.
    fn normalize_backend(s: &str) -> Result<String, String> {
        match s.trim().to_lowercase().as_str() {
            "" | "auto" => Ok("auto".to_string()),
            "local_ollama" | "lan_ollama" | "gemini" | "grok" => {
                Ok(s.trim().to_lowercase())
            }
            other => Err(format!(
                "unknown backend '{}': use local_ollama | lan_ollama | gemini | grok | auto",
                other
            )),
        }
    }

    /// Unique id derived from the name ("Inbox check" -> "inbox_check",
    /// "inbox_check-2" on collision).
    fn unique_id(&self, name: &str) -> String {
        let mut base: String = name
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        // Collapse runs of '_' and trim them from the ends.
        let mut clean = String::with_capacity(base.len());
        let mut prev_us = false;
        for c in base.chars() {
            if c == '_' {
                if !prev_us {
                    clean.push(c);
                }
                prev_us = true;
            } else {
                clean.push(c);
                prev_us = false;
            }
        }
        base = clean.trim_matches('_').to_string();
        if base.is_empty() {
            base = "routine".to_string();
        }
        let mut id = base.clone();
        let mut n = 2;
        while self.routines.iter().any(|r| r.id == id) {
            id = format!("{}-{}", base, n);
            n += 1;
        }
        id
    }

    /// Create a routine. The schedule must be canonical
    /// ("daily HH:MM" / "every <n>s|m|h|d"); the backend must be known.
    /// Persists. Returns the created routine.
    pub fn add(
        &mut self,
        name: &str,
        schedule: &str,
        agent_role: &str,
        task: &str,
        backend: &str,
        enabled: bool,
    ) -> Result<Routine, String> {
        parse_schedule(schedule).map_err(|e| format!("bad schedule: {}", e))?;
        let backend = Self::normalize_backend(backend)?;
        if name.trim().is_empty() {
            return Err("name must not be empty".to_string());
        }
        let r = Routine {
            id: self.unique_id(name),
            name: name.trim().to_string(),
            schedule: schedule.trim().to_string(),
            enabled,
            last_run_ms: None,
            next_run_ms: None,
            agent_role: agent_role.to_string(),
            task: task.to_string(),
            backend,
        };
        self.routines.push(r.clone());
        self.refresh_next_runs(crate::utils::now_ms());
        if let Err(e) = self.save() {
            eprintln!("cron: failed to persist new routine: {}", e);
        }
        Ok(self
            .routines
            .iter()
            .find(|x| x.id == r.id)
            .cloned()
            .unwrap_or(r))
    }

    /// Patch a routine's fields by id. `None` = leave unchanged. A supplied
    /// schedule/backend is validated. Persists. Returns the updated routine
    /// or an error string.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        id: &str,
        name: Option<&str>,
        schedule: Option<&str>,
        agent_role: Option<&str>,
        task: Option<&str>,
        backend: Option<&str>,
        enabled: Option<bool>,
    ) -> Result<Routine, String> {
        let idx = self
            .routines
            .iter()
            .position(|r| r.id == id)
            .ok_or_else(|| "unknown routine id".to_string())?;
        if let Some(s) = schedule {
            parse_schedule(s).map_err(|e| format!("bad schedule: {}", e))?;
            self.routines[idx].schedule = s.trim().to_string();
        }
        if let Some(b) = backend {
            self.routines[idx].backend = Self::normalize_backend(b)?;
        }
        if let Some(n) = name {
            if n.trim().is_empty() {
                return Err("name must not be empty".to_string());
            }
            self.routines[idx].name = n.trim().to_string();
        }
        if let Some(ar) = agent_role {
            self.routines[idx].agent_role = ar.to_string();
        }
        if let Some(t) = task {
            self.routines[idx].task = t.to_string();
        }
        if let Some(e) = enabled {
            self.routines[idx].enabled = e;
        }
        self.refresh_next_runs(crate::utils::now_ms());
        let updated = self.routines[idx].clone();
        if let Err(e) = self.save() {
            eprintln!("cron: failed to persist routine update: {}", e);
        }
        Ok(updated)
    }

    /// Delete a routine by id. Persists. Returns true when one was removed.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.routines.len();
        self.routines.retain(|r| r.id != id);
        let removed = self.routines.len() != before;
        if removed {
            if let Err(e) = self.save() {
                eprintln!("cron: failed to persist routine delete: {}", e);
            }
        }
        removed
    }

    pub fn save(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let data = serde_json::to_string_pretty(&self.routines)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(&self.path, data)
    }

    /// Fire every enabled routine whose next run has passed. Returns the
    /// fired routines; run times are advanced and persisted.
    pub fn fire_due(&mut self, now_ms: u64) -> Vec<Routine> {
        let mut fired = Vec::new();
        for r in &mut self.routines {
            if !r.enabled {
                continue;
            }
            let due = r.next_run_ms.map(|n| n <= now_ms).unwrap_or(true);
            if !due {
                continue;
            }
            r.last_run_ms = Some(now_ms);
            r.next_run_ms = parse_schedule(&r.schedule)
                .ok()
                .map(|sched| next_run_after(sched, now_ms));
            fired.push(r.clone());
        }
        if !fired.is_empty() {
            if let Err(e) = self.save() {
                eprintln!("cron: failed to persist fired routines: {}", e);
            }
        }
        fired
    }
}

// ── Scheduler ───────────────────────────────────────────────────────────────

/// Spawn the scheduler task. The 60s loop is housekeeping cadence only —
/// each routine fires on its own schedule via `fire_due`, never on a
/// global 5s tick. Fired jobs publish `routine_run` to the CPU over the bus
/// (the heartbeat's job-pump picks them up from there).
pub fn spawn_scheduler(bus: Arc<Bus>, registry: Arc<RwLock<RoutineRegistry>>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
        ticker.tick().await; // skip the immediate first tick
        loop {
            ticker.tick().await;
            let now_ms = crate::utils::now_ms();
            let fired = { registry.write().await.fire_due(now_ms) };
            for r in fired {
                let msg = Message {
                    to: "cpu".to_string(),
                    from: "cron".to_string(),
                    data: serde_json::json!({
                        "type": "routine_run",
                        "routine_id": r.id,
                        "routine_name": r.name,
                        // Task 216: agent job payload (empty for legacy routines).
                        "agent_role": r.agent_role,
                        "task": r.task,
                        "backend": r.backend,
                    })
                    .to_string(),
                    timestamp: crate::utils::now_ms(),
                };
                match bus.publish(msg) {
                    Ok(_) => println!("cron: fired routine '{}' ({})", r.name, r.id),
                    Err(e) => eprintln!("cron: failed to publish routine_run for {}: {}", r.id, e),
                }
            }
        }
    });
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    fn tmp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("helix-cron-tests");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn parse_daily_ok() {
        assert_eq!(
            parse_schedule("daily 02:00").unwrap(),
            Schedule::Daily { hour: 2, min: 0 }
        );
        assert_eq!(
            parse_schedule("Daily 8:07").unwrap(),
            Schedule::Daily { hour: 8, min: 7 }
        );
    }

    #[test]
    fn default_routines_seed_consolidation_inside_nightly_window() {
        // Task 205.3: the consolidation job must be a NAMED cron registry job
        // firing inside hartbeat.md's 1–7am window (task 202 owns the
        // scheduling mechanics; this just pins the registration).
        let routines = default_routines();
        let consolidation = routines
            .iter()
            .find(|r| r.id == "nightly_memory_consolidation")
            .expect("nightly_memory_consolidation must be a seeded routine");
        assert!(consolidation.enabled);
        match parse_schedule(&consolidation.schedule).expect("schedule must parse") {
            Schedule::Daily { hour, .. } => {
                assert!(
                    (1..7).contains(&hour),
                    "consolidation must fire inside the 1–7am window, got {:02}:xx",
                    hour
                );
            }
            other => panic!("consolidation must be a daily schedule, got {:?}", other),
        }
    }

    #[test]
    fn routine_payload_round_trip() {
        // Task 216: a routine WITH an agent job payload survives save/load.
        let path = tmp_path("payload_rt.json");
        let _ = std::fs::remove_file(&path);
        let reg = RoutineRegistry {
            path: path.clone(),
            routines: vec![Routine {
                id: "mail_check".to_string(),
                name: "Inbox check".to_string(),
                schedule: "every 12h".to_string(),
                enabled: true,
                last_run_ms: None,
                next_run_ms: None,
                agent_role: "quartermaster".to_string(),
                task: "Check the inbox.".to_string(),
                backend: "auto".to_string(),
            }],
        };
        reg.save().expect("save");
        let loaded = RoutineRegistry::load_or_seed(path.clone());
        let r = loaded.list().iter().find(|r| r.id == "mail_check").expect("routine");
        assert!(r.has_agent_job());
        assert_eq!(r.agent_role, "quartermaster");
        assert_eq!(r.task, "Check the inbox.");
        assert_eq!(r.backend, "auto");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn legacy_routine_without_payload_still_loads() {
        // Task 216: pre-payload routine files (no new fields) deserialize fine.
        let path = tmp_path("legacy_compat.json");
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            r#"[{"id":"old","name":"Old","schedule":"daily 02:00","enabled":true}]"#,
        )
        .unwrap();
        let loaded = RoutineRegistry::load_or_seed(path.clone());
        let r = loaded.list().iter().find(|r| r.id == "old").expect("routine");
        assert!(!r.has_agent_job());
        assert_eq!(r.backend, "auto"); // serde default
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn cron_agent_job_end_to_end_via_stub() {
        // Task 218.3 (headless): an every-2m routine carrying an agent payload
        // fires, the payload reaches the shared dispatch helper, and a
        // commander report comes back — the full cron path minus a live LLM.
        let _tl = crate::tools::subagent_tool::test_env_lock();
        let path = tmp_path("e2e_cron.json");
        let _ = std::fs::remove_file(&path);
        let mut reg = RoutineRegistry::load_or_seed(path.clone());
        reg.routines.push(Routine {
            id: "e2e_drill".to_string(),
            name: "E2E drill".to_string(),
            schedule: "every 2m".to_string(),
            enabled: true,
            last_run_ms: None,
            next_run_ms: Some(crate::utils::now_ms() - 1_000), // due now
            agent_role: "e2e-role".to_string(),
            task: "trivial e2e task: reply ok".to_string(),
            backend: default_routine_backend(),
        });

        // The scheduler fires it...
        let fired = reg.fire_due(crate::utils::now_ms());
        assert_eq!(fired.len(), 1, "the due routine must fire");
        let r = &fired[0];
        assert!(r.has_agent_job(), "payload must survive the fire");

        // ...the CPU job-pump maps the payload to a SpawnSpec (same fields
        // handle_routine_run uses) and dispatches through the shared helper...
        let spec = crate::tools::subagent_tool::SpawnSpec {
            task: r.task.clone(),
            backend: r.backend.clone(),
            role_hint: r.agent_role.clone(),
            timeout_secs: 30,
        };
        // SAFETY: test holds the shared env lock.
        unsafe { std::env::set_var("HELIX_SUBAGENT_TEST_STUB", "1") };
        let report = crate::tools::subagent_tool::dispatch_spawn(&spec).await;
        unsafe { std::env::remove_var("HELIX_SUBAGENT_TEST_STUB") };

        // ...and the commander gets a report naming the task and role.
        assert!(report.contains("Sub-agent complete"), "got: {}", report);
        assert!(report.contains("trivial e2e task"), "got: {}", report);
        assert!(report.contains("e2e-role"), "got: {}", report);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn seeded_mail_check_example_exists() {
        // Task 216.4: the twice-daily mail-check template is seeded (disabled).
        let routines = default_routines();
        let mc = routines.iter().find(|r| r.id == "mail_check").expect("mail_check seed");
        assert!(mc.has_agent_job());
        assert!(!mc.enabled, "mail check ships disabled; John enables it");
        assert_eq!(mc.schedule, "every 12h");
    }

    #[test]
    fn parse_daily_rejects_bad() {
        assert!(parse_schedule("daily 25:00").is_err());
        assert!(parse_schedule("daily 02:60").is_err());
        assert!(parse_schedule("daily noon").is_err());
        assert!(parse_schedule("daily").is_err());
    }

    #[test]
    fn parse_every_ok() {
        assert_eq!(parse_schedule("every 6h").unwrap(), Schedule::Every { secs: 21600 });
        assert_eq!(parse_schedule("every 30m").unwrap(), Schedule::Every { secs: 1800 });
        assert_eq!(parse_schedule("every 90s").unwrap(), Schedule::Every { secs: 90 });
        assert_eq!(parse_schedule("every 2d").unwrap(), Schedule::Every { secs: 172800 });
    }

    #[test]
    fn parse_every_rejects_bad() {
        assert!(parse_schedule("every 0h").is_err());
        assert!(parse_schedule("every h").is_err());
        assert!(parse_schedule("every 5x").is_err());
        assert!(parse_schedule("sometimes").is_err());
    }

    #[test]
    fn display_matches_task_197_example() {
        // Task 197's contract shows schedule text like 'Every day at 8:07 AM'.
        let d = schedule_display(Schedule::Daily { hour: 8, min: 7 });
        assert_eq!(d, "Every day at 8:07 AM");
        assert_eq!(
            schedule_display(Schedule::Daily { hour: 14, min: 30 }),
            "Every day at 2:30 PM"
        );
        assert_eq!(
            schedule_display(Schedule::Every { secs: 21600 }),
            "Every 6 hours"
        );
        assert_eq!(
            schedule_display(Schedule::Every { secs: 60 }),
            "Every 1 minute"
        );
    }

    #[test]
    fn every_next_run_is_from_plus_interval() {
        let now = 1_700_000_000_000u64;
        assert_eq!(
            next_run_after(Schedule::Every { secs: 3600 }, now),
            now + 3_600_000
        );
    }

    #[test]
    fn daily_next_run_is_tomorrow_when_time_passed() {
        // 2026-10-05 09:00 local -> daily 02:00 must land on 2026-10-06 02:00.
        use chrono::{Local, TimeZone};
        let morning = Local
            .with_ymd_and_hms(2026, 10, 5, 9, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64;
        let next = next_run_after(Schedule::Daily { hour: 2, min: 0 }, morning);
        let expected = Local
            .with_ymd_and_hms(2026, 10, 6, 2, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64;
        assert_eq!(next, expected);
    }

    #[test]
    fn daily_next_run_is_today_when_time_ahead() {
        // 2026-10-05 01:00 local -> daily 02:00 lands today at 02:00.
        use chrono::{Local, TimeZone};
        let early = Local
            .with_ymd_and_hms(2026, 10, 5, 1, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64;
        let next = next_run_after(Schedule::Daily { hour: 2, min: 0 }, early);
        let expected = Local
            .with_ymd_and_hms(2026, 10, 5, 2, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64;
        assert_eq!(next, expected);
    }

    #[test]
    fn seed_creates_hartbeat_jobs() {
        // GIVEN no routines file:
        let path = tmp_path("seed-test.json");
        let _ = std::fs::remove_file(&path);

        // WHEN the registry loads:
        let reg = RoutineRegistry::load_or_seed(path.clone());

        // THEN John's hartbeat.md jobs are seeded, enabled, and scheduled:
        let ids: Vec<&str> = reg.list().iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"nightly_memory_consolidation"));
        assert!(ids.contains(&"error_log_review"));
        // Task 216: the mail_check example ships DISABLED (John enables it when
        // he wants the agent watching mail); everything else ships enabled.
        assert!(reg
            .list()
            .iter()
            .filter(|r| r.id != "mail_check")
            .all(|r| r.enabled));
        assert!(reg.list().iter().all(|r| r.next_run_ms.is_some()));
        assert!(path.exists());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn toggle_persists_across_reload() {
        // GIVEN a seeded registry:
        let path = tmp_path("toggle-test.json");
        let _ = std::fs::remove_file(&path);
        let mut reg = RoutineRegistry::load_or_seed(path.clone());

        // WHEN a routine is toggled off:
        let updated = reg
            .set_enabled("error_log_review", false)
            .expect("routine exists");
        assert!(!updated.enabled);

        // THEN a fresh load sees the persisted state:
        let reg2 = RoutineRegistry::load_or_seed(path.clone());
        let r = reg2
            .list()
            .iter()
            .find(|r| r.id == "error_log_review")
            .unwrap();
        assert!(!r.enabled);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn toggle_unknown_id_returns_none() {
        let path = tmp_path("toggle-unknown-test.json");
        let _ = std::fs::remove_file(&path);
        let mut reg = RoutineRegistry::load_or_seed(path.clone());
        assert!(reg.set_enabled("nope", true).is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn fire_due_fires_only_due_enabled() {
        // GIVEN one due routine, one future routine, one disabled-due routine:
        let path = tmp_path("fire-test.json");
        let _ = std::fs::remove_file(&path);
        let mut reg = RoutineRegistry::load_or_seed(path.clone());
        let now = crate::utils::now_ms();
        for r in reg.routines.iter_mut() {
            match r.id.as_str() {
                "nightly_memory_consolidation" => r.next_run_ms = Some(now - 1000),
                "error_log_review" => {
                    r.next_run_ms = Some(now - 1000);
                    r.enabled = false;
                }
                _ => {}
            }
        }
        // add a future routine
        reg.routines.push(Routine {
            id: "future".to_string(),
            name: "Future".to_string(),
            schedule: "every 1d".to_string(),
            enabled: true,
            last_run_ms: None,
            next_run_ms: Some(now + 3_600_000),
            agent_role: String::new(),
            task: String::new(),
            backend: default_routine_backend(),
        });

        // WHEN due jobs fire:
        let fired = reg.fire_due(now);

        // THEN only the due+enabled one fires, and its next run advanced:
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].id, "nightly_memory_consolidation");
        assert_eq!(fired[0].last_run_ms, Some(now));
        let r = reg
            .list()
            .iter()
            .find(|r| r.id == "nightly_memory_consolidation")
            .unwrap();
        assert!(r.next_run_ms.unwrap() > now);
        let _ = std::fs::remove_file(&path);
    }

    // ── Task 220: CRUD ───────────────────────────────────────────────

    fn empty_reg(name: &str) -> (RoutineRegistry, PathBuf) {
        let path = tmp_path(name);
        let _ = std::fs::remove_file(&path);
        let reg = RoutineRegistry {
            path: path.clone(),
            routines: Vec::new(),
        };
        (reg, path)
    }

    #[test]
    fn crud_add_ok_and_next_run_set() {
        let (mut reg, path) = empty_reg("crud_add.json");
        let r = reg
            .add("Inbox check", "every 12h", "quartermaster", "Check mail.", "auto", true)
            .expect("add");
        assert_eq!(r.id, "inbox_check");
        assert_eq!(r.backend, "auto");
        assert!(r.has_agent_job());
        assert!(r.next_run_ms.is_some(), "next run computed on add");
        // Persisted: reload finds it.
        let loaded = RoutineRegistry::load_or_seed(path.clone());
        assert!(loaded.list().iter().any(|x| x.id == "inbox_check"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn crud_add_rejects_bad_schedule_and_backend() {
        let (mut reg, path) = empty_reg("crud_add_bad.json");
        assert!(reg.add("Bad", "sometimes", "", "", "auto", true).is_err());
        assert!(reg.add("Bad", "every 6h", "", "", "watson", true).is_err());
        assert!(reg.add("", "every 6h", "", "", "auto", true).is_err());
        assert!(reg.list().is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn crud_add_unique_ids() {
        let (mut reg, path) = empty_reg("crud_add_ids.json");
        let a = reg.add("Nightly", "daily 02:00", "", "", "auto", true).unwrap();
        let b = reg.add("Nightly", "daily 03:00", "", "", "auto", true).unwrap();
        let c = reg.add("!!!", "daily 04:00", "", "", "auto", true).unwrap();
        assert_eq!(a.id, "nightly");
        assert_eq!(b.id, "nightly-2");
        assert_eq!(c.id, "routine");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn crud_update_patches_and_validates() {
        let (mut reg, path) = empty_reg("crud_update.json");
        let r = reg.add("Old name", "every 6h", "", "", "auto", true).unwrap();
        let u = reg
            .update(&r.id, Some("New name"), Some("daily 09:30"), None, Some("Do things."), Some("gemini"), Some(false))
            .expect("update");
        assert_eq!(u.name, "New name");
        assert_eq!(u.schedule, "daily 09:30");
        assert_eq!(u.task, "Do things.");
        assert_eq!(u.backend, "gemini");
        assert!(!u.enabled);
        // Partial update keeps untouched fields.
        let u2 = reg.update(&r.id, None, None, None, None, None, None).expect("noop update");
        assert_eq!(u2.name, "New name");
        // Bad schedule / unknown id rejected.
        assert!(reg.update(&r.id, None, Some("never"), None, None, None, None).is_err());
        assert!(reg.update("nope", Some("x"), None, None, None, None, None).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn crud_remove() {
        let (mut reg, path) = empty_reg("crud_remove.json");
        let r = reg.add("Temp", "every 1h", "", "", "auto", true).unwrap();
        assert!(reg.remove(&r.id));
        assert!(!reg.remove(&r.id));
        assert!(reg.list().is_empty());
        let _ = std::fs::remove_file(&path);
    }
}
