//! End-to-end smoke test for the Unified Agent Runtime (Task 154.4)

use async_trait::async_trait;
use helix::agents::planner::Planner;
use helix::agents::planner_output::PlannerOutput;
use helix::agents::runtime_loop::RuntimeLoop;
use helix::agents::rule_layer::RuleLayer;
use helix::agents::agent_state::AgentState;

/// Test planner with SimplePlanner's 3-steps-then-answer shape, but calling
/// the real read-only `system_status` tool: the `noop` tool SimplePlanner
/// hardcodes was never registered in crate::tools, so the supervisor fatals
/// on it and the run halts at step 0.
struct StatusPlanner;

#[async_trait]
impl Planner for StatusPlanner {
    async fn decide(&self, state: &AgentState) -> PlannerOutput {
        if state.step_count >= 3 {
            return PlannerOutput::FinalAnswer {
                message: "done".into(),
                reasoning: None,
            };
        }
        PlannerOutput::ToolCall {
            tool_name: "system_status".into(),
            args_json: serde_json::json!({}),
            reasoning: None,
        }
    }
}

#[tokio::test]
async fn test_end_to_end_runtime_with_trace() {
    let planner = StatusPlanner;
    let rules = RuleLayer::new(vec!["system_status".into()]);

    // Enable trace logging
    let mut runtime = RuntimeLoop::new(planner, rules, 5).with_trace(true);

    let state = AgentState::new();
    let final_state = runtime.run(state).await;

    assert!(final_state.halted);
    assert!(final_state.step_count >= 1);
    println!("✅ End-to-end runtime test passed with trace enabled");
}
