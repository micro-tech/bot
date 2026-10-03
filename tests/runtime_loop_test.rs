//! Integration test for the Unified Agent Runtime Loop.

use bot::agents::agent_state::AgentState;
use bot::agents::planner::Planner;
use bot::agents::planner_output::PlannerOutput;
use bot::agents::rule_layer::RuleLayer;
use bot::agents::runtime_loop::RuntimeLoop;
use async_trait::async_trait;

struct DummyPlanner;

#[async_trait]
impl Planner for DummyPlanner {
    async fn decide(&self, _state: &AgentState) -> PlannerOutput {
        PlannerOutput::FinalAnswer {
            message: "done".to_string(),
            reasoning: None,
        }
    }
}

#[tokio::test]
async fn test_runtime_loop_terminates() {
    let planner = DummyPlanner;
    let rules = RuleLayer::new(vec![]);
    let mut runtime = RuntimeLoop::new(planner, rules, 10);

    let state = AgentState::new();
    let final_state = runtime.run(state).await;

    assert!(final_state.halted);
    assert!(final_state.step_count >= 1);
}
