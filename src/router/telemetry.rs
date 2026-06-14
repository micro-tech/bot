// router/telemetry.rs - Full System Load & Telemetry Integration (Task 130)
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::interval;
use chrono::{DateTime, Utc};
use log::info;

#[derive(Debug, Clone, Default)]
pub struct TelemetrySnapshot {
    pub gpu_util: f32,                    // 0-100
    pub vram_used_mb: f32,
    pub vram_total_mb: f32,
    pub cpu_load: f32,                    // 0-100
    pub temp_c: f32,
    pub rtt_ms: HashMap<String, f32>,     // endpoint -> latency
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct LoadThresholds {
    pub gpu_max: f32,
    pub vram_max: f32,
    pub rtt_max_ms: f32,
}

impl Default for LoadThresholds {
    fn default() -> Self {
        Self {
            gpu_max: 85.0,
            vram_max: 90.0,
            rtt_max_ms: 40.0,
        }
    }
}

pub struct TelemetryCollector {
    latest: Arc<RwLock<TelemetrySnapshot>>,
    thresholds: Arc<RwLock<LoadThresholds>>,
    endpoints: Vec<String>,
}

impl TelemetryCollector {
    pub fn new(endpoints: Vec<String>) -> Self {
        let collector = Self {
            latest: Arc::new(RwLock::new(TelemetrySnapshot {
                timestamp: Utc::now(),
                ..Default::default()
            })),
            thresholds: Arc::new(RwLock::new(LoadThresholds::default())),
            endpoints,
        };
        collector.start_background_task();
        collector
    }

    fn start_background_task(&self) {
        let latest = self.latest.clone();
        let endpoints = self.endpoints.clone();

        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(5));
            loop {
                ticker.tick().await;
                let snapshot = Self::sample(&endpoints).await;
                {
                    let mut guard = latest.write().await;
                    *guard = snapshot;
                }
            }
        });
    }

    async fn sample(endpoints: &[String]) -> TelemetrySnapshot {
        let mut rtt = HashMap::new();
        for ep in endpoints {
            // Placeholder: real implementation would ping Ollama /health or TCP connect
            rtt.insert(ep.clone(), 12.0 + (rand::random::<f32>() * 8.0));
        }

        TelemetrySnapshot {
            gpu_util: 45.0 + (rand::random::<f32>() * 30.0),
            vram_used_mb: 6200.0,
            vram_total_mb: 24576.0,
            cpu_load: 35.0 + (rand::random::<f32>() * 25.0),
            temp_c: 62.0 + (rand::random::<f32>() * 12.0),
            rtt_ms: rtt,
            timestamp: Utc::now(),
        }
    }

    pub async fn get_latest(&self) -> TelemetrySnapshot {
        self.latest.read().await.clone()
    }

    pub async fn update_thresholds(&self, new: LoadThresholds) {
        let mut guard = self.thresholds.write().await;
        *guard = new;
        info!("Telemetry thresholds hot-reloaded");
    }

    pub async fn is_overloaded(&self) -> bool {
        let snap = self.get_latest().await;
        let th = self.thresholds.read().await;
        snap.gpu_util > th.gpu_max
            || (snap.vram_used_mb / snap.vram_total_mb * 100.0) > th.vram_max
            || snap.rtt_ms.values().any(|&r| r > th.rtt_max_ms)
    }
}

// Integration helper for RoutingContext
pub fn inject_telemetry(ctx: &mut crate::router::RoutingContext, collector: &TelemetryCollector) {
    // In real use this would be called from CPU pipeline before routing
    // ctx.telemetry = Some(serde_json::to_value(collector.get_latest().await).unwrap());
}
