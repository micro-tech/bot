// router/security.rs - Security & Validation (Task 138)
use crate::router::LLMBackend;

/// Capability flags a backend must support
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capability {
    ToolCalling,
    Vision,
    LongContext,
    FunctionCalling,
}

/// Validate that the chosen backend supports required capabilities
pub fn validate_capabilities(
    backend: &LLMBackend,
    required: &[Capability],
) -> Result<(), String> {
    // In a real system this would come from a capability registry
    let supported = match backend {
        LLMBackend::LocalOllama => vec![Capability::ToolCalling, Capability::LongContext],
        LLMBackend::LanOllama => vec![Capability::ToolCalling],
        LLMBackend::Gemini => vec![Capability::Vision, Capability::LongContext],
        LLMBackend::Grok => vec![Capability::ToolCalling, Capability::FunctionCalling],
        LLMBackend::Fallback => vec![],
    };

    for cap in required {
        if !supported.contains(cap) {
            return Err(format!(
                "Backend {:?} does not support capability {:?}",
                backend, cap
            ));
        }
    }
    Ok(())
}

/// Sanitize user-provided backend preference (prevent injection)
pub fn sanitize_backend_name(name: &str) -> Option<LLMBackend> {
    match name.to_lowercase().as_str() {
        "localollama" | "ollama" => Some(LLMBackend::LocalOllama),
        "lanollama" => Some(LLMBackend::LanOllama),
        "gemini" => Some(LLMBackend::Gemini),
        "grok" => Some(LLMBackend::Grok),
        _ => None,
    }
}
