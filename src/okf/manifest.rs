//! OKF Manifest Parser (Task 164.2)
//!
//! Defines the canonical structs for an OKF bundle manifest.
//! These are parsed from `manifest.json` files served by the remote OKF server.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level manifest for an OKF bundle.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OkfBundleManifest {
    /// Unique identifier for this bundle (e.g. "helix-core-tools-v1")
    pub id: String,

    /// Semantic version of the bundle
    pub version: String,

    /// Human-readable name
    #[serde(default)]
    pub name: String,

    /// Optional description
    #[serde(default)]
    pub description: Option<String>,

    /// List of tools exposed by this bundle
    #[serde(default)]
    pub tools: Vec<OkfToolEntry>,

    /// List of knowledge documents / bundles
    #[serde(default)]
    pub knowledge: Vec<OkfKnowledgeEntry>,

    /// JSON Schema definitions provided by the bundle
    #[serde(default)]
    pub schemas: Vec<OkfSchemaEntry>,

    /// Arbitrary metadata (author, license, tags, etc.)
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,

    /// Timestamp when this manifest was generated (ISO 8601 or unix ms)
    #[serde(default)]
    pub generated_at: Option<String>,
}

/// A single tool definition inside an OKF bundle.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OkfToolEntry {
    pub name: String,
    #[serde(default)]
    pub description: String,

    /// JSON Schema for the input parameters (as object)
    #[serde(default)]
    pub input_schema: Option<serde_json::Value>,

    /// JSON Schema for the output (as object)
    #[serde(default)]
    pub output_schema: Option<serde_json::Value>,

    /// Optional tags for categorization
    #[serde(default)]
    pub tags: Vec<String>,

    /// Optional implementation hint (e.g. "rust", "python", "remote")
    #[serde(default)]
    pub implementation: Option<String>,
}

/// A knowledge document or bundle reference.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OkfKnowledgeEntry {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub content_type: String, // e.g. "markdown", "json", "text"

    /// Either inline content or a URL to fetch the full content
    #[serde(default)]
    pub content: Option<String>,

    #[serde(default)]
    pub url: Option<String>,

    #[serde(default)]
    pub version: Option<String>,

    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// A JSON Schema definition exported by the bundle.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OkfSchemaEntry {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    pub schema: serde_json::Value,
}

impl OkfBundleManifest {
    /// Parse a manifest from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Serialize back to pretty JSON (useful for debugging / writing index files).
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Basic structural validation (more thorough validation lives in okf_validator).
    pub fn is_valid(&self) -> bool {
        !self.id.is_empty() && !self.version.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_manifest() {
        let json = r#"{
            "id": "test-bundle",
            "version": "1.0.0",
            "tools": [
                {
                    "name": "example_tool",
                    "description": "Does something useful"
                }
            ]
        }"#;

        let manifest = OkfBundleManifest::from_json(json).expect("parse failed");
        assert_eq!(manifest.id, "test-bundle");
        assert_eq!(manifest.version, "1.0.0");
        assert_eq!(manifest.tools.len(), 1);
        assert_eq!(manifest.tools[0].name, "example_tool");
        assert!(manifest.is_valid());
    }

    #[test]
    fn test_invalid_manifest_missing_fields() {
        let json = r#"{"name": "broken"}"#;
        let manifest = OkfBundleManifest::from_json(json).unwrap_or_default();
        assert!(!manifest.is_valid());
    }
}
