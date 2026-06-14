// router/tui.rs - Lightweight TUI inspector (Task 147.6)

use crate::router::debug::DecisionTree;
use std::io::{self, Write};

/// Simple text-based interactive inspector
pub fn run_simple_tui(tree: &DecisionTree) {
    println!("\n=== LLM Router Debug Inspector ===");
    println!("Prompt: {}", tree.prompt);
    println!("Timestamp: {}", tree.timestamp);
    println!("Final Backend: {:?}", tree.final_backend);
    println!("Latency: {} ms", tree.latency_ms);
    println!("Complexity Score: {:.2}", tree.complexity_score);
    println!("\n--- Decision Steps ---");

    for step in &tree.steps {
        println!("{}. {}", step.step, step.description);
    }

    println!("\nPress Enter to exit...");
    let mut input = String::new();
    let _ = io::stdin().read_line(&mut input);
}

/// Print a compact one-line summary (useful for logs)
pub fn print_compact_summary(tree: &DecisionTree) {
    println!(
        "[ROUTER] {:?} | {}ms | score={:.2} | prompt=\"{:.40}...\"",
        tree.final_backend,
        tree.latency_ms,
        tree.complexity_score,
        tree.prompt
    );
}