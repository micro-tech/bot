//! OKF API Endpoints (Task 164.5)
//!
//! Axum handlers for the OKF Librarian system.
//! These are mounted under the main web server.

use axum::{
    extract::{Json, State},
    response::IntoResponse,
};
use serde_json::json;

use crate::config::okf::OkfConfig;
use crate::okf::{OkfLibrarian, OkfRegistry};

/// Shared state for OKF routes.
/// We use a Mutex because loading/reloading mutates the librarian.
pub type OkfState = std::sync::Arc<tokio::sync::Mutex<OkfLibrarian>>;

/// GET /okf/status
/// Returns current OKF status and registry summary.
pub async fn okf_status(State(state): State<OkfState>) -> impl IntoResponse {
    let librarian = state.lock().await;
    Json(librarian.summary())
}

/// GET /okf/registry
/// Returns the full current registry (tools, knowledge, schemas).
pub async fn okf_registry(State(state): State<OkfState>) -> impl IntoResponse {
    let librarian = state.lock().await;
    Json(json!({
        "bundle_id": librarian.registry.bundle_id,
        "version": librarian.registry.bundle_version,
        "tool_count": librarian.registry.tool_count(),
        "knowledge_count": librarian.registry.knowledge_count(),
        "tools": librarian.registry.tools,
        "knowledge": librarian.registry.knowledge,
        "schemas": librarian.registry.schemas,
    }))
}

/// POST /okf/reload
/// Forces a reload from the remote server (if enabled). Task 165.
pub async fn okf_reload(State(state): State<OkfState>) -> impl IntoResponse {
    let mut librarian = state.lock().await;

    if !librarian.is_enabled() {
        return Json(json!({
            "status": "disabled",
            "msg": "OKF is disabled in config. Set [helix.okf] enabled = true"
        }));
    }

    match librarian.hot_reload_from_remote().await {
        Ok(validation) => Json(json!({
            "status": "success",
            "validation": {
                "is_valid": validation.is_valid,
                "errors": validation.errors,
                "warnings": validation.warnings,
            },
            "reload_info": librarian.last_reload_info(),
            "registry": librarian.registry.to_index_summary(),
        })),
        Err(e) => Json(json!({
            "status": "error",
            "msg": e
        })),
    }
}

/// POST /okf/reload/force
/// Forces a reload while ignoring any cached ETag (Task 165).
pub async fn okf_force_reload(State(state): State<OkfState>) -> impl IntoResponse {
    let mut librarian = state.lock().await;

    if !librarian.is_enabled() {
        return Json(json!({
            "status": "disabled",
            "msg": "OKF is disabled"
        }));
    }

    match librarian.force_reload_from_remote().await {
        Ok(validation) => Json(json!({
            "status": "success",
            "forced": true,
            "validation": {
                "is_valid": validation.is_valid,
                "errors": validation.errors,
                "warnings": validation.warnings,
            },
            "reload_info": librarian.last_reload_info(),
        })),
        Err(e) => Json(json!({
            "status": "error",
            "msg": e
        })),
    }
}

/// POST /okf/manifest
/// Upload / push a manifest JSON directly (useful for testing and local bundles).
pub async fn okf_push_manifest(
    State(state): State<OkfState>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let mut librarian = state.lock().await;

    let json_str = serde_json::to_string(&payload).unwrap_or_default();

    match librarian.load_manifest_from_json(&json_str) {
        Ok(validation) => Json(json!({
            "status": if validation.is_valid { "success" } else { "invalid" },
            "validation": {
                "is_valid": validation.is_valid,
                "errors": validation.errors,
                "warnings": validation.warnings,
            },
            "registry": librarian.registry.to_index_summary(),
        })),
        Err(e) => Json(json!({
            "status": "error",
            "msg": e
        })),
    }
}

/// POST /okf/webhook
/// Receives change notifications from the remote OKF server.
/// Expects a small JSON payload like: { "bundle_id": "...", "action": "updated" }
pub async fn okf_webhook(
    State(state): State<OkfState>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let bundle_id = payload["bundle_id"].as_str().unwrap_or("unknown").to_string();
    let action = payload["action"].as_str().unwrap_or("changed").to_string();

    println!("[OKF Webhook] Received notification for bundle '{}' - action: {}", bundle_id, action);

    // For now, if auto_reload is enabled, trigger a reload.
    // In a real implementation we would check etag/version first.
    let mut librarian = state.lock().await;

    if librarian.config.auto_reload() {
        match librarian.load_from_remote().await {
            Ok(v) if v.is_valid => {
                println!("[OKF] Auto-reloaded bundle after webhook");
                Json(json!({
                    "status": "reloaded",
                    "bundle_id": bundle_id,
                    "action": action,
                }))
            }
            Ok(v) => Json(json!({
                "status": "reload_failed_validation",
                "errors": v.errors,
            })),
            Err(e) => Json(json!({
                "status": "reload_error",
                "error": e,
            })),
        }
    } else {
        Json(json!({
            "status": "notification_received",
            "auto_reload": false,
            "msg": "Use /okf/reload to apply changes manually"
        }))
    }
}

/// Simple health endpoint for the OKF system.
pub async fn okf_health(State(state): State<OkfState>) -> impl IntoResponse {
    let librarian = state.lock().await;
    Json(json!({
        "okf_enabled": librarian.is_enabled(),
        "bundle_loaded": librarian.registry.bundle_id.is_some(),
        "tool_count": librarian.registry.tool_count(),
    }))
}

/// GET /okf/schema
/// Serves the official OKF Bundle Schema v1.0 (Task 167).
/// Returns the JSON Schema that defines valid bundles.
pub async fn okf_schema() -> impl IntoResponse {
    // Serve the schema file we keep at the project root
    match std::fs::read_to_string("okf_bundle_schema.json") {
        Ok(schema_text) => {
            // Try to return as parsed JSON so Content-Type is application/json
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&schema_text) {
                Json(parsed).into_response()
            } else {
                // Fallback to raw text
                axum::response::Response::builder()
                    .header("content-type", "application/json")
                    .body(schema_text)
                    .unwrap()
                    .into_response()
            }
        }
        Err(_) => Json(json!({
            "error": "Schema file not found",
            "hint": "okf_bundle_schema.json should be present in the project root"
        })).into_response()
    }
}
