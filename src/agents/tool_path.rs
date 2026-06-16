//! Tool action path — executes tools via the Tool Supervisor.

use crate::agents::agent_step::ToolInvocation;
use log::info;

pub async fn execute_tool_call(inv: &ToolInvocation) -> Result<serde_json::Value, String> {
    info!("Executing tool '{}' (corr_id={})", inv.name, inv.correlation_id);
    // Placeholder — real implementation calls ToolSupervisor
    Ok(serde_json::json!({"status": "ok", "tool": inv.name}))
}
