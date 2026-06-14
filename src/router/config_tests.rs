// router/config_tests.rs - Tests for Task 145.8

use crate::router::config::{RouterConfig, validate_config, apply_env_overrides};
use std::env;

#[test]
fn test_validate_valid_config() {
    let cfg = RouterConfig::default();
    assert!(validate_config(&cfg).is_ok());
}

#[test]
fn test_validate_negative_weight() {
    let mut cfg = RouterConfig::default();
    cfg.complexity.token_weight = -0.1;
    let result = validate_config(&cfg);
    assert!(result.is_err());
    assert!(result.unwrap_err().iter().any(|e| e.contains("token_weight")));
}

#[test]
fn test_validate_invalid_threshold() {
    let mut cfg = RouterConfig::default();
    cfg.complexity.global_threshold = 1.5;
    let result = validate_config(&cfg);
    assert!(result.is_err());
}

#[test]
fn test_env_override_gpu_max() {
    env::set_var("LLM_ROUTER_LOAD_GPU_MAX", "92.5");
    let mut cfg = RouterConfig::default();
    apply_env_overrides(&mut cfg);
    assert_eq!(cfg.load_thresholds.gpu_max, 92.5);
    env::remove_var("LLM_ROUTER_LOAD_GPU_MAX");
}

#[test]
fn test_env_override_hot_reload() {
    env::set_var("LLM_ROUTER_HOT_RELOAD", "false");
    let mut cfg = RouterConfig::default();
    apply_env_overrides(&mut cfg);
    assert!(!cfg.hot_reload);
    env::remove_var("LLM_ROUTER_HOT_RELOAD");
}