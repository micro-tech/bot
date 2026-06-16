//! CPU command handlers for the Unified Agent Runtime.

use crate::cpu::Cpu;
use crate::hy_evo::reflection::ReflectionLlm;
use crate::cpu::interfaces::LlmInterface;
use log::info;

/// Execute the `agent.run` command.
/// This is the simplest entry point to test the new runtime loop.
pub async fn handle_agent_run<L>(
    cpu: &Cpu<L>,
    goal: &str,
) -> anyhow::Result<String>
where
    L: ReflectionLlm + LlmInterface + Send + Sync + 'static,
{
    info!("[CPU] Received agent.run command with goal: {}", goal);

    // For now we use a dummy planner.
    // In a real implementation you would inject a proper planner.
    use crate::agents::planner::Planner;
    use crate::agents::agent_step::AgentStep;
    use async_trait::async_trait;

    struct DummyPlanner {
        goal: String,
    }

    impl DummyPlanner {
        fn new(goal: String) -> Self {
            Self { goal }
        }
    }

    #[async_trait]
    impl Planner for DummyPlanner {
        async fn decide(&self, _state: &crate::agents::agent_state::AgentState) -> AgentStep {
            AgentStep::FinalAnswer(format!("Completed goal: {}", self.goal))
        }
    }

    let planner = crate::agents::simple_planner::SimplePlanner::new(goal.to_string());
    let allowed_tools: Vec<String> = vec!["search".into(), "calc".into(), "noop".into()];
    let result = cpu
        .run_unified_agent(goal, planner, allowed_tools, 20)
        .await?;

    info!("[CPU] agent.run completed: {}", result);
    Ok(result)
}
