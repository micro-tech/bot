// router/schedule.rs - Full Time-of-Day & Schedule Logic (Task 129)
use chrono::{DateTime, Timelike, Utc, Weekday};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ScheduleWindow {
    pub start_hour: u32,      // 0-23
    pub end_hour: u32,        // 0-23 (exclusive)
    pub days: Option<Vec<Weekday>>, // None = every day
    pub profile: String,
}

#[derive(Debug, Clone)]
pub struct RoutingProfile {
    pub name: String,
    pub complexity_threshold: f32,
    pub preferred_backends: Vec<String>,
    pub fallback_chain: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ScheduleConfig {
    pub windows: Vec<ScheduleWindow>,
    pub profiles: HashMap<String, RoutingProfile>,
}

impl ScheduleConfig {
    pub fn default() -> Self {
        let mut profiles = HashMap::new();
        profiles.insert("default".into(), RoutingProfile {
            name: "default".into(),
            complexity_threshold: 0.7,
            preferred_backends: vec!["gemini".into()],
            fallback_chain: vec!["ollama".into(), "fallback".into()],
        });
        profiles.insert("night".into(), RoutingProfile {
            name: "night".into(),
            complexity_threshold: 0.4,
            preferred_backends: vec!["ollama".into()],
            fallback_chain: vec!["gemini".into()],
        });
        Self { windows: vec![], profiles }
    }
}

/// Pure deterministic evaluator — returns the active profile name
pub fn evaluate(timestamp: DateTime<Utc>, config: &ScheduleConfig) -> String {
    let hour = timestamp.hour();
    let weekday = timestamp.weekday();

    for window in &config.windows {
        let hour_match = hour >= window.start_hour && hour < window.end_hour;
        let day_match = window.days.as_ref()
            .map(|days| days.contains(&weekday))
            .unwrap_or(true);

        if hour_match && day_match {
            return window.profile.clone();
        }
    }
    "default".to_string()
}

/// Returns the full RoutingProfile for the active window
pub fn active_profile(timestamp: DateTime<Utc>, config: &ScheduleConfig) -> RoutingProfile {
    let name = evaluate(timestamp, config);
    config.profiles.get(&name)
        .cloned()
        .unwrap_or_else(|| config.profiles.get("default").cloned().unwrap())
}
