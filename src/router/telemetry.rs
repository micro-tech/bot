// router/telemetry.rs - System Load & Telemetry (task 147)
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct TelemetrySnapshot {
    pub gpu_util: f32,
    pub vram_used: f32,
    pub cpu_load: f32,
    pub temp: f32,
    pub rtt_ms: std::collections::HashMap<String, f32>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

pub struct TelemetryCollector {
    latest: Arc<RwLock<TelemetrySnapshot>>,
}

impl TelemetryCollector {
    pub fn new() -> Self {
        Self {
            latest: Arc::new(RwLock::new(TelemetrySnapshot {
                gpu_util: 0.0,
                vram_used: 0.0,
                cpu_load: 0.0,
                temp: 0.0,
                rtt_ms: Default::default(),
                timestamp: chrono::Utc::now(),
            })),
        }
    }

    pub async fn get_latest(&self) -> TelemetrySnapshot {
        self.latest.read().await.clone()
    }

    // TODO: background sampling loop + LAN RTT pings
}
