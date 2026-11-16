//! OKF context authoring (task 365, Phase 2).
//!
//! Same shape as skills: the loop drafts a new `OkfBundleManifest`
//! (id + version + knowledge entries with inline markdown or URLs +
//! tools + schemas), validates it with `OkfValidator`, and delivers
//! it for John's tap — it never pushes anything itself.
//!
//! A context draft is a PROPOSAL, not a publish:
//! `propose_context_draft` validates the manifest and stores it in
//! `rt/self_improve/context_drafts.json` with status "proposed".
//! Nothing here touches the librarian or the in-memory registry.
//!
//! Delivery boundary (the task's must-say-which): a manifest pushed
//! to the local librarian (`POST /okf/manifest`) is IN-MEMORY ONLY —
//! the next remote reload from the OKF server (.106) overwrites it
//! (auto_reload / webhook / poller all reload from the server).
//! The DURABLE path is publishing the bundle to the OKF server
//! itself — that is John's step (or a server-side write), not the
//! loop's. A draft delivered for his tap must say which path it
//! takes; this module records drafts only.
//!
//! An OKF tool entry is NOT a skill: `try_execute_okf_tool`
//! simulates (it returns metadata plus a "remote call simulated"
//! line; `TODO(real)` dispatch is unimplemented). An OKF tool entry
//! describes a remote tool; a skill is a `SkillRegistry` closure
//! that runs. Do not confuse the two.

use crate::okf::{OkfBundleManifest, OkfValidator, ValidationResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A loop-drafted OKF context, awaiting John's tap. Lives in
/// `rt/self_improve/context_drafts.json` — NOT the librarian, NOT
/// the OKF server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextDraft {
    pub manifest: OkfBundleManifest,
    /// Where the draft came from: an eval finding trace, or John's
    /// request (same trigger order as skills — task 362).
    pub source: String,
    /// Why this context helps (one or two sentences).
    pub rationale: String,
    /// Lifecycle: "proposed" at intake; later "approved",
    /// "published", or "declined" (John's tap moves it).
    pub status: String,
    /// Unix milliseconds at intake.
    pub created_at: u64,
    /// Unix milliseconds of the last status change.
    pub updated_at: u64,
}

pub fn context_drafts_path(rt_dir: &Path) -> PathBuf {
    rt_dir.join("self_improve").join("context_drafts.json")
}

/// Read stored context drafts. Missing file reads as empty; an
/// unparseable file is an error (never silently append over corrupt
/// data).
pub fn read_context_drafts(rt_dir: &Path) -> Result<Vec<ContextDraft>, String> {
    let path = context_drafts_path(rt_dir);
    let Ok(data) = std::fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    serde_json::from_str::<Vec<ContextDraft>>(&data)
        .map_err(|e| format!("context drafts read failed: {e}"))
}

/// Validate a manifest with the OKF validator (id + version required,
/// no duplicate tool/knowledge/schema ids, version format, diagram
/// entries need content or a URL). Warnings do not block.
pub fn validate_context_draft(manifest: &OkfBundleManifest) -> ValidationResult {
    OkfValidator::new().validate_manifest(manifest)
}

/// Validate + store a context draft as "proposed" (atomic temp +
/// rename). An invalid manifest is rejected BEFORE anything is
/// written — the errors are returned, never stored. This never
/// touches the librarian/registry or the OKF server; the durable
/// publish is John's step.
pub fn propose_context_draft(
    rt_dir: &Path,
    manifest: OkfBundleManifest,
    source: &str,
    rationale: &str,
    now_ms: u64,
) -> Result<ContextDraft, String> {
    let result = validate_context_draft(&manifest);
    if !result.is_valid {
        return Err(format!(
            "context draft failed validation: {}",
            result.errors.join("; ")
        ));
    }
    let draft = ContextDraft {
        manifest,
        source: source.to_string(),
        rationale: rationale.to_string(),
        status: "proposed".to_string(),
        created_at: now_ms,
        updated_at: now_ms,
    };
    let mut all = read_context_drafts(rt_dir)?;
    all.push(draft.clone());
    let path = context_drafts_path(rt_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("context drafts dir create failed: {e}"))?;
    }
    let data = serde_json::to_string_pretty(&all)
        .map_err(|e| format!("context drafts serialize failed: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, data).map_err(|e| format!("context drafts temp write failed: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("context drafts rename failed: {e}"))?;
    Ok(draft)
}

/// The drafter brief for one context draft. The drafter returns the
/// manifest as a single ```json fenced block and nothing else.
pub fn draft_context_prompt(source: &str, rationale: &str) -> String {
    format!(
        "Helix self-improvement loop context drafter (Phase 2, OKF authoring). \
         You are a read-only investigator: read the relevant repo files and notes with your tools, then return the context. \
         You write no files yourself; the handler validates your draft and stores it for John's tap.\n\n\
         Source: {source}\n\
         Rationale: {rationale}\n\n\
         Return ONLY a manifest as a single ```json fenced block, in this shape:\n\
         ```json\n\
         {{\n  \"id\": \"<bundle id, e.g. helix-<topic>-v1>\",\n  \"version\": \"1.0.0\",\n  \"name\": \"<human-readable name>\",\n  \"description\": \"<what this context is for>\",\n  \"tools\": [{{\"name\": \"<tool>\", \"description\": \"<what it does>\"}}],\n  \"knowledge\": [{{\"id\": \"<entry id>\", \"title\": \"<title>\", \"content_type\": \"markdown\", \"content\": \"<inline markdown>\"}}],\n  \"schemas\": []\n}}\n```\n\n\
         Rules: id and version are required; knowledge entries carry inline markdown OR a url (never neither); \
         tool entries are DESCRIPTIVE only — an OKF tool entry is not a skill and never executes code; \
         do not put secrets, credentials, or tokens anywhere in the manifest. Return ONLY the fenced block.",
    )
}

/// Parse a drafter response into a manifest: a ```json fenced block
/// first, else the whole trimmed text as JSON. Parsing is not
/// validation — the caller must still run `validate_context_draft`.
pub fn parse_context_draft_response(text: &str) -> Result<OkfBundleManifest, String> {
    let fenced = text
        .split("```json")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .map(|s| s.trim().to_string());
    let candidate = fenced.unwrap_or_else(|| text.trim().to_string());
    OkfBundleManifest::from_json(&candidate)
        .map_err(|e| format!("context draft is not a valid manifest JSON: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::okf::manifest::OkfKnowledgeEntry;

    fn manifest(id: &str) -> OkfBundleManifest {
        OkfBundleManifest {
            id: id.to_string(),
            version: "1.0.0".to_string(),
            name: "Test bundle".to_string(),
            description: Some("A test context".to_string()),
            tools: Vec::new(),
            knowledge: vec![OkfKnowledgeEntry {
                id: "k1".to_string(),
                title: "Intro".to_string(),
                content_type: "markdown".to_string(),
                content: Some("# Intro\n".to_string()),
                url: None,
                version: None,
                metadata: std::collections::HashMap::new(),
            }],
            schemas: Vec::new(),
            metadata: std::collections::HashMap::new(),
            generated_at: None,
        }
    }

    fn tmpdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn validate_good_and_bad_manifests() {
        // GIVEN a well-formed manifest, WHEN validated, THEN valid;
        // GIVEN a missing id, THEN invalid with an error naming id.
        assert!(validate_context_draft(&manifest("helix-test-v1")).is_valid);
        let mut bad = manifest("helix-test-v1");
        bad.id = String::new();
        let result = validate_context_draft(&bad);
        assert!(!result.is_valid);
        assert!(result.errors.iter().any(|e| e.contains("'id'")));
    }

    #[test]
    fn parse_fenced_manifest_response() {
        // GIVEN a drafter response with a ```json block, WHEN parsed,
        // THEN the manifest has the right id/version/knowledge.
        let text = "```json\n{\"id\": \"helix-ctx-v1\", \"version\": \"1.0.0\", \
            \"knowledge\": [{\"id\": \"k1\", \"content_type\": \"markdown\", \"content\": \"# Hi\"}]}\n```";
        let m = parse_context_draft_response(text).unwrap();
        assert_eq!(m.id, "helix-ctx-v1");
        assert_eq!(m.version, "1.0.0");
        assert_eq!(m.knowledge.len(), 1);
        assert_eq!(m.knowledge[0].id, "k1");
    }

    #[test]
    fn parse_bare_json_and_reject_garbage() {
        // Bare JSON (no fence) is accepted; garbage and a response
        // with no JSON at all are rejected.
        let bare = "{\"id\": \"helix-ctx-v1\", \"version\": \"1.0.0\"}";
        assert_eq!(parse_context_draft_response(bare).unwrap().id, "helix-ctx-v1");
        assert!(parse_context_draft_response("not json at all").is_err());
        assert!(parse_context_draft_response("```json\n{\"id\": 1}\n```").is_err());
    }

    #[test]
    fn propose_stores_valid_and_rejects_invalid() {
        // GIVEN a valid manifest, WHEN proposed, THEN it is stored as
        // "proposed" with source/rationale; GIVEN an invalid one,
        // THEN it is rejected and nothing is stored.
        let dir = tmpdir("helix-self-improve-context-propose");
        let draft = propose_context_draft(
            &dir,
            manifest("helix-ctx-v1"),
            "errors:logs/error_log.md",
            "Error-log context for the loop",
            2_000,
        )
        .unwrap();
        assert_eq!(draft.status, "proposed");
        assert_eq!(draft.source, "errors:logs/error_log.md");
        let stored = read_context_drafts(&dir).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].manifest.id, "helix-ctx-v1");
        // Invalid: rejected, file unchanged.
        let mut bad = manifest("helix-ctx-v1");
        bad.id = String::new();
        let r = propose_context_draft(&dir, bad, "x", "y", 3_000);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("failed validation"));
        assert_eq!(read_context_drafts(&dir).unwrap().len(), 1);
        // A second valid draft appends.
        propose_context_draft(&dir, manifest("helix-ctx-v2"), "x", "y", 4_000).unwrap();
        assert_eq!(read_context_drafts(&dir).unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn propose_writes_only_the_drafts_file() {
        // GIVEN a fresh runtime dir, WHEN a draft is proposed, THEN
        // the only file created under self_improve is the drafts file
        // — the librarian/registry is never touched.
        let dir = tmpdir("helix-self-improve-context-only");
        propose_context_draft(&dir, manifest("helix-ctx-v1"), "x", "y", 2_000).unwrap();
        let entries: Vec<String> = std::fs::read_dir(dir.join("self_improve"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(entries, vec!["context_drafts.json".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn draft_context_prompt_carries_source_and_shape() {
        let prompt = draft_context_prompt("errors:logs/error_log.md", "Error-log context");
        assert!(prompt.contains("errors:logs/error_log.md"));
        assert!(prompt.contains("Error-log context"));
        assert!(prompt.contains("```json"));
        assert!(prompt.contains("DESCRIPTIVE only"));
    }
}
