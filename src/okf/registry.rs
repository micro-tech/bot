//! OKF Registry (Task 164.3)
//!
//! Builds and holds in-memory registries from OKF bundle manifests.
//! Supports tools, knowledge, schemas, and version tracking.
//! Can generate index files for persistence / serving.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::okf::manifest::{OkfBundleManifest, OkfKnowledgeEntry, OkfSchemaEntry, OkfToolEntry};

/// In-memory registry built from one or more OKF manifests.
/// This is the central data structure for the OKF Librarian.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OkfRegistry {
    /// Map of tool name -> tool definition
    pub tools: HashMap<String, OkfToolEntry>,

    /// Map of knowledge id -> knowledge entry
    pub knowledge: HashMap<String, OkfKnowledgeEntry>,

    /// Map of schema id -> schema definition
    pub schemas: HashMap<String, OkfSchemaEntry>,

    /// Current active bundle version (if loaded from a manifest)
    pub bundle_version: Option<String>,

    /// Bundle ID this registry was built from
    pub bundle_id: Option<String>,

    /// When this registry was last updated (unix timestamp ms)
    pub last_updated: Option<u64>,
}

impl OkfRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a registry from a single manifest.
    pub fn from_manifest(manifest: &OkfBundleManifest) -> Self {
        let mut reg = Self::new();

        reg.bundle_id = Some(manifest.id.clone());
        reg.bundle_version = Some(manifest.version.clone());
        reg.last_updated = Some(crate::utils::now_ms());

        for tool in &manifest.tools {
            reg.tools.insert(tool.name.clone(), tool.clone());
        }

        for knowledge in &manifest.knowledge {
            reg.knowledge.insert(knowledge.id.clone(), knowledge.clone());
        }

        for schema in &manifest.schemas {
            reg.schemas.insert(schema.id.clone(), schema.clone());
        }

        reg
    }

    /// Merge another manifest into this registry (later bundles can override).
    pub fn merge_manifest(&mut self, manifest: &OkfBundleManifest) {
        self.bundle_id = Some(manifest.id.clone());
        self.bundle_version = Some(manifest.version.clone());
        self.last_updated = Some(crate::utils::now_ms());

        for tool in &manifest.tools {
            self.tools.insert(tool.name.clone(), tool.clone());
        }

        for knowledge in &manifest.knowledge {
            self.knowledge.insert(knowledge.id.clone(), knowledge.clone());
        }

        for schema in &manifest.schemas {
            self.schemas.insert(schema.id.clone(), schema.clone());
        }
    }

    /// Number of tools currently registered.
    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    /// Number of knowledge entries.
    pub fn knowledge_count(&self) -> usize {
        self.knowledge.len()
    }

    /// List all registered tool names (sorted).
    pub fn list_tool_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.tools.keys().cloned().collect();
        names.sort();
        names
    }

    /// Get a specific tool by name.
    pub fn get_tool(&self, name: &str) -> Option<&OkfToolEntry> {
        self.tools.get(name)
    }

    /// Get a knowledge entry by id.
    pub fn get_knowledge(&self, id: &str) -> Option<&OkfKnowledgeEntry> {
        self.knowledge.get(id)
    }

    /// Generate a compact index summary (useful for /helix/okf/index API).
    pub fn to_index_summary(&self) -> serde_json::Value {
        serde_json::json!({
            "bundle_id": self.bundle_id,
            "version": self.bundle_version,
            "last_updated": self.last_updated,
            "tool_count": self.tool_count(),
            "knowledge_count": self.knowledge_count(),
            "tools": self.list_tool_names(),
        })
    }

    /// Serialize the entire registry to pretty JSON.
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Write the registry to a file (e.g. okf_index.json).
    pub fn write_to_file(&self, path: &str) -> std::io::Result<()> {
        let json = self.to_json_pretty().map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, e)
        })?;
        fs::write(path, json)
    }

    /// Load a registry from a previously written index file.
    pub fn load_from_file(path: &str) -> std::io::Result<Self> {
        let content = fs::read_to_string(path)?;
        let reg: OkfRegistry = serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(reg)
    }

    /// Clear the registry.
    pub fn clear(&mut self) {
        self.tools.clear();
        self.knowledge.clear();
        self.schemas.clear();
        self.bundle_version = None;
        self.bundle_id = None;
        self.last_updated = None;
    }
}

impl OkfRegistry {
    /// Convenience: build from a JSON manifest string.
    pub fn from_json_manifest(json: &str) -> Result<Self, String> {
        let manifest = OkfBundleManifest::from_json(json)
            .map_err(|e| format!("Failed to parse manifest: {}", e))?;
        Ok(Self::from_manifest(&manifest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::okf::manifest::OkfBundleManifest;

    fn sample_manifest() -> OkfBundleManifest {
        OkfBundleManifest::from_json(r#"{
            "id": "test-bundle",
            "version": "1.2.0",
            "tools": [
                {"name": "search", "description": "Search knowledge"},
                {"name": "read_file", "description": "Read a file"}
            ],
            "knowledge": [
                {"id": "guide-1", "title": "Getting Started"}
            ]
        }"#).unwrap()
    }

    #[test]
    fn test_registry_from_manifest() {
        let manifest = sample_manifest();
        let reg = OkfRegistry::from_manifest(&manifest);

        assert_eq!(reg.tool_count(), 2);
        assert_eq!(reg.knowledge_count(), 1);
        assert_eq!(reg.bundle_version.as_deref(), Some("1.2.0"));
        assert!(reg.get_tool("search").is_some());
        assert!(reg.list_tool_names().contains(&"search".to_string()));
    }

    #[test]
    fn test_registry_index_summary() {
        let manifest = sample_manifest();
        let reg = OkfRegistry::from_manifest(&manifest);
        let summary = reg.to_index_summary();

        assert_eq!(summary["tool_count"], 2);
        assert!(summary["tools"].is_array());
    }

    #[test]
    fn test_registry_merge() {
        let mut reg = OkfRegistry::new();
        let m1 = sample_manifest();
        reg.merge_manifest(&m1);

        let m2_json = r#"{
            "id": "extra-tools",
            "version": "1.3.0",
            "tools": [{"name": "calculate", "description": "Math tool"}]
        }"#;
        let m2 = OkfBundleManifest::from_json(m2_json).unwrap();
        reg.merge_manifest(&m2);

        assert_eq!(reg.tool_count(), 3);
        assert_eq!(reg.bundle_version.as_deref(), Some("1.3.0"));
    }
}
