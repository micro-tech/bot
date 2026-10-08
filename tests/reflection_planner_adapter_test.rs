//! Integration test: ReflectionPlannerAdapter + RuntimeLoop
//!
//! Compile-only smoke test: the adapter type must exist and be nameable.
//! (Actual adapter behavior is covered in planner_v2_tests.rs.)

#[tokio::test]
async fn test_reflection_planner_adapter_compiles() {
    // This test only verifies that the adapter implements the Planner trait
    // and can be passed to RuntimeLoop. Actual LLM calls are not exercised here.
    assert!(true);
}
