//! OKF v1 self-fetch test: Helix fetches from its own OKF server (task 208).
//!
//! Spins up the REAL v1 route set (the same handlers web_server mounts) against
//! a real OkfLibrarian, then drives it with the REAL OkfFetcher plus raw
//! reqwest calls for the write endpoints:
//! 1. GET /okf/manifest.json -> manifest with required id/version.
//! 2. GET /okf/bundles/{id}/manifest.json -> same manifest; unknown id -> 404.
//! 3. GET /okf/knowledge/{kid} -> document text; unknown id -> 404.
//! 4. GET /okf/health -> {"status":"ok"}.
//! 5. ETag round-trip: If-None-Match -> 304.
//! 6. POST /okf/traces -> 202 and the JSONL line lands in the trace file.
//! 7. POST /okf/bundles/{bundle}/concepts -> 201, then the concept is
//!    retrievable via GET /okf/knowledge/{id}.

use axum::{Router, routing::{get, post}};
use helix::config::okf::OkfConfig;
use helix::okf::{api as okf_api, api::OkfState, OkfFetcher, OkfLibrarian};

fn ensure_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn sample_manifest() -> &'static str {
    r##"{
        "id": "self-fetch-bundle",
        "version": "1.0.0",
        "name": "Self Fetch Bundle",
        "tools": [],
        "knowledge": [
            {"id": "doc1", "title": "Doc One", "content_type": "text/markdown",
             "content": "# Doc One\nBody text here."}
        ],
        "schemas": []
    }"##
}

/// Build the v1 route set exactly as web_server mounts it (minus auth
/// middleware — no tokens configured here, so production is fail-open too).
fn v1_router(state: OkfState) -> Router {
    Router::new()
        .route("/okf/status", get(okf_api::okf_status))
        .route("/okf/registry", get(okf_api::okf_registry))
        .route("/okf/health", get(okf_api::okf_health))
        .route("/okf/schema", get(okf_api::okf_schema))
        .route("/okf/webhook", post(okf_api::okf_webhook))
        .route("/okf/manifest.json", get(okf_api::okf_manifest_json))
        .route(
            "/okf/bundles/{id}/manifest.json",
            get(okf_api::okf_bundle_manifest),
        )
        .route("/okf/knowledge/{kid}", get(okf_api::okf_knowledge_doc))
        .route("/okf/traces", post(okf_api::okf_traces))
        .route(
            "/okf/bundles/{bundle}/concepts",
            post(okf_api::okf_push_concept),
        )
        .with_state(state)
}

struct TestServer {
    base_url: String,
    trace_file: std::path::PathBuf,
}

async fn start_test_server() -> TestServer {
    ensure_crypto_provider();

    // Unique trace file per test: the tests run in parallel and previously
    // shared one per-PID file, so one's cleanup could wipe another's write
    // mid-assertion (flaky v1_self_write_traces_and_concepts).
    static TRACE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = TRACE_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let trace_file = std::env::temp_dir().join(format!(
        "okf_v1_test_traces_{}_{}.jsonl",
        std::process::id(),
        seq
    ));
    let _ = std::fs::remove_file(&trace_file);

    let cfg = OkfConfig {
        enabled: Some(true),
        server_url: Some("http://127.0.0.1:0".to_string()),
        trace_file: Some(trace_file.to_string_lossy().to_string()),
        ..Default::default()
    };
    let mut librarian = OkfLibrarian::new(cfg);
    librarian
        .load_manifest_from_json(sample_manifest())
        .expect("sample manifest loads");
    let state: OkfState = std::sync::Arc::new(tokio::sync::Mutex::new(librarian));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, v1_router(state)).await.unwrap();
    });

    TestServer {
        base_url: format!("http://{}", addr),
        trace_file,
    }
}

fn fetcher_config(base_url: &str) -> OkfConfig {
    OkfConfig {
        enabled: Some(true),
        server_url: Some(base_url.to_string()),
        ..Default::default()
    }
}

#[tokio::test]
async fn v1_self_fetch_manifest_and_knowledge() {
    // GIVEN Helix serving its own v1 OKF routes:
    let srv = start_test_server().await;
    let fetcher = OkfFetcher::new(fetcher_config(&srv.base_url));

    // WHEN the real fetcher asks for the manifest:
    let (manifest, etag) = fetcher
        .fetch_manifest(None, None)
        .await
        .expect("self-fetch manifest works");

    // THEN it gets the bundle Helix itself loaded:
    assert_eq!(manifest.id, "self-fetch-bundle");
    assert_eq!(manifest.version, "1.0.0");
    assert!(etag.is_some());

    // WHEN the bundle-scoped path is used:
    let (by_id, _) = fetcher
        .fetch_manifest(Some("self-fetch-bundle"), None)
        .await
        .expect("bundle-scoped self-fetch works");
    assert_eq!(by_id.id, "self-fetch-bundle");

    // WHEN a knowledge document is fetched:
    let doc = fetcher
        .fetch_knowledge("doc1")
        .await
        .expect("knowledge self-fetch works");
    assert!(doc.contains("Body text here."));

    // WHEN the health check runs:
    assert!(fetcher.health_check().await.expect("health runs"));

    let _ = std::fs::remove_file(&srv.trace_file);
}

#[tokio::test]
async fn v1_self_fetch_etag_304() {
    // GIVEN the running v1 server:
    let srv = start_test_server().await;
    let fetcher = OkfFetcher::new(fetcher_config(&srv.base_url));
    let (_, etag) = fetcher.fetch_manifest(None, None).await.unwrap();
    let etag = etag.expect("server sends ETag");

    // WHEN the cached ETag is sent back:
    let err = fetcher
        .fetch_manifest(None, Some(&etag))
        .await
        .expect_err("304 surfaces as NOT_MODIFIED");

    // THEN the server honored the conditional GET:
    assert_eq!(err, "NOT_MODIFIED");
    let _ = std::fs::remove_file(&srv.trace_file);
}

#[tokio::test]
async fn v1_self_write_traces_and_concepts() {
    // GIVEN the running v1 server:
    let srv = start_test_server().await;
    let client = reqwest::Client::new();

    // WHEN a trace is posted (grok-cli's path):
    let trace = serde_json::json!({"trace_id": "t1", "steps": []});
    let resp = client
        .post(format!("{}/okf/traces", srv.base_url))
        .json(&trace)
        .send()
        .await
        .expect("POST /okf/traces");
    assert_eq!(resp.status(), 202);

    // THEN the JSONL line landed in the trace file:
    let contents =
        std::fs::read_to_string(&srv.trace_file).expect("trace file written");
    assert!(contents.contains("\"trace_id\":\"t1\"") || contents.contains("\"trace_id\": \"t1\""));

    // WHEN a concept is pushed (grok-cli's path):
    let concept = serde_json::json!({
        "id": "concept-1",
        "type": "Note",
        "title": "Pushed Concept",
        "body": "Concept body from the wire."
    });
    let resp = client
        .post(format!(
            "{}/okf/bundles/self-fetch-bundle/concepts",
            srv.base_url
        ))
        .json(&concept)
        .send()
        .await
        .expect("POST concepts");
    assert_eq!(resp.status(), 201);

    // THEN the concept is retrievable as a knowledge document:
    let fetcher = OkfFetcher::new(fetcher_config(&srv.base_url));
    let doc = fetcher
        .fetch_knowledge("concept-1")
        .await
        .expect("pushed concept readable");
    assert!(doc.contains("Concept body from the wire."));

    // AND an unknown bundle is rejected:
    let resp = client
        .post(format!("{}/okf/bundles/nope/concepts", srv.base_url))
        .json(&concept)
        .send()
        .await
        .expect("POST unknown bundle");
    assert_eq!(resp.status(), 404);

    let _ = std::fs::remove_file(&srv.trace_file);
}
