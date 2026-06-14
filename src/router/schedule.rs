// router/schedule.rs - Time-of-Day & Schedule Logic (task 146)
use chrono::{DateTime, Timelike, Utc};

#[derive(Debug, Clone)]
pub struct ScheduleWindow {
    pub start_hour: u32,
    pub end_hour: u32,
    pub profile: String,
}

#[derive(Debug, Clone)]
pub struct ScheduleConfig {
    pub windows: Vec<ScheduleWindow>,
}

pub fn evaluate(timestamp: DateTime<Utc>, config: &ScheduleConfig) -> String {
    let hour = timestamp.hour();
    for w in &config.windows {
        if hour >= w.start_hour && hour < w.end_hour {
            return w.profile.clone();
        }
    }
    "default".to_string()
}
