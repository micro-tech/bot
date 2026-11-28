//! Task-list validator test (M28, task 415).
//!
//! Runs the [`helix::task_list`] validator against the real
//! `.zed/task_list.json` — this test must stay green, which is what
//! keeps the list from silently rotting again (the task-320 JSON
//! conflict, the dangling `154 → 153` dependency, and the open subtasks
//! under `done` tasks all predate the validator). The validator's own
//! rejection behaviour is unit-tested in `src/task_list.rs` against
//! deliberately broken lists; this file only pins the real list clean.
//!
//! Wired into CI via `.github/workflows/ci.yml`
//! ("Validate task list rules (M28)") and into the full suite via
//! `cargo test --all-targets`.

#[test]
fn task_list_is_clean() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".zed/task_list.json");
    let violations = helix::task_list::validate_task_list_file(&path)
        .unwrap_or_else(|e| panic!("task list unreadable: {e}"));
    assert!(
        violations.is_empty(),
        "task list has {} violation(s):\n{}",
        violations.len(),
        violations.join("\n")
    );
}
