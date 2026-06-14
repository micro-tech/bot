// router/complexity.rs - Complexity Scoring Engine (task 145)
use regex::Regex;

#[derive(Debug, Clone)]
pub struct ComplexityFeatures {
    pub token_count: usize,
    pub code_density: f32,
    pub reasoning_keywords: f32,
    pub math_symbols: f32,
    pub multi_step: f32,
    pub structural_depth: f32,
}

pub fn extract_features(prompt: &str) -> ComplexityFeatures {
    let token_count = prompt.split_whitespace().count();
    let code_blocks = prompt.matches("```").count() as f32 / 2.0;
    let code_density = (code_blocks * 0.3).min(1.0);

    let reasoning = ["explain", "step-by-step", "prove", "why", "how"].iter()
        .filter(|&w| prompt.to_lowercase().contains(w))
        .count() as f32 / 5.0;

    let math = prompt.matches(|c: char| "+-*/=^".contains(c)).count() as f32 / 20.0;

    let multi_step = prompt.matches(|c| c == '\n').count() as f32 / 10.0;

    ComplexityFeatures {
        token_count,
        code_density,
        reasoning_keywords: reasoning.min(1.0),
        math_symbols: math.min(1.0),
        multi_step: multi_step.min(1.0),
        structural_depth: 0.5, // placeholder
    }
}

pub fn score(features: &ComplexityFeatures) -> f32 {
    let score = 0.3 * features.code_density
        + 0.25 * features.reasoning_keywords
        + 0.2 * features.math_symbols
        + 0.15 * features.multi_step
        + 0.1 * (features.token_count as f32 / 4000.0).min(1.0);
    score.min(1.0)
}
