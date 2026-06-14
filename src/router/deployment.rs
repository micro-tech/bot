// router/deployment.rs - Deployment & Configuration Guide (Task 140)

/// # Production Deployment Guide
///
/// ## 1. Configuration
/// Place `router.toml` or `router.json` in your config directory.
///
/// Example minimal production config:
/// ```toml
/// hot_reload = true
///
/// [complexity]
/// global_threshold = 0.65
///
/// [load_thresholds]
/// gpu_max = 85.0
/// vram_max = 90.0
///
/// [health_thresholds]
/// error_rate_max = 0.05
/// latency_max_ms = 2000
/// ```
///
/// ## 2. Systemd Service
/// ```ini
/// [Unit]
/// Description=Grok Bot with LLM Router
/// After=network.target
///
/// [Service]
/// ExecStart=/usr/local/bin/bot --config /etc/bot/router.toml
/// Restart=always
/// User=bot
/// Group=botctl
///
/// [Install]
/// WantedBy=multi-user.target
/// ```
///
/// ## 3. Health Checks
/// The router exposes health via `HealthStore`. Integrate with your
/// monitoring system (Prometheus, Datadog, etc.) using the metrics
/// emitted in `observability.rs`.
///
/// ## 4. Recommended Production Settings
/// - Enable `hot_reload = true`
/// - Set conservative load thresholds
/// - Use persistent overrides only for trusted users
/// - Monitor `RouterMetrics` for latency drift
pub mod deployment {}
