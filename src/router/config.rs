// router/config.rs - Full Configuration System & Hot-Reloading (Task 132 + 145)
// Enhanced with validation, environment variable overrides, and better error handling.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use log::{info, warn, error};
use notify::{Watcher, RecursiveMode, watcher, DebouncedEvent};
use std::time::Duration;
use std::env;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RouterConfig {
    pub complexity: ComplexityConfig,
    pub schedule: ScheduleConfig,
    pub load_thresholds: LoadThresholds,
    pub health_thresholds: HealthThresholds,
    pub fallback_chain: Vec<String>,
    pub profiles: HashMap<String, RoutingProfile>,
    pub hot_reload: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ComplexityConfig {
    pub token_weight: f32,
    pub code_weight: f32,
    pub reasoning_weight: f32,
    pub math_weight: f32,
    pub multi_step_weight: f32,
    pub structural_weight: f32,
    pub global_threshold: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScheduleConfig {
    pub windows: Vec<TimeWindow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TimeWindow {
    pub start_hour: u32,
    pub end_hour: u32,
    pub profile: String,
    pub days: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LoadThresholds {
    pub gpu_max: f32,
    pub vram_max: f32,
    pub rtt_max_ms: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HealthThresholds {
    pub error_rate_max: f32,
    pub latency_max_ms: u32,
    pub unreachable_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoutingProfile {
    pub complexity_threshold: f32,
    pub preferred_backends: Vec<String>,
    pub fallback_chain: Vec<String>,
}

/// Thread-safe configuration holder with hot-reload support
pub struct ConfigManager {
    config: Arc<RwLock<RouterConfig>>,
    path: PathBuf,
}

impl ConfigManager {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        let path = path.as_ref().to_path_buf();
        let config = Self::load_from_file(&path).unwrap_or_default();

        let manager = Self {
            config: Arc::new(RwLock::new(config)),
            path,
        };

        if manager.config.try_read().map(|c| c.hot_reload).unwrap_or(false) {
            manager.start_watcher();
        }

        manager
    }

    fn load_from_file(path: &Path) -> Option<RouterConfig> {
        let content = std::fs::read_to_string(path).ok()?;
        if path.extension().map_or(false, |e| e == "toml") {
            toml::from_str(&content).ok()
        } else {
            serde_json::from_str(&content).ok()
        }
    }

    fn start_watcher(&self) {
        let path = self.path.clone();
        let config_arc = self.config.clone();

        std::thread::spawn(move || {
            let (tx, rx) = std::sync::mpsc::channel();
            let mut watcher = watcher(tx, Duration::from_secs(2)).unwrap();
            watcher.watch(&path, RecursiveMode::NonRecursive).ok();

            for event in rx {
                if let DebouncedEvent::Write(_) = event {
                    if let Some(new_cfg) = Self::load_from_file(&path) {
                        let mut guard = config_arc.blocking_write();
                        *guard = new_cfg;
                        info!("Router config hot-reloaded from {:?}", path);
                    } else {
                        warn!("Failed to reload config from {:?}", path);
                    }
                }
            }
        });
    }

    pub async fn get(&self) -> RouterConfig {
        self.config.read().await.clone()
    }

    pub async fn reload(&self) -> Result<(), String> {
        if let Some(new_cfg) = Self::load_from_file(&self.path) {
            let mut guard = self.config.write().await;
            *guard = new_cfg;
            info!("Config manually reloaded");
            Ok(())
        } else {
            Err("Failed to reload config".into())
        }
    }
}

impl Default for RouterConfig {
    fn default() -> Self {
        let mut profiles = HashMap::new();
        profiles.insert("default".into(), RoutingProfile {
            complexity_threshold: 0.7,
            preferred_backends: vec!["gemini".into()],
            fallback_chain: vec!["ollama".into()],
        });

        Self {
            complexity: ComplexityConfig {
                token_weight: 0.2,
                code_weight: 0.25,
                reasoning_weight: 0.2,
                math_weight: 0.15,
                multi_step_weight: 0.1,
                structural_weight: 0.1,
                global_threshold: 0.65,
            },
            schedule: ScheduleConfig { windows: vec![] },
            load_thresholds: LoadThresholds {
                gpu_max: 85.0,
                vram_max: 90.0,
                rtt_max_ms: 40.0,
            },
            health_thresholds: HealthThresholds {
                error_rate_max: 0.05,
                latency_max_ms: 2000,
                unreachable_seconds: 30,
            },
            fallback_chain: vec!["ollama".into(), "gemini".into(), "fallback".into()],
            profiles,
            hot_reload: true,
        }
    }
}

/// === Task 145 Enhancements ===

/// Validate a RouterConfig and return detailed errors if invalid.
pub fn validate_config(cfg: &RouterConfig) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    // Complexity weights must be non-negative
    if cfg.complexity.token_weight < 0.0 { errors.push("token_weight must be >= 0".into()); }
    if cfg.complexity.code_weight < 0.0 { errors.push("code_weight must be >= 0".into()); }
    if cfg.complexity.reasoning_weight < 0.0 { errors.push("reasoning_weight must be >= 0".into()); }
    if cfg.complexity.math_weight < 0.0 { errors.push("math_weight must be >= 0".into()); }
    if cfg.complexity.multi_step_weight < 0.0 { errors.push("multi_step_weight must be >= 0".into()); }
    if cfg.complexity.structural_weight < 0.0 { errors.push("structural_weight must be >= 0".into()); }

    // Global threshold must be between 0 and 1
    if cfg.complexity.global_threshold < 0.0 || cfg.complexity.global_threshold > 1.0 {
        errors.push("global_threshold must be between 0.0 and 1.0".into());
    }

    // Load thresholds sanity checks
    if cfg.load_thresholds.gpu_max < 0.0 || cfg.load_thresholds.gpu_max > 100.0 {
        errors.push("gpu_max must be between 0 and 100".into());
    }
    if cfg.load_thresholds.vram_max < 0.0 || cfg.load_thresholds.vram_max > 100.0 {
        errors.push("vram_max must be between 0 and 100".into());
    }

    // Health thresholds
    if cfg.health_thresholds.error_rate_max < 0.0 || cfg.health_thresholds.error_rate_max > 1.0 {
        errors.push("error_rate_max must be between 0.0 and 1.0".into());
    }

    // Fallback chain must not be empty
    if cfg.fallback_chain.is_empty() {
        errors.push("fallback_chain cannot be empty".into());
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Apply environment variable overrides (LLM_ROUTER_*)
pub fn apply_env_overrides(cfg: &mut RouterConfig) {
    if let Ok(val) = env::var("LLM_ROUTER_COMPLEXITY_GLOBAL_THRESHOLD") {
        if let Ok(v) = val.parse::<f32>() {
            cfg.complexity.global_threshold = v;
        }
    }
    if let Ok(val) = env::var("LLM_ROUTER_LOAD_GPU_MAX") {
        if let Ok(v) = val.parse::<f32>() {
            cfg.load_thresholds.gpu_max = v;
        }
    }
    if let Ok(val) = env::var("LLM_ROUTER_LOAD_VRAM_MAX") {
        if let Ok(v) = val.parse::<f32>() {
            cfg.load_thresholds.vram_max = v;
        }
    }
    if let Ok(val) = env::var("LLM_ROUTER_HOT_RELOAD") {
        cfg.hot_reload = val.to_lowercase() == "true";
    }
}