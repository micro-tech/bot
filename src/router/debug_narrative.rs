// router/debug_narrative.rs - Human-readable narrative generator (Task 147.2)

use crate::router::debug::DecisionTree;

/// Generate a clean, human-readable explanation narrative
pub fn generate_narrative(tree: &DecisionTree) -> String {
    let mut output = String::new();

    output.push_str(&format!("Routing Decision for prompt at {}\n", tree.timestamp));
    output.push_str(&format!("Prompt: {}\n\n", tree.prompt));

    for step in &tree.steps {
        output.push_str(&format!("{}. {}\n", step.step, step.description));
    }

    output.push_str(&format!("\n→ Final selection: {:?}\n", tree.final_backend));
    output.push_str(&format!("   Latency: {} ms | Complexity: {:.2}\n", tree.latency_ms, tree.complexity_score));

    if let Some(ov) = tree.override_used {
        output.push_str(&format!("   (User override was active: {:?})\n", ov));
    }

    output
}