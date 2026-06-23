//! Concrete implementation of the RepoExplorer.
//!
//! FastContext-style local explorer that can run multiple read-only tools
//! (repo_glob, repo_read, repo_grep) in parallel, aggregates compact evidence,
//! and returns high-signal results to the main agent.

use super::{LocalLLMClient, RepoEvidence, RepoEvidenceItem, RepoExplorer, RepoQuery};
use async_trait::async_trait;
use serde_json::{json, Value};

/// System prompt that keeps the explorer strictly read-only.
const EXPLORER_SYSTEM_PROMPT: &str = r#"
You are a repository exploration assistant. You may ONLY use these read-only tools:
- repo_glob (pattern)
- repo_read (path, optional start_line, end_line)
- repo_grep (pattern, optional path)

Never suggest write/delete operations.
You can request MULTIPLE tools in one response as a JSON array:
[{"action":"repo_glob","args":{...}}, {"action":"repo_read","args":{...}}]

When you have enough evidence output:
{"action":"done","evidence":[{ "path": "...", "line_start": N, "line_end": N, "summary": "..." }]}

Keep evidence minimal and high-signal. Max 8 items total.
"#;

/// The concrete explorer.
pub struct LocalRepoExplorer {
    llm: LocalLLMClient,
    root: String,
    max_steps: usize,
    max_parallel: usize,
}

impl LocalRepoExplorer {
    pub fn new(llm: LocalLLMClient, root: impl Into<String>) -> Self {
        Self {
            llm,
            root: root.into(),
            max_steps: 5,
            max_parallel: 4,
        }
    }

    /// Execute a single tool decision (used by both sequential and parallel paths).
    async fn execute_single_tool(&self, decision: &Value) -> Option<RepoEvidenceItem> {
        let action = decision.get("action").and_then(|a| a.as_str()).unwrap_or("");
        let args = decision.get("args").cloned().unwrap_or_else(|| json!({}));

        match action {
            "repo_glob" => {
                let pattern = args.get("pattern").and_then(|p| p.as_str()).unwrap_or("**/*");
                let result = crate::tools::execute("repo_glob", &json!({"pattern": pattern}));
                Some(RepoEvidenceItem {
                    path: format!("[glob:{}]", pattern),
                    line_start: None,
                    line_end: None,
                    summary: result.chars().take(140).collect(),
                })
            }
            "repo_read" => {
                let path = args.get("path").and_then(|p| p.as_str()).unwrap_or("");
                if path.is_empty() { return None; }
                let start = args.get("start_line").and_then(|v| v.as_u64()).map(|v| v as usize);
                let end = args.get("end_line").and_then(|v| v.as_u64()).map(|v| v as usize);

                let mut call_args = json!({"path": path});
                if let Some(s) = start { call_args["start_line"] = json!(s); }
                if let Some(e) = end { call_args["end_line"] = json!(e); }

                let content = crate::tools::execute("repo_read", &call_args);
                Some(RepoEvidenceItem {
                    path: path.to_string(),
                    line_start: start,
                    line_end: end,
                    summary: content.chars().take(220).collect(),
                })
            }
            "repo_grep" => {
                let pattern = args.get("pattern").and_then(|p| p.as_str()).unwrap_or("");
                if pattern.is_empty() { return None; }
                let path = args.get("path").and_then(|p| p.as_str()).unwrap_or(".");
                let result = crate::tools::execute("repo_grep", &json!({"pattern": pattern, "path": path}));
                Some(RepoEvidenceItem {
                    path: path.to_string(),
                    line_start: None,
                    line_end: None,
                    summary: result.chars().take(180).collect(),
                })
            }
            _ => None,
        }
    }

    /// Execute multiple tool decisions in parallel (for 159.4 requirement).
    async fn execute_parallel(&self, decisions: &[Value]) -> Vec<RepoEvidenceItem> {
        use futures_util::future::join_all;

        let futures: Vec<_> = decisions
            .iter()
            .take(self.max_parallel)
            .map(|d| self.execute_single_tool(d))
            .collect();

        let results = join_all(futures).await;
        results.into_iter().filter_map(|r| r).collect()
    }

    /// Core exploration with support for parallel tool calls.
    async fn run_exploration(&self, query: &RepoQuery) -> RepoEvidence {
        let cwd = query.cwd.as_deref().unwrap_or(&self.root);
        let hints = if query.hints.is_empty() {
            String::new()
        } else {
            format!("Hints: {}", query.hints.join(", "))
        };

        let mut evidence_items: Vec<RepoEvidenceItem> = Vec::new();

        for _step in 0..self.max_steps {
            let user_prompt = format!(
                "Task: {}\nWorking directory: {}\n{}\nEvidence so far: {}\nDecide next action(s) or finish.",
                query.natural_language, cwd, hints, evidence_items.len()
            );

            let decision_text = self.llm.chat(EXPLORER_SYSTEM_PROMPT, &user_prompt).await;

            // Check for explicit done
            if decision_text.contains("\"action\":\"done\"") {
                if let Ok(parsed) = serde_json::from_str::<Value>(&decision_text) {
                    if let Some(ev_arr) = parsed.get("evidence").and_then(|e| e.as_array()) {
                        for item in ev_arr {
                            if let (Some(path), Some(summary)) = (
                                item.get("path").and_then(|p| p.as_str()),
                                item.get("summary").and_then(|s| s.as_str()),
                            ) {
                                evidence_items.push(RepoEvidenceItem {
                                    path: path.to_string(),
                                    line_start: item.get("line_start").and_then(|v| v.as_u64()).map(|v| v as usize),
                                    line_end: item.get("line_end").and_then(|v| v.as_u64()).map(|v| v as usize),
                                    summary: summary.to_string(),
                                });
                            }
                        }
                    }
                }
                break;
            }

            // Try to parse as array (parallel) or single object
            let parsed: Result<Value, _> = serde_json::from_str(&decision_text);
            let mut new_items = Vec::new();

            if let Ok(val) = parsed {
                if let Some(arr) = val.as_array() {
                    // Parallel execution path (159.4)
                    new_items = self.execute_parallel(arr).await;
                } else if val.is_object() {
                    if let Some(item) = self.execute_single_tool(&val).await {
                        new_items.push(item);
                    }
                }
            }

            if new_items.is_empty() {
                break; // LLM produced unparseable output
            }

            evidence_items.extend(new_items);

            // Safety cap
            if evidence_items.len() >= 8 {
                evidence_items.truncate(8);
                break;
            }
        }

        RepoEvidence { items: evidence_items }
    }
}

#[async_trait]
impl RepoExplorer for LocalRepoExplorer {
    async fn explore(&self, query: RepoQuery) -> RepoEvidence {
        self.run_exploration(&query).await
    }
}