// router/debug.rs - Debug, Inspection & Explainability Layer (Task 147)

use crate::router::context::{RoutingContext, LLMBackend};
use crate::router::config::RouterConfig;
use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};

/// A single step in the routing decision explanation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplainStep {
    pub step: u32,
    pub description: String,
    pub data: Option<serde_json::Value>,
}

/// Full decision tree for explainability and visualization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionTree {
    pub prompt: String,
    pub timestamp: DateTime<Utc>,
    pub steps: Vec<ExplainStep>,
    pub final_backend: LLMBackend,
    pub latency_ms: u64,
    pub complexity_score: f32,
    pub override_used: Option<LLMBackend>,
    pub health_status: Option<String>,
    pub telemetry_snapshot: Option<String>,
}

/// Machine-readable visualization output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VizJSON {
    pub decision_tree: DecisionTree,
    pub mermaid_diagram: Option<String>,
}

/// Generate a human-readable explanation for a routing decision
pub fn generate_explain(
    ctx: &RoutingContext,
    backend: LLMBackend,
    latency_ms: u64,
) -> DecisionTree {
    let mut steps = Vec::new();
    let mut step_num = 1;

    // Step 1: Prompt analysis
    steps.push(ExplainStep {
        step: step_num,
        description: format!("Analyzed prompt ({} tokens, code: {})", ctx.token_estimate, ctx.has_code),
        data: Some(serde_json::json!({
            "prompt_length": ctx.prompt.len(),
            "token_estimate": ctx.token_estimate,
            "has_code": ctx.has_code
        })),
    });
    step_num += 1;

    // Step 2: Complexity
    steps.push(ExplainStep {
        step: step_num,
        description: format!("Complexity score calculated: {:.2}", ctx.complexity_score),
        data: Some(serde_json::json!({ "complexity_score": ctx.complexity_score })),
    });
    step_num += 1;

    // Step 3: Override check
    if let Some(ov) = ctx.user_override {
        steps.push(ExplainStep {
            step: step_num,
            description: format!("User override applied: {:?}", ov),
            data: Some(serde_json::json!({ "override": format!("{:?}", ov) })),
        });
        step_num += 1;
    }

    // Step 4: Health / Telemetry influence
    if let Some(health) = &ctx.health {
        steps.push(ExplainStep {
            step: step_num,
            description: "Health status considered during selection".to_string(),
            data: Some(serde_json::json!({ "health_backends": health.len() })),
        });
        step_num += 1;
    }

    if let Some(tele) = &ctx.telemetry {
        steps.push(ExplainStep {
            step: step_num,
            description: format!("Telemetry: GPU {:.1}%, VRAM {:.1}GB", tele.gpu_util, tele.vram_used),
            data: Some(serde_json::json!({
                "gpu_util": tele.gpu_util,
                "vram_used": tele.vram_used
            })),
        });
        step_num += 1;
    }

    // Final decision
    steps.push(ExplainStep {
        step: step_num,
        description: format!("Selected backend: {:?}", backend),
        data: Some(serde_json::json!({ "selected_backend": format!("{:?}", backend) })),
    });

    DecisionTree {
        prompt: ctx.prompt.clone(),
        timestamp: ctx.timestamp,
        steps,
        final_backend: backend,
        latency_ms,
        complexity_score: ctx.complexity_score,
        override_used: ctx.user_override,
        health_status: ctx.health.as_ref().map(|h| format!("{} backends checked", h.len())),
        telemetry_snapshot: ctx.telemetry.as_ref().map(|t| format!("GPU: {:.0}%", t.gpu_util)),
    }
}

/// Generate machine-readable visualization JSON
pub fn generate_viz_json(tree: DecisionTree) -> VizJSON {
    VizJSON {
        decision_tree: tree,
        mermaid_diagram: None, // Can be generated later
    }
}

/// Redact sensitive information from a DecisionTree (production safety)
pub fn redact_sensitive(mut tree: DecisionTree) -> DecisionTree {
    // Redact the actual prompt text
    tree.prompt = "[REDACTED]".to_string();
    tree
}