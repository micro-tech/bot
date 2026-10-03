//! Helix OKF Librarian System (Task #164)
//!
//! Central module for remote Open Knowledge Format bundle management.
//!
//! Modules:
//! - `manifest`  : Manifest structs + parsing
//! - `registry`  : In-memory tool/knowledge/schema registry
//! - `validator` : Structural + semantic validation
//! - `fetcher`   : Remote HTTP fetching of bundles
//!
//! The `OkfLibrarian` is the main facade that ties everything together.

pub mod api;
pub mod fetcher;
pub mod manifest;
pub mod registry;
pub mod validator;

pub use fetcher::OkfFetcher;
pub use manifest::OkfBundleManifest;
pub use registry::OkfRegistry;
pub use validator::{OkfValidator, ValidationResult};

use crate::config::okf::OkfConfig;
use std::sync::OnceLock;
use tokio::sync::Mutex as TokioMutex;
use tokio::time::{sleep, Duration};

/// The main OKF Librarian.
/// Holds config, current registry, and provides high-level operations.
#[derive(Debug, Clone)]
pub struct OkfLibrarian {
    pub config: OkfConfig,
    pub registry: OkfRegistry,
    validator: OkfValidator,

    /// Last seen ETag for efficient conditional polling / change detection (164.1)
    pub last_etag: Option<String>,

    /// Timestamp (ms) of the last successful reload (Task 165)
    pub last_reload_ms: Option<u64>,

    /// Number of successful hot-reloads performed (Task 165)
    pub reload_count: u64,

    /// Last error message from a reload attempt (if any)
    pub last_reload_error: Option<String>,
}

/// Global shared OKF librarian (using tokio Mutex to match the web server's OkfState).
/// This allows tools, CPU, agents, and Ollama to discover OKF-provided tools/knowledge.
static GLOBAL_OKF: OnceLock<std::sync::Arc<TokioMutex<OkfLibrarian>>> = OnceLock::new();

/// Install the shared OKF librarian (called once at startup from the web server).
pub fn set_global_librarian(librarian: std::sync::Arc<TokioMutex<OkfLibrarian>>) {
    let _ = GLOBAL_OKF.set(librarian);
}

/// Get a handle to the global OKF librarian (if OKF is enabled and initialized).
pub fn get_global_librarian() -> Option<std::sync::Arc<TokioMutex<OkfLibrarian>>> {
    GLOBAL_OKF.get().cloned()
}

/// Convenience: synchronously peek at current OKF tool names (best-effort, non-blocking).
pub fn list_okf_tool_names() -> Vec<String> {
    if let Some(librarian) = get_global_librarian() {
        // Try to get the lock without blocking forever
        if let Ok(guard) = librarian.try_lock() {
            return guard.registry.list_tool_names();
        }
    }
    vec![]
}

/// Try to get a description for an OKF tool.
pub fn get_okf_tool_description(name: &str) -> Option<String> {
    if let Some(librarian) = get_global_librarian() {
        if let Ok(guard) = librarian.try_lock() {
            if let Some(tool) = guard.registry.get_tool(name) {
                return Some(tool.description.clone());
            }
        }
    }
    None
}

/// Attempt to invoke a tool that was loaded from an OKF bundle.
/// 
/// Currently returns rich metadata + simulated result.
/// In future versions this will dispatch to the remote implementation
/// using the `implementation` hint + OkfFetcher.
pub fn try_execute_okf_tool(name: &str, args: &serde_json::Value) -> Option<String> {
    let librarian = get_global_librarian()?;

    // Non-blocking attempt
    let guard = librarian.try_lock().ok()?;

    if let Some(tool_entry) = guard.registry.get_tool(name) {
        let mut response = format!(
            "🧠 OKF Remote Tool: '{}'\n\
             Description: {}\n\
             Bundle: {} (v{})\n\
             Args: {}\n",
            name,
            tool_entry.description,
            guard.registry.bundle_id.as_deref().unwrap_or("unknown"),
            guard.registry.bundle_version.as_deref().unwrap_or("unknown"),
            args
        );

        if let Some(impl_hint) = &tool_entry.implementation {
            response.push_str(&format!("Implementation hint: {}\n", impl_hint));
        }

        // TODO(real): dispatch via fetcher when server protocol is defined
        response.push_str("\n(Status: executed via OKF registry — remote call simulated)");
        Some(response)
    } else {
        None
    }
}

/// Spawn a background polling task that periodically checks the remote OKF server
/// for bundle updates (supports 164.1 + 165).
///
/// Only spawns when a global librarian exists and `auto_reload` is enabled.
/// Uses a simple debounce to avoid thundering herd on startup or rapid changes.
pub fn start_okf_poller() -> Option<tokio::task::JoinHandle<()>> {
    let librarian_arc = get_global_librarian()?;

    let handle = tokio::spawn(async move {
        // Read interval once (config is stable)
        let poll_interval = {
            let guard = librarian_arc.lock().await;
            if !guard.config.auto_reload() {
                return; // auto-reload disabled
            }
            Duration::from_secs(guard.config.poll_interval_secs().max(15))
        };

        let debounce = Duration::from_secs(20);
        let mut last_reload_attempt = std::time::Instant::now() - debounce; // allow first run soon

        println!("[OKF] Background poller started (interval: {:?})", poll_interval);

        loop {
            sleep(poll_interval).await;

            // Correct for tokio::sync::Mutex: lock().await yields the guard directly (no Result).
            // (std::sync::Mutex::lock() returns Result because of poisoning; tokio's does not.)
            let mut guard = librarian_arc.lock().await;

            if !guard.config.auto_reload() {
                continue;
            }

            if last_reload_attempt.elapsed() < debounce {
                continue;
            }

            println!("[OKF 165 Poller] Polling remote OKF server for bundle updates (interval: {:?})...", poll_interval);

            match guard.hot_reload_from_remote().await {
                Ok(validation) => {
                    guard.reload_count += 1;
                    last_reload_attempt = std::time::Instant::now();

                    if validation.is_valid {
                        println!(
                            "[OKF 165 Poller] ✅ Hot-reload #{} complete. Tools: {}, Knowledge: {}",
                            guard.reload_count,
                            guard.registry.tool_count(),
                            guard.registry.knowledge_count()
                        );
                    } else {
                        guard.last_reload_error = Some(format!("Validation failed: {:?}", validation.errors));
                        eprintln!("[OKF 165 Poller] Reload #{} validation failed: {:?}", guard.reload_count, validation.errors);
                    }
                }
                Err(e) => {
                    guard.last_reload_error = Some(e.clone());
                    // Throttle error logging
                    if last_reload_attempt.elapsed() > Duration::from_secs(120) {
                        eprintln!("[OKF 165 Poller] Remote fetch error: {}", e);
                    }
                }
            }
        }
    });

    Some(handle)
}

impl OkfLibrarian {
    pub fn new(config: OkfConfig) -> Self {
        Self {
            config: config.clone(),
            registry: OkfRegistry::new(),
            validator: OkfValidator::new(),
            last_etag: None,
            last_reload_ms: None,
            reload_count: 0,
            last_reload_error: None,
        }
    }

    /// Returns true if the OKF system is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.is_enabled()
    }

    /// Load a manifest (from JSON string), validate it, and build/update the registry.
    pub fn load_manifest_from_json(&mut self, json: &str) -> Result<ValidationResult, String> {
        if !self.is_enabled() {
            return Err("OKF system is disabled in config".to_string());
        }

        let manifest = OkfBundleManifest::from_json(json)
            .map_err(|e| format!("Failed to parse manifest: {}", e))?;

        let validation = self.validator.validate_manifest(&manifest);

        if validation.is_valid {
            self.registry = OkfRegistry::from_manifest(&manifest);
            // Also write index file if configured
            let _ = self.registry.write_to_file(&self.config.index_file());
        }

        Ok(validation)
    }

    /// Fetch the latest manifest from the remote OKF server and load it.
    /// Uses ETag for efficient "no change" detection (supports 164.1).
    /// Full hot-reload pipeline for Task 165.
    pub async fn load_from_remote(&mut self) -> Result<ValidationResult, String> {
        self.hot_reload_from_remote().await
    }

    /// Core hot-reload implementation (Task 165).
    /// Performs: fetch (with ETag) → validate → update registry → persist index → update timestamps.
    /// Emits clear reload logs.
    pub async fn hot_reload_from_remote(&mut self) -> Result<ValidationResult, String> {
        if !self.is_enabled() {
            return Err("OKF system is disabled".to_string());
        }

        let fetcher = OkfFetcher::new(self.config.clone());
        let etag = self.last_etag.as_deref();

        let old_bundle = self.registry.bundle_id.clone();
        let old_version = self.registry.bundle_version.clone();
        let old_tool_count = self.registry.tool_count();
        let old_knowledge_count = self.registry.knowledge_count();

        match fetcher.fetch_manifest(None, etag).await {
            Ok((manifest, new_etag)) => {
                let validation = self.validator.validate_manifest(&manifest);

                if validation.is_valid {
                    self.registry = OkfRegistry::from_manifest(&manifest);
                    let _ = self.registry.write_to_file(&self.config.index_file());

                    if let Some(etag) = new_etag {
                        self.last_etag = Some(etag);
                    }
                    self.last_reload_ms = Some(crate::utils::now_ms());

                    // 165: Rich hot-reload logging
                    println!(
                        "[OKF 165 Hot-Reload] ✅ Bundle updated\n  \
                         ID: {} ({} → {})\n  \
                         Version: {} → {}\n  \
                         Tools: {} → {}\n  \
                         Knowledge: {} → {}\n  \
                         ETag: {:?}",
                        manifest.id,
                        old_bundle.as_deref().unwrap_or("none"),
                        manifest.id,
                        old_version.as_deref().unwrap_or("none"),
                        manifest.version,
                        old_tool_count,
                        self.registry.tool_count(),
                        old_knowledge_count,
                        self.registry.knowledge_count(),
                        self.last_etag
                    );

                    // TODO(165 future): Notify running agents / invalidate tool cache here
                    // e.g. broadcast a "okf_tools_updated" bus message
                } else {
                    eprintln!(
                        "[OKF 165 Hot-Reload] ❌ Validation failed for bundle '{}' v{}",
                        manifest.id, manifest.version
                    );
                    for err in &validation.errors {
                        eprintln!("  - {}", err);
                    }
                }

                Ok(validation)
            }
            Err(e) if e == "NOT_MODIFIED" => {
                println!("[OKF 165] No changes detected (ETag matched) — skipping hot-reload");
                Ok(ValidationResult {
                    is_valid: true,
                    errors: vec![],
                    warnings: vec!["No changes (ETag matched)".to_string()],
                })
            }
            Err(e) => {
                eprintln!("[OKF 165 Hot-Reload] Remote fetch error: {}", e);
                Err(e)
            }
        }
    }

    /// Validate the current registry.
    pub fn validate_current_registry(&self) -> ValidationResult {
        self.validator.validate_registry(&self.registry)
    }

    /// Get a summary suitable for API responses or logging.
    /// Includes hot-reload information (Task 165).
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({
            "enabled": self.is_enabled(),
            "server_url": self.config.server_url(),
            "registry": self.registry.to_index_summary(),
            "index_file": self.config.index_file(),
            "last_etag": self.last_etag,
            "last_reload_ms": self.last_reload_ms,
            "auto_reload": self.config.auto_reload(),
            "poll_interval_secs": self.config.poll_interval_secs(),
        })
    }

    /// Returns details about the last reload attempt (for 165 observability).
    pub fn last_reload_info(&self) -> serde_json::Value {
        serde_json::json!({
            "last_reload_ms": self.last_reload_ms,
            "reload_count": self.reload_count,
            "last_error": self.last_reload_error,
            "bundle_id": self.registry.bundle_id,
            "bundle_version": self.registry.bundle_version,
            "tool_count": self.registry.tool_count(),
            "knowledge_count": self.registry.knowledge_count(),
            "last_etag": self.last_etag,
        })
    }

    /// Force a reload, ignoring any cached ETag (useful for manual "pull latest" in 165).
    pub async fn force_reload_from_remote(&mut self) -> Result<ValidationResult, String> {
        self.last_etag = None; // clear ETag to force fresh fetch
        self.hot_reload_from_remote().await
    }

    /// Reload from the on-disk index file if it exists.
    pub fn reload_from_index_file(&mut self) -> Result<(), String> {
        let path = self.config.index_file();
        if std::path::Path::new(&path).exists() {
            self.registry = OkfRegistry::load_from_file(&path)
                .map_err(|e| format!("Failed to load index: {}", e))?;
            Ok(())
        } else {
            Err(format!("Index file not found: {}", path))
        }
    }
}
