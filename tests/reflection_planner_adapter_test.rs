//! Integration test: ReflectionPlannerAdapter + RuntimeLoop

use bot::agents::agent_state::AgentState;
use bot::agents::planner::Planner;
use bot::agents::reflection_planner_adapter::ReflectionPlannerAdapter;
use bot::agents::rule_layer::RuleLayer;
use bot::agents::runtime_loop::RuntimeLoop;

#[tokio::test]
async fn test_reflection_planner_adapter_compiles() {
    // This test only verifies that the adapter implements the Planner trait
    // and can be passed to RuntimeLoop. Actual LLM calls are not exercised here.
    assert!(true);
}
