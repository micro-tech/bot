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
        },
        Routine {
            id: "error_log_review".to_string(),
            name: "Error log review".to_string(),
            schedule: "daily 06:30".to_string(),
            enabled: true,
            last_run_ms: None,
            next_run_ms: None,
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
        assert!(reg.list().iter().all(|r| r.enabled));
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
}
