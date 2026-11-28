//! Task-list validator (M28 — Helix triple review).
//!
//! Validates `.zed/task_list.json` — the repo's task list. The list
//! rotted silently before (a JSON conflict in task 320, a dangling
//! dependency `154 → 153`, dozens of open subtasks under `done` tasks),
//! because nothing checked it beyond "does it parse". This module is
//! the check.
//!
//! Rules (see [`validate_task_list`]):
//! 1. The file parses as JSON with a top-level `tasks` array.
//! 2. Task IDs are unique integers; every task has a non-empty title,
//!    a status in the allowed set, and a `subtasks` array.
//! 3. Every task-level dependency names an existing task.
//! 4. Subtask IDs are unique within a task (normalized to strings —
//!    the file mixes string `"154.1"` and numeric `238.1` IDs); every
//!    subtask dependency names an existing subtask (subtask deps may
//!    cross tasks, e.g. `267.2 → 266.4`) or an existing task (integer
//!    deps, e.g. `218.1 → 214`); the unified dependency graph has no
//!    cycles.
//! 5. Statuses (task and subtask) are in
//!    `pending | in_progress | done | deferred`.
//! 6. Every task holds the 3 mandatory subtasks (titles starting with
//!    "Error checking", "Cargo test coverage", "Documentation").
//! 7. A `done` task has no open subtasks (every subtask is `done`).
//!
//! Wired into CI via `cargo test --test validate_task_list`
//! (`.github/workflows/ci.yml`, "Validate task list rules (M28)") and
//! into the full suite via `cargo test --all-targets`.

use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Statuses a task or subtask may carry.
pub const ALLOWED_STATUSES: [&str; 4] = ["pending", "in_progress", "done", "deferred"];

/// Mandatory subtask title prefixes (case-insensitive). Every task must
/// hold one subtask starting with each of these (Task Builder Guide).
pub const MANDATORY_SUBTASKS: [&str; 3] =
    ["error checking", "cargo test coverage", "documentation"];

/// Validate a task-list JSON document. Returns the list of violations;
/// empty means the list is clean.
pub fn validate_task_list(json: &str) -> Vec<String> {
    let mut violations = Vec::new();
    let value: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(e) => return vec![format!("invalid JSON: {e}")],
    };
    let Some(tasks) = value.get("tasks").and_then(|t| t.as_array()) else {
        return vec!["missing or non-array top-level \"tasks\"".to_string()];
    };

    // First pass: task IDs, fields, and the task-level dependency edges.
    let mut task_ids: HashSet<i64> = HashSet::new();
    let mut task_edges: HashMap<i64, Vec<i64>> = HashMap::new();
    // Global subtask ID set (normalized to strings) — subtask deps may
    // cross tasks (e.g. 267.2 depends on 266.4).
    let mut subtask_ids: HashSet<String> = HashSet::new();
    let mut subtask_edges: HashMap<String, Vec<String>> = HashMap::new();
    // Subtask deps may also name whole TASKS (integer values, e.g.
    // 218.1 depends on tasks 214 and 215) — those resolve against the
    // task ID set, not the subtask set.
    let mut subtask_task_edges: HashMap<String, Vec<i64>> = HashMap::new();

    for task in tasks {
        let Some(task_obj) = task.as_object() else {
            violations.push("task entry is not a JSON object".to_string());
            continue;
        };
        let Some(id) = task_obj.get("id").and_then(|v| v.as_i64()) else {
            violations.push("task missing integer \"id\"".to_string());
            continue;
        };
        if !task_ids.insert(id) {
            violations.push(format!("duplicate task id {id}"));
        }
        match task_obj.get("title").and_then(|v| v.as_str()) {
            Some(t) if !t.trim().is_empty() => {}
            _ => violations.push(format!("task {id} missing non-empty \"title\"")),
        }
        let status = task_obj.get("status").and_then(|v| v.as_str());
        if !matches!(status, Some(s) if ALLOWED_STATUSES.contains(&s)) {
            violations.push(format!(
                "task {id} has invalid status {:?} (allowed: {})",
                status,
                ALLOWED_STATUSES.join(", ")
            ));
        }
        let Some(subtasks) = task_obj.get("subtasks").and_then(|v| v.as_array()) else {
            violations.push(format!("task {id} missing \"subtasks\" array"));
            continue;
        };
        // Task-level dependencies.
        if let Some(deps) = task_obj.get("dependencies") {
            match deps.as_array() {
                Some(deps) => {
                    for dep in deps {
                        match dep.as_i64() {
                            Some(d) => task_edges.entry(id).or_default().push(d),
                            None => violations
                                .push(format!("task {id} has non-integer dependency {dep}")),
                        }
                    }
                }
                None => violations.push(format!("task {id} has non-array \"dependencies\"")),
            }
        }
        // Subtasks: fields, uniqueness, mandatory three, done ⇒ all done.
        let mut seen_subtasks: HashSet<String> = HashSet::new();
        let mut titles: Vec<String> = Vec::new();
        for sub in subtasks {
            let Some(sub_obj) = sub.as_object() else {
                violations.push(format!("task {id} has a non-object subtask"));
                continue;
            };
            let Some(sub_id) = sub_obj.get("id").map(normalize_id) else {
                violations.push(format!("task {id} has a subtask missing \"id\""));
                continue;
            };
            if !seen_subtasks.insert(sub_id.clone()) {
                violations.push(format!("task {id} has duplicate subtask id {sub_id}"));
            }
            subtask_ids.insert(sub_id.clone());
            match sub_obj.get("title").and_then(|v| v.as_str()) {
                Some(t) if !t.trim().is_empty() => titles.push(t.to_string()),
                _ => violations.push(format!(
                    "task {id} subtask {sub_id} missing non-empty \"title\""
                )),
            }
            let sub_status = sub_obj.get("status").and_then(|v| v.as_str());
            if !matches!(sub_status, Some(s) if ALLOWED_STATUSES.contains(&s)) {
                violations.push(format!(
                    "task {id} subtask {sub_id} has invalid status {sub_status:?} \
                     (allowed: {})",
                    ALLOWED_STATUSES.join(", ")
                ));
            }
            if let Some(deps) = sub_obj.get("dependencies") {
                match deps.as_array() {
                    Some(deps) => {
                        for dep in deps {
                            // Integer dep = a whole task; dotted dep
                            // (string "213.1" or float 238.1) = a subtask.
                            if let Some(task_dep) = dep.as_i64() {
                                subtask_task_edges
                                    .entry(sub_id.clone())
                                    .or_default()
                                    .push(task_dep);
                            } else {
                                subtask_edges
                                    .entry(sub_id.clone())
                                    .or_default()
                                    .push(normalize_id(dep));
                            }
                        }
                    }
                    None => violations.push(format!(
                        "task {id} subtask {sub_id} has non-array \"dependencies\""
                    )),
                }
            }
        }
        // Mandatory three.
        for want in MANDATORY_SUBTASKS {
            if !titles.iter().any(|t| t.to_lowercase().starts_with(want)) {
                violations.push(format!("task {id} is missing mandatory subtask \"{want}\""));
            }
        }
        // Done task ⇒ no open subtasks.
        if status == Some("done") {
            for sub in subtasks {
                let sub_id = sub.get("id").map(normalize_id);
                let sub_status = sub.get("status").and_then(|v| v.as_str());
                if sub_status != Some("done") {
                    violations.push(format!(
                        "done task {id} has open subtask {} (status {sub_status:?})",
                        sub_id.as_deref().unwrap_or("?")
                    ));
                }
            }
        }
    }

    // Second pass: dangling references and cycles (needs the full ID sets).
    for (from, deps) in &task_edges {
        for dep in deps {
            if !task_ids.contains(dep) {
                violations.push(format!("task {from} depends on missing task {dep}"));
            }
        }
    }
    for (from, deps) in &subtask_task_edges {
        for dep in deps {
            if !task_ids.contains(dep) {
                violations.push(format!("subtask {from} depends on missing task {dep}"));
            }
        }
    }
    for (from, deps) in &subtask_edges {
        for dep in deps {
            if !subtask_ids.contains(dep) {
                violations.push(format!("subtask {from} depends on missing subtask {dep}"));
            }
        }
    }
    // One unified graph for cycles: task nodes are "t:<id>", subtask
    // nodes "s:<id>". A subtask naming a task is an edge into that
    // task's node, so a cycle spanning tasks and subtasks is caught.
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();
    for (from, deps) in &task_edges {
        for dep in deps {
            graph
                .entry(format!("t:{from}"))
                .or_default()
                .push(format!("t:{dep}"));
        }
    }
    for (from, deps) in &subtask_task_edges {
        for dep in deps {
            graph
                .entry(format!("s:{from}"))
                .or_default()
                .push(format!("t:{dep}"));
        }
    }
    for (from, deps) in &subtask_edges {
        for dep in deps {
            graph
                .entry(format!("s:{from}"))
                .or_default()
                .push(format!("s:{dep}"));
        }
    }
    for cycle in find_cycles(&graph) {
        violations.push(format!("dependency cycle: {}", cycle.join(" → ")));
    }

    violations
}

/// Validate the task list at `path`. Returns the violations, or an `Err`
/// describing why the file could not be read.
pub fn validate_task_list_file(path: &Path) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(validate_task_list(&text))
}

/// Normalize a subtask ID (string `"154.1"` or numeric `238.1`) to the
/// string form used for uniqueness and dependency resolution.
fn normalize_id(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Find cycles in a dependency graph (`from → [deps]`). Returns each
/// cycle as the node sequence from the first repeated node. Start nodes
/// are visited in sorted order, so the reported cycles (and their
/// order) are deterministic across runs — `HashMap` iteration order
/// is not.
fn find_cycles<T>(edges: &HashMap<T, Vec<T>>) -> Vec<Vec<T>>
where
    T: Eq + std::hash::Hash + Clone + Ord,
{
    let mut cycles = Vec::new();
    // 0 = unvisited, 1 = on the current DFS stack, 2 = finished.
    let mut state: HashMap<&T, u8> = HashMap::new();
    let mut stack: Vec<T> = Vec::new();
    let mut nodes: Vec<&T> = edges.keys().collect();
    nodes.sort();
    for node in nodes {
        if state.get(node).copied().unwrap_or(0) == 0 {
            dfs_cycle(node, edges, &mut state, &mut stack, &mut cycles);
        }
    }
    cycles
}

fn dfs_cycle<'a, T>(
    node: &'a T,
    edges: &'a HashMap<T, Vec<T>>,
    state: &mut HashMap<&'a T, u8>,
    stack: &mut Vec<T>,
    cycles: &mut Vec<Vec<T>>,
) where
    T: Eq + std::hash::Hash + Clone,
{
    state.insert(node, 1);
    stack.push(node.clone());
    if let Some(deps) = edges.get(node) {
        for dep in deps {
            match state.get(dep).copied().unwrap_or(0) {
                0 => dfs_cycle(dep, edges, state, stack, cycles),
                1 => {
                    // Back edge: the cycle is the stack suffix from dep.
                    if let Some(pos) = stack.iter().position(|n| n == dep) {
                        let mut cycle: Vec<T> = stack[pos..].to_vec();
                        cycle.push(dep.clone());
                        cycles.push(cycle);
                    }
                }
                _ => {}
            }
        }
    }
    stack.pop();
    state.insert(node, 2);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_task(id: i64, status: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "title": format!("task {id}"),
            "status": status,
            "subtasks": [
                {"id": format!("{id}.1"), "status": "done", "title": "Error checking: x"},
                {"id": format!("{id}.2"), "status": "done", "title": "Cargo test coverage: x"},
                {"id": format!("{id}.3"), "status": "done", "title": "Documentation: x"}
            ]
        })
    }

    fn list_with(tasks: Vec<serde_json::Value>) -> String {
        serde_json::json!({ "tasks": tasks }).to_string()
    }

    #[test]
    fn accepts_clean_minimal_list() {
        let json = list_with(vec![minimal_task(1, "done"), minimal_task(2, "pending")]);
        assert!(validate_task_list(&json).is_empty(), "{}", json);
    }

    #[test]
    fn rejects_bad_json() {
        let violations = validate_task_list("{ not json");
        assert!(
            violations.iter().any(|v| v.contains("invalid JSON")),
            "{violations:?}"
        );
    }

    #[test]
    fn rejects_duplicate_task_id() {
        let json = list_with(vec![minimal_task(1, "done"), minimal_task(1, "pending")]);
        let violations = validate_task_list(&json);
        assert!(
            violations.iter().any(|v| v.contains("duplicate task id 1")),
            "{violations:?}"
        );
    }

    #[test]
    fn rejects_dangling_task_dependency() {
        let mut task = minimal_task(154, "done");
        task["dependencies"] = serde_json::json!([153]);
        let violations = validate_task_list(&list_with(vec![task]));
        assert!(
            violations
                .iter()
                .any(|v| v.contains("depends on missing task 153")),
            "{violations:?}"
        );
    }

    #[test]
    fn rejects_circular_task_dependency() {
        let mut a = minimal_task(1, "pending");
        a["dependencies"] = serde_json::json!([2]);
        let mut b = minimal_task(2, "pending");
        b["dependencies"] = serde_json::json!([1]);
        let violations = validate_task_list(&list_with(vec![a, b]));
        assert!(
            violations
                .iter()
                .any(|v| v.contains("dependency cycle: t:1")),
            "{violations:?}"
        );
    }

    #[test]
    fn rejects_bad_status() {
        let violations = validate_task_list(&list_with(vec![minimal_task(1, "finished")]));
        assert!(
            violations.iter().any(|v| v.contains("invalid status")),
            "{violations:?}"
        );
    }

    #[test]
    fn rejects_missing_mandatory_subtask() {
        let mut task = minimal_task(1, "pending");
        task["subtasks"] = serde_json::json!([
            {"id": "1.1", "status": "pending", "title": "Error checking: x"}
        ]);
        let violations = validate_task_list(&list_with(vec![task]));
        assert!(
            violations
                .iter()
                .any(|v| v.contains("missing mandatory subtask \"cargo test coverage\"")),
            "{violations:?}"
        );
        assert!(
            violations
                .iter()
                .any(|v| v.contains("missing mandatory subtask \"documentation\"")),
            "{violations:?}"
        );
    }

    #[test]
    fn rejects_done_task_with_open_subtasks() {
        let mut task = minimal_task(1, "done");
        task["subtasks"] = serde_json::json!([
            {"id": "1.1", "status": "done", "title": "Error checking: x"},
            {"id": "1.2", "status": "pending", "title": "Cargo test coverage: x"},
            {"id": "1.3", "status": "done", "title": "Documentation: x"}
        ]);
        let violations = validate_task_list(&list_with(vec![task]));
        assert!(
            violations
                .iter()
                .any(|v| v.contains("done task 1 has open subtask 1.2")),
            "{violations:?}"
        );
    }

    #[test]
    fn rejects_dangling_subtask_dependency() {
        let mut task = minimal_task(1, "pending");
        task["subtasks"] = serde_json::json!([
            {"id": "1.1", "status": "pending", "title": "Error checking: x", "dependencies": ["1.9"]},
            {"id": "1.2", "status": "pending", "title": "Cargo test coverage: x"},
            {"id": "1.3", "status": "pending", "title": "Documentation: x"}
        ]);
        let violations = validate_task_list(&list_with(vec![task]));
        assert!(
            violations
                .iter()
                .any(|v| v.contains("depends on missing subtask 1.9")),
            "{violations:?}"
        );
    }

    #[test]
    fn rejects_circular_subtask_dependency() {
        let mut task = minimal_task(1, "pending");
        task["subtasks"] = serde_json::json!([
            {"id": "1.1", "status": "pending", "title": "Error checking: x", "dependencies": ["1.2"]},
            {"id": "1.2", "status": "pending", "title": "Cargo test coverage: x", "dependencies": ["1.1"]},
            {"id": "1.3", "status": "pending", "title": "Documentation: x"}
        ]);
        let violations = validate_task_list(&list_with(vec![task]));
        assert!(
            violations
                .iter()
                .any(|v| v.contains("dependency cycle: s:1.1")),
            "{violations:?}"
        );
    }

    #[test]
    fn accepts_cross_task_subtask_dependency() {
        // Subtask deps may cross tasks (live shape: 267.2 depends on 266.4).
        let mut a = minimal_task(266, "done");
        a["subtasks"] = serde_json::json!([
            {"id": "266.1", "status": "done", "title": "Error checking: x"},
            {"id": "266.2", "status": "done", "title": "Cargo test coverage: x"},
            {"id": "266.3", "status": "done", "title": "Documentation: x"},
            {"id": "266.4", "status": "done", "title": "Extra step"}
        ]);
        let mut b = minimal_task(267, "pending");
        b["subtasks"] = serde_json::json!([
            {"id": "267.1", "status": "pending", "title": "Error checking: x"},
            {"id": "267.2", "status": "pending", "title": "Cargo test coverage: x", "dependencies": ["266.4"]},
            {"id": "267.3", "status": "pending", "title": "Documentation: x"}
        ]);
        assert!(validate_task_list(&list_with(vec![a, b])).is_empty());
    }

    #[test]
    fn accepts_subtask_dependency_on_task() {
        // Subtask deps may name a whole task (live shape: 218.1 depends
        // on tasks 214 and 215). An integer dep resolves against tasks.
        let a = minimal_task(214, "done");
        let mut b = minimal_task(218, "pending");
        b["subtasks"] = serde_json::json!([
            {"id": "218.1", "status": "pending", "title": "Error checking: x", "dependencies": [214]},
            {"id": "218.2", "status": "pending", "title": "Cargo test coverage: x"},
            {"id": "218.3", "status": "pending", "title": "Documentation: x"}
        ]);
        assert!(validate_task_list(&list_with(vec![a, b])).is_empty());
    }

    #[test]
    fn rejects_subtask_dependency_on_missing_task() {
        let mut b = minimal_task(218, "pending");
        b["subtasks"] = serde_json::json!([
            {"id": "218.1", "status": "pending", "title": "Error checking: x", "dependencies": [999]},
            {"id": "218.2", "status": "pending", "title": "Cargo test coverage: x"},
            {"id": "218.3", "status": "pending", "title": "Documentation: x"}
        ]);
        let violations = validate_task_list(&list_with(vec![b]));
        assert!(
            violations
                .iter()
                .any(|v| v.contains("subtask 218.1 depends on missing task 999")),
            "{violations:?}"
        );
    }

    #[test]
    fn numeric_subtask_ids_normalize() {
        // The file mixes string "154.1" and numeric 238.1 subtask IDs;
        // numeric IDs normalize to the same string form for deps.
        let mut task = minimal_task(238, "pending");
        task["subtasks"] = serde_json::json!([
            {"id": 238.1, "status": "pending", "title": "Error checking: x"},
            {"id": 238.2, "status": "pending", "title": "Cargo test coverage: x", "dependencies": [238.1]},
            {"id": 238.3, "status": "pending", "title": "Documentation: x"}
        ]);
        assert!(validate_task_list(&list_with(vec![task])).is_empty());
    }
}
