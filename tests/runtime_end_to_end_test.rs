//! End-to-end smoke test for the Unified Agent Runtime (Task 154.4)

use bot::agents::simple_planner::SimplePlanner;
use bot::agents::runtime_loop::RuntimeLoop;
use bot::agents::rule_layer::RuleLayer;
use bot::agents::agent_state::AgentState;

#[tokio::test]
async fn test_end_to_end_runtime_with_trace() {
    let planner = SimplePlanner::new("List all files in current directory".to_string());
    let rules = RuleLayer::new(vec!["noop".into()]);
    
    // Enable trace logging
    let runtime = RuntimeLoop::new(planner, rules, 5).with_trace(true);

    let state = AgentState::new();
    let final_state = runtime.run(state).await;

    assert!(final_state.halted);
    assert!(final_state.step_count >= 1);
    println!("✅ End-to-end runtime test passed with trace enabled");
}
