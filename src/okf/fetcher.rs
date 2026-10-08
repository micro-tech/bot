//! Remote OKF Bundle Fetcher (supports 164.1, 164.7, 169)
//!
//! Fetches manifests and bundles from a remote OKF server using HTTP.
//!
//! SECURITY NOTE (169):
//! This fetcher ONLY downloads read-only manifest/knowledge data.
//! It never executes remote code, never runs arbitrary commands,
//! and never grants the remote server access to your machine.
//! All remote tool execution is still simulated locally until
//! a trusted, explicit remote-dispatch protocol is added.
//! Only enable [helix.okf] when you fully trust the server_url.

use reqwest::Client;
use std::time::Duration;

use crate::config::okf::OkfConfig;
use crate::okf::manifest::OkfBundleManifest;

/// Simple remote fetcher for OKF manifests.
#[derive(Debug, Clone)]
pub struct OkfFetcher {
    client: Client,
    config: OkfConfig,
}

impl OkfFetcher {
    pub fn new(config: OkfConfig) -> Self {
        let timeout = Duration::from_secs(config.request_timeout_secs());

        let client = Client::builder()
            .timeout(timeout)
            .user_agent("Helix-OKF-Fetcher/1.0")
            .build()
            .expect("Failed to build reqwest client");

        Self { client, config }
    }

    /// Fetch the manifest.json for a bundle.
    /// Supports conditional requests using ETag for change detection (164.1).
    /// Returns the manifest + the ETag from the response (if any).
    ///
    /// Note (169): This is a read-only data fetch. No remote execution occurs here.
    pub async fn fetch_manifest(
        &self,
        bundle_id: Option<&str>,
        if_none_match: Option<&str>,
    ) -> Result<(OkfBundleManifest, Option<String>), String> {
        if !self.config.is_enabled() {
            return Err("OKF is disabled in config".to_string());
        }

        let base = self.config.server_url();
        let url = if let Some(id) = bundle_id {
            format!("{}/okf/bundles/{}/manifest.json", base.trim_end_matches('/'), id)
        } else {
            format!("{}/okf/manifest.json", base.trim_end_matches('/'))
        };

        let mut req = self.client.get(&url);

        if let Some(etag) = if_none_match {
            if !etag.is_empty() {
                req = req.header("If-None-Match", etag);
            }
        }

        if let Some(token) = &self.config.auth_token {
            if !token.is_empty() {
                req = req.bearer_auth(token);
            }
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
            // No change — we can signal this upstream
            return Err("NOT_MODIFIED".to_string());
        }

        if !resp.status().is_success() {
            return Err(format!("Server returned status {}", resp.status()));
        }

        let etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response body: {}", e))?;

        let manifest = OkfBundleManifest::from_json(&text)
            .map_err(|e| format!("Failed to parse manifest JSON: {}", e))?;

        Ok((manifest, etag))
    }

    /// Fetch a specific knowledge document (future).
    pub async fn fetch_knowledge(&self, knowledge_id: &str) -> Result<String, String> {
        if !self.config.is_enabled() {
            return Err("OKF is disabled".to_string());
        }

        let base = self.config.server_url();
        let url = format!(
            "{}/okf/knowledge/{}",
            base.trim_end_matches('/'),
            knowledge_id
        );

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Server error: {}", resp.status()));
        }

        resp.text()
            .await
            .map_err(|e| format!("Failed to read body: {}", e))
    }

    /// Simple health check against the OKF server.
    pub async fn health_check(&self) -> Result<bool, String> {
        if !self.config.is_enabled() {
            return Ok(false);
        }

        let base = self.config.server_url();
        let url = format!("{}/okf/health", base.trim_end_matches('/'));

        match self.client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => Ok(true),
            Ok(_resp) => Ok(false),
            Err(_) => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::okf::OkfConfig;

    /// Unit tests don't run through main(), which installs the rustls ring
    /// provider — install it once here so the reqwest client can build.
    fn ensure_crypto_provider() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
    }

    /// Minimal manifest conforming to okf_bundle_schema.json (required: id, version).
    fn sample_manifest_json() -> String {
        serde_json::json!({
            "id": "test-bundle",
            "version": "1.0.0",
            "name": "Test Bundle",
            "tools": [],
            "knowledge": [
                {
                    "id": "k1",
                    "title": "Test Knowledge",
                    "content_type": "text/markdown",
                    "content": "# Hello\nTest content.",
                }
            ],
            "schemas": [],
        })
        .to_string()
    }

    fn test_config(server_url: &str, token: Option<&str>) -> OkfConfig {
        OkfConfig {
            enabled: Some(true),
            server_url: Some(server_url.to_string()),
            auth_token: token.map(|s| s.to_string()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn contract_fetch_manifest_hits_canonical_path_with_bearer_and_etag() {
        ensure_crypto_provider();
        // GIVEN a mock OKF v1 server:
        let mut server = mockito::Server::new_async().await;
        let manifest = sample_manifest_json();
        let mock = server
            .mock("GET", "/okf/manifest.json")
            .match_header("authorization", "Bearer test-token")
            .with_status(200)
            .with_header("etag", "\"v1-abc\"")
            .with_header("content-type", "application/json")
            .with_body(&manifest)
            .create_async()
            .await;

        // WHEN the fetcher requests the manifest:
        let fetcher = OkfFetcher::new(test_config(&server.url(), Some("test-token")));
        let (parsed, etag) = fetcher
            .fetch_manifest(None, None)
            .await
            .expect("fetch_manifest should succeed");

        // THEN the exact canonical path was hit once, with the Bearer <redacted>
        // the ETag was captured, and the manifest parses with required fields:
        mock.assert_async().await;
        assert_eq!(parsed.id, "test-bundle");
        assert_eq!(parsed.version, "1.0.0");
        assert_eq!(etag.as_deref(), Some("\"v1-abc\""));
        assert_eq!(parsed.knowledge.len(), 1);
    }

    #[tokio::test]
    async fn contract_fetch_manifest_bundle_scoped_path() {
        ensure_crypto_provider();
        // GIVEN a mock server serving a per-bundle manifest:
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/okf/bundles/my-bundle/manifest.json")
            .with_status(200)
            .with_body(sample_manifest_json())
            .create_async()
            .await;

        // WHEN fetching with a bundle id:
        let fetcher = OkfFetcher::new(test_config(&server.url(), None));
        let (parsed, _) = fetcher
            .fetch_manifest(Some("my-bundle"), None)
            .await
            .expect("bundle-scoped fetch should succeed");

        // THEN the bundle-scoped canonical path was hit:
        mock.assert_async().await;
        assert_eq!(parsed.id, "test-bundle");
    }

    #[tokio::test]
    async fn contract_etag_round_trip_yields_not_modified() {
        ensure_crypto_provider();
        // GIVEN a server that answers 304 when the ETag matches:
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/okf/manifest.json")
            .match_header("if-none-match", "\"v1-abc\"")
            .with_status(304)
            .create_async()
            .await;

        // WHEN the fetcher sends the cached ETag:
        let fetcher = OkfFetcher::new(test_config(&server.url(), None));
        let err = fetcher
            .fetch_manifest(None, Some("\"v1-abc\""))
            .await
            .expect_err("304 should surface as NOT_MODIFIED");

        // THEN the conditional request went out and the sentinel came back:
        mock.assert_async().await;
        assert_eq!(err, "NOT_MODIFIED");
    }

    #[tokio::test]
    async fn contract_fetch_knowledge_hits_canonical_path() {
        ensure_crypto_provider();
        // GIVEN a mock server with a knowledge document:
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/okf/knowledge/k1")
            .with_status(200)
            .with_header("content-type", "text/markdown")
            .with_body("# Hello\nTest content.")
            .create_async()
            .await;

        // WHEN fetching the document:
        let fetcher = OkfFetcher::new(test_config(&server.url(), None));
        let body = fetcher
            .fetch_knowledge("k1")
            .await
            .expect("fetch_knowledge should succeed");

        // THEN the canonical knowledge path was hit and the body returned:
        mock.assert_async().await;
        assert!(body.contains("Test content."));
    }

    #[tokio::test]
    async fn contract_health_check_hits_canonical_path() {
        ensure_crypto_provider();
        // GIVEN a mock server reporting healthy:
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/okf/health")
            .with_status(200)
            .with_body(r#"{"status":"ok"}"#)
            .create_async()
            .await;

        // WHEN the health check runs:
        let fetcher = OkfFetcher::new(test_config(&server.url(), None));
        let healthy = fetcher.health_check().await.expect("health check runs");

        // THEN the canonical health path was hit and reported healthy:
        mock.assert_async().await;
        assert!(healthy);
    }

    #[tokio::test]
    async fn contract_manifest_conforms_to_bundle_schema() {
        ensure_crypto_provider();
        // GIVEN the official schema and a manifest from the mock server:
        let schema_text =
            std::fs::read_to_string("okf_bundle_schema.json").expect("schema file present");
        let schema: serde_json::Value =
            serde_json::from_str(&schema_text).expect("schema parses");
        let required: Vec<&str> = schema["required"]
            .as_array()
            .expect("schema has required")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();

        // WHEN the manifest is fetched and parsed:
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/okf/manifest.json")
            .with_status(200)
            .with_body(sample_manifest_json())
            .create_async()
            .await;
        let fetcher = OkfFetcher::new(test_config(&server.url(), None));
        let (parsed, _) = fetcher.fetch_manifest(None, None).await.unwrap();
        let as_value = serde_json::to_value(&parsed).unwrap();

        // THEN every schema-required field is present (id, version):
        for field in required {
            assert!(
                as_value.get(field).is_some(),
                "manifest missing required field '{}'",
                field
            );
        }
    }
}
