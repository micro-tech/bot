//! LLM action path — routes LLM calls through the existing router.

use crate::agents::agent_step::LLMCallSpec;
use log::info;

pub async fn execute_llm_call(spec: &LLMCallSpec) -> Result<String, String> {
    // Placeholder: in real implementation this would call the Router
    info!("Executing LLM call to backend={} (corr_id={})", spec.backend, spec.correlation_id);
    Ok(format!("LLM response from {}", spec.backend))
}
