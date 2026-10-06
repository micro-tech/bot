//! OKF API Endpoints (Task 164.5)
//!
//! Axum handlers for the OKF Librarian system.
//! These are mounted under the main web server.

use axum::{
    extract::{Json, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde_json::json;

use crate::okf::OkfLibrarian;

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
        "status": "ok",
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

// ── OKF Protocol v1: read endpoints the fetcher expects ─────────────────────
// These reconcile the server half with src/okf/fetcher.rs (and grok-cli):
// the fetcher asks for manifest.json / bundles/{id}/manifest.json /
// knowledge/{kid}, which the server never mounted until now.

/// Compute a weak ETag for a response body (v1 conditional GETs).
fn etag_for(bytes: &[u8]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("\"{:x}\"", hasher.finish())
}

/// True when the request's If-None-Match matches our ETag.
fn etag_matches(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(axum::http::header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag || t.trim() == "*"))
}

/// Serve a JSON body with ETag / 304 handling. Shared by the manifest GETs.
fn json_with_etag(body: String, headers: &HeaderMap) -> axum::response::Response {
    let etag = etag_for(body.as_bytes());
    if etag_matches(headers, &etag) {
        return (StatusCode::NOT_MODIFIED, "").into_response();
    }
    (
        [
            (axum::http::header::ETAG, etag),
            (
                axum::http::header::CONTENT_TYPE,
                "application/json".to_string(),
            ),
        ],
        body,
    )
        .into_response()
}

/// GET /okf/manifest.json
/// Serves the currently loaded bundle manifest (v1). 404 when none loaded.
pub async fn okf_manifest_json(
    State(state): State<OkfState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let librarian = state.lock().await;
    match &librarian.current_manifest {
        Some(manifest) => {
            let body = serde_json::to_string(manifest).unwrap_or_default();
            json_with_etag(body, &headers)
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "no bundle loaded"})),
        )
            .into_response(),
    }
}

/// GET /okf/bundles/{id}/manifest.json
/// Serves the manifest when {id} matches the loaded bundle (v1).
pub async fn okf_bundle_manifest(
    State(state): State<OkfState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let librarian = state.lock().await;
    match &librarian.current_manifest {
        Some(manifest) if manifest.id == id => {
            let body = serde_json::to_string(manifest).unwrap_or_default();
            json_with_etag(body, &headers)
        }
        _ => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("unknown bundle '{}'", id)})),
        )
            .into_response(),
    }
}

/// GET /okf/knowledge/{kid}
/// Serves a single knowledge document by id (v1). 404 when unknown.
pub async fn okf_knowledge_doc(
    State(state): State<OkfState>,
    Path(kid): Path<String>,
) -> impl IntoResponse {
    let librarian = state.lock().await;
    match librarian.registry.get_knowledge(&kid) {
        Some(entry) => {
            let content_type = if entry.content_type.is_empty() {
                "text/plain".to_string()
            } else {
                entry.content_type.clone()
            };
            (
                [(axum::http::header::CONTENT_TYPE, content_type)],
                entry.content.clone().unwrap_or_default(),
            )
                .into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("unknown knowledge id '{}'", kid)})),
        )
            .into_response(),
    }
}

// ── OKF Protocol v1: write endpoints (grok-cli targets) ─────────────────────

/// POST /okf/traces → 202
/// Appends a workflow trace as one JSONL line to the configured trace file.
/// v1 canonical path; replaces grok-cli's legacy /api/traces.
pub async fn okf_traces(
    State(state): State<OkfState>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    use tokio::io::AsyncWriteExt;

    let trace_file = { state.lock().await.config.trace_file() };
    let mut line = serde_json::to_string(&payload).unwrap_or_default();
    line.push('\n');

    let open = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&trace_file)
        .await;
    match open {
        Ok(mut f) => {
            // NOTE: tokio::fs::File pipelines writes — write_all() returning
            // Ok only means the bytes were accepted into the pipeline, not
            // that they reached the OS. flush() waits for the pipelined
            // write to finish, so the 202 is only sent once the JSONL line
            // is actually visible to readers.
            let io_result = match f.write_all(line.as_bytes()).await {
                Ok(_) => f.flush().await,
                Err(e) => Err(e),
            };
            match io_result {
                Ok(_) => (
                    StatusCode::ACCEPTED,
                    Json(json!({"status": "accepted"})),
                )
                    .into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": e.to_string()})),
                )
                    .into_response(),
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// Incoming concept shape (grok-cli OkfConcept JSON, v1).
#[derive(serde::Deserialize)]
pub struct IncomingConcept {
    id: String,
    #[serde(default, rename = "type")]
    concept_type: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    resource: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    body: String,
}

/// POST /okf/bundles/{bundle}/concepts → 201
/// Appends a knowledge concept to the bundle's registry (v1).
/// v1 canonical path; replaces grok-cli's legacy /bundles/{b}/concepts.
pub async fn okf_push_concept(
    State(state): State<OkfState>,
    Path(bundle): Path<String>,
    Json(concept): Json<IncomingConcept>,
) -> impl IntoResponse {
    if concept.id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "concept id is required"})),
        )
            .into_response();
    }

    let mut librarian = state.lock().await;

    if let Some(loaded_id) = &librarian.registry.bundle_id {
        if *loaded_id != bundle {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("unknown bundle '{}'", bundle)})),
            )
                .into_response();
        }
    }

    let mut metadata = std::collections::HashMap::new();
    if !concept.concept_type.is_empty() {
        metadata.insert(
            "type".to_string(),
            serde_json::Value::String(concept.concept_type),
        );
    }
    if !concept.description.is_empty() {
        metadata.insert(
            "description".to_string(),
            serde_json::Value::String(concept.description),
        );
    }
    if !concept.tags.is_empty() {
        metadata.insert(
            "tags".to_string(),
            serde_json::Value::Array(
                concept.tags.into_iter().map(serde_json::Value::String).collect(),
            ),
        );
    }
    if let Some(ts) = concept.timestamp {
        metadata.insert("timestamp".to_string(), serde_json::Value::String(ts));
    }

    let title = if concept.title.is_empty() {
        concept.id.clone()
    } else {
        concept.title
    };
    let entry = crate::okf::manifest::OkfKnowledgeEntry {
        id: concept.id.clone(),
        title,
        content_type: "text/markdown".to_string(),
        content: Some(concept.body),
        url: concept.resource,
        version: None,
        metadata,
    };
    librarian.registry.knowledge.insert(entry.id.clone(), entry);
    // Persist so the concept survives restarts (best-effort, like manifest loads).
    let _ = librarian
        .registry
        .write_to_file(&librarian.config.index_file());

    (
        StatusCode::CREATED,
        Json(json!({"status": "created", "id": concept.id})),
    )
        .into_response()
}
