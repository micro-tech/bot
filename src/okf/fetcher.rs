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
            Ok(resp) => Ok(false),
            Err(_) => Ok(false),
        }
    }
}
