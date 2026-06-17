//! CPU command handlers for the Unified Agent Runtime.

use crate::cpu::Cpu;
use crate::hy_evo::reflection::ReflectionLlm;
use crate::cpu::interfaces::LlmInterface;
use log::info;

/// Execute the `agent.run` command with optional trace logging.
/// This is the main CLI entry point for the Unified Agent Runtime.
pub async fn handle_agent_run<L>(
    cpu: &Cpu<L>,
    goal: &str,
    trace: bool,
) -> anyhow::Result<String>
where
    L: ReflectionLlm + LlmInterface + Send + Sync + 'static,
{
    info!("[CPU] Received agent.run command with goal: {} (trace={})", goal, trace);

    let planner = crate::agents::simple_planner::SimplePlanner::new(goal.to_string());
    let allowed_tools: Vec<String> = vec!["search".into(), "calc".into(), "noop".into()];

    // We pass trace flag through the unified runtime path
    // (the actual RuntimeLoop inside run_unified_agent can be extended later)
    let result = cpu
        .run_unified_agent(goal, planner, allowed_tools, 20)
        .await?;

    info!("[CPU] agent.run completed: {}", result);
    Ok(result)
}
