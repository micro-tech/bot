//! Unit + integration tests for PlannerOutput + ReflectionPlannerAdapter (Task 155.5)

use bot::agents::planner_output::PlannerOutput;
use bot::agents::planner::Planner;
use bot::agents::reflection_planner_adapter::ReflectionPlannerAdapter;
use bot::agents::agent_state::AgentState;

#[tokio::test]
async fn test_planner_output_variants() {
    // ToolCall
    let tc = PlannerOutput::ToolCall {
        tool_name: "search".into(),
        args_json: serde_json::json!({"q": "rust"}),
        reasoning: None,
    };
    assert!(matches!(tc, PlannerOutput::ToolCall { .. }));

    // LLMCall
    let llm = PlannerOutput::LLMCall {
        prompt: "hello".into(),
        reasoning: Some("test".into()),
    };
    assert!(matches!(llm, PlannerOutput::LLMCall { .. }));

    // FinalAnswer
    let fa = PlannerOutput::FinalAnswer {
        message: "done".into(),
        reasoning: None,
    };
    assert!(matches!(fa, PlannerOutput::FinalAnswer { .. }));

    // Error
    let err = PlannerOutput::Error { message: "fail".into() };
    assert!(matches!(err, PlannerOutput::Error { .. }));
}

#[tokio::test]
async fn test_reflection_planner_adapter_parsing() {
    // Create a dummy inner planner (we only test the adapter's parsing layer)
    struct Dummy;
    let adapter = ReflectionPlannerAdapter::new(Dummy);

    let mut state = AgentState::new();
    state.messages.push("use tool: calc {\"x\": 2}".into());

    let decision = adapter.decide(&state).await;
    assert!(matches!(decision, PlannerOutput::ToolCall { .. }));
}

#[tokio::test]
async fn test_reflection_planner_adapter_final_answer() {
    struct Dummy;
    let adapter = ReflectionPlannerAdapter::new(Dummy);

    let mut state = AgentState::new();
    state.messages.push("final answer: task complete".into());

    let decision = adapter.decide(&state).await;
    match decision {
        PlannerOutput::FinalAnswer { message, .. } => {
            assert!(message.contains("task complete"));
        }
        _ => panic!("expected FinalAnswer"),
    }
}
