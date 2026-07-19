//! OKF Validator (Task 164.4)
//!
//! Validates OKF bundle manifests and registry contents.
//! Performs structural, semantic, and consistency checks.

use crate::okf::manifest::{OkfBundleManifest, OkfToolEntry};
use crate::okf::registry::OkfRegistry;

/// Result of a validation run.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl ValidationResult {
    pub fn ok() -> Self {
        Self {
            is_valid: true,
            errors: vec![],
            warnings: vec![],
        }
    }

    pub fn with_error(msg: &str) -> Self {
        Self {
            is_valid: false,
            errors: vec![msg.to_string()],
            warnings: vec![],
        }
    }

    pub fn add_error(&mut self, msg: &str) {
        self.is_valid = false;
        self.errors.push(msg.to_string());
    }

    pub fn add_warning(&mut self, msg: &str) {
        self.warnings.push(msg.to_string());
    }
}

/// Validator for OKF bundles and registries.
#[derive(Debug, Clone, Default)]
pub struct OkfValidator {
    /// Whether to enforce strict semver on versions.
    pub strict_version: bool,
}

impl OkfValidator {
    pub fn new() -> Self {
        Self { strict_version: false }
    }

    pub fn with_strict_version(mut self, strict: bool) -> Self {
        self.strict_version = strict;
        self
    }

    /// Validate a full manifest.
    pub fn validate_manifest(&self, manifest: &OkfBundleManifest) -> ValidationResult {
        let mut result = ValidationResult::ok();

        // Required fields
        if manifest.id.trim().is_empty() {
            result.add_error("Bundle 'id' is required and cannot be empty");
        }
        if manifest.version.trim().is_empty() {
            result.add_error("Bundle 'version' is required and cannot be empty");
        }

        // Version format check
        if !self.is_valid_version(&manifest.version) {
            let msg = format!("Invalid version format: '{}'. Expected semver-like (e.g. 1.2.3)", manifest.version);
            if self.strict_version {
                result.add_error(&msg);
            } else {
                result.add_warning(&msg);
            }
        }

        // Tool validation
        let mut seen_tools = std::collections::HashSet::new();
        for (i, tool) in manifest.tools.iter().enumerate() {
            if tool.name.trim().is_empty() {
                result.add_error(&format!("Tool #{} has empty name", i));
            } else if !seen_tools.insert(&tool.name) {
                result.add_error(&format!("Duplicate tool name: '{}'", tool.name));
            }

            // Basic description check
            if tool.description.trim().is_empty() {
                result.add_warning(&format!("Tool '{}' has no description", tool.name));
            }
        }

        // Knowledge validation
        let mut seen_knowledge = std::collections::HashSet::new();
        let allowed_content_types = [
            "", "markdown", "json", "text", "yaml",
            "mermaid", "diagram", "file-tree", "directory-snapshot",
            "file-change-log", "project-map"
        ];

        for (i, k) in manifest.knowledge.iter().enumerate() {
            if k.id.trim().is_empty() {
                result.add_error(&format!("Knowledge entry #{} has empty id", i));
            } else if !seen_knowledge.insert(&k.id) {
                result.add_error(&format!("Duplicate knowledge id: '{}'", k.id));
            }

            if !k.content_type.is_empty() && !allowed_content_types.contains(&k.content_type.as_str()) {
                result.add_warning(&format!(
                    "Knowledge '{}' uses non-standard content_type '{}'. Consider using 'mermaid', 'file-tree', 'project-map', etc.",
                    k.id, k.content_type
                ));
            }

            // Special validation for diagram types
            if matches!(k.content_type.as_str(), "mermaid" | "diagram" | "project-map") {
                if k.content.as_ref().map_or(true, |c| c.trim().is_empty()) && k.url.is_none() {
                    result.add_warning(&format!(
                        "Diagram knowledge '{}' has no inline content and no url. It should contain Mermaid source or a link.",
                        k.id
                    ));
                }
            }
        }

        // Schema validation
        let mut seen_schemas = std::collections::HashSet::new();
        for (i, s) in manifest.schemas.iter().enumerate() {
            if s.id.trim().is_empty() {
                result.add_error(&format!("Schema #{} has empty id", i));
            } else if !seen_schemas.insert(&s.id) {
                result.add_error(&format!("Duplicate schema id: '{}'", s.id));
            }

            if !s.schema.is_object() {
                result.add_warning(&format!("Schema '{}' does not appear to be a valid JSON object", s.id));
            }
        }

        result
    }

    /// Validate a built registry.
    pub fn validate_registry(&self, registry: &OkfRegistry) -> ValidationResult {
        let mut result = ValidationResult::ok();

        if registry.bundle_id.as_ref().map_or(true, |s| s.trim().is_empty()) {
            result.add_warning("Registry has no bundle_id");
        }

        if registry.bundle_version.as_ref().map_or(true, |s| s.trim().is_empty()) {
            result.add_warning("Registry has no bundle_version");
        }

        // Check that every tool in registry has a name
        for name in registry.tools.keys() {
            if name.trim().is_empty() {
                result.add_error("Registry contains a tool with empty name");
            }
        }

        result
    }

    /// Validate a single tool definition.
    pub fn validate_tool(&self, tool: &OkfToolEntry) -> ValidationResult {
        let mut result = ValidationResult::ok();

        if tool.name.trim().is_empty() {
            result.add_error("Tool name cannot be empty");
        }
        if tool.description.trim().is_empty() {
            result.add_warning("Tool has no description");
        }

        result
    }

    fn is_valid_version(&self, version: &str) -> bool {
        // Very simple semver-ish check: major.minor.patch or major.minor
        let parts: Vec<&str> = version.split('.').collect();
        if parts.is_empty() || parts[0].is_empty() {
            return false;
        }
        // Accept 1.0, 1.2.3, v1.2.3, 2.0.0-alpha etc. loosely
        parts.iter().all(|p| !p.is_empty() && p.chars().next().unwrap().is_ascii_digit() || p.starts_with('v'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::okf::manifest::OkfBundleManifest;

    #[test]
    fn test_validate_good_manifest() {
        let json = r#"{
            "id": "core-tools",
            "version": "1.0.0",
            "tools": [{"name": "search", "description": "Search"}]
        }"#;
        let manifest = OkfBundleManifest::from_json(json).unwrap();
        let validator = OkfValidator::new();
        let result = validator.validate_manifest(&manifest);

        assert!(result.is_valid);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_validate_bad_manifest() {
        let json = r#"{
            "id": "",
            "version": "badver",
            "tools": [{"name": "search"}, {"name": "search"}]
        }"#;
        let manifest = OkfBundleManifest::from_json(json).unwrap();
        let validator = OkfValidator::new();
        let result = validator.validate_manifest(&manifest);

        assert!(!result.is_valid);
        assert!(result.errors.len() >= 2);
    }

    #[test]
    fn test_validate_registry() {
        let mut reg = OkfRegistry::new();
        reg.bundle_id = Some("test".into());
        reg.bundle_version = Some("1.0.0".into());

        let validator = OkfValidator::new();
        let result = validator.validate_registry(&reg);
        assert!(result.is_valid);
    }
}
