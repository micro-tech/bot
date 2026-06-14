// router/complexity.rs - Full Complexity Scoring Engine (Task 128)
use regex::Regex;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct ComplexityFeatures {
    pub token_count: usize,
    pub code_density: f32,
    pub reasoning_keywords: f32,
    pub math_symbols: f32,
    pub multi_step: f32,
    pub structural_depth: f32,
}

/// Trait for pluggable heuristic modules
pub trait Heuristic: Send + Sync {
    fn name(&self) -> &str;
    fn score(&self, prompt: &str) -> f32;
}

/// Registry for dynamic heuristic registration
pub struct HeuristicRegistry {
    heuristics: HashMap<String, Box<dyn Heuristic>>,
    weights: HashMap<String, f32>,
}

impl HeuristicRegistry {
    pub fn new() -> Self {
        Self {
            heuristics: HashMap::new(),
            weights: HashMap::new(),
        }
    }

    pub fn register(&mut self, h: Box<dyn Heuristic>, weight: f32) {
        let name = h.name().to_string();
        self.heuristics.insert(name.clone(), h);
        self.weights.insert(name, weight);
    }

    pub fn score(&self, prompt: &str) -> f32 {
        let mut total = 0.0;
        let mut weight_sum = 0.0;

        for (name, heuristic) in &self.heuristics {
            if let Some(&w) = self.weights.get(name) {
                total += heuristic.score(prompt) * w;
                weight_sum += w;
            }
        }

        if weight_sum > 0.0 {
            (total / weight_sum).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

// ============== Core Heuristic Implementations ==============

pub struct TokenCountHeuristic;
impl Heuristic for TokenCountHeuristic {
    fn name(&self) -> &str { "token_count" }
    fn score(&self, prompt: &str) -> f32 {
        (prompt.split_whitespace().count() as f32 / 4000.0).clamp(0.0, 1.0)
    }
}

pub struct CodeDensityHeuristic;
impl Heuristic for CodeDensityHeuristic {
    fn name(&self) -> &str { "code_density" }
    fn score(&self, prompt: &str) -> f32 {
        let code_blocks = prompt.matches("```").count() as f32 / 2.0;
        (code_blocks * 0.4).clamp(0.0, 1.0)
    }
}

pub struct ReasoningKeywordsHeuristic;
impl Heuristic for ReasoningKeywordsHeuristic {
    fn name(&self) -> &str { "reasoning_keywords" }
    fn score(&self, prompt: &str) -> f32 {
        let keywords = ["explain", "step-by-step", "prove", "why", "how", "analyze", "compare"];
        let count = keywords.iter().filter(|&w| prompt.to_lowercase().contains(w)).count();
        (count as f32 / keywords.len() as f32).clamp(0.0, 1.0)
    }
}

pub struct MathSymbolsHeuristic;
impl Heuristic for MathSymbolsHeuristic {
    fn name(&self) -> &str { "math_symbols" }
    fn score(&self, prompt: &str) -> f32 {
        let symbols = prompt.matches(|c: char| "+-*/=^√∫∑".contains(c)).count();
        (symbols as f32 / 25.0).clamp(0.0, 1.0)
    }
}

pub struct MultiStepHeuristic;
impl Heuristic for MultiStepHeuristic {
    fn name(&self) -> &str { "multi_step" }
    fn score(&self, prompt: &str) -> f32 {
        let lines = prompt.matches('\n').count();
        let numbered = prompt.matches(|c: char| c.is_ascii_digit()).count();
        ((lines + numbered) as f32 / 20.0).clamp(0.0, 1.0)
    }
}

pub struct StructuralDepthHeuristic;
impl Heuristic for StructuralDepthHeuristic {
    fn name(&self) -> &str { "structural_depth" }
    fn score(&self, prompt: &str) -> f32 {
        let nested_lists = prompt.matches("  -").count() + prompt.matches("    ").count();
        (nested_lists as f32 / 15.0).clamp(0.0, 1.0)
    }
}

/// Convenience function to build default registry with sensible weights
pub fn default_registry() -> HeuristicRegistry {
    let mut reg = HeuristicRegistry::new();
    reg.register(Box::new(TokenCountHeuristic), 0.20);
    reg.register(Box::new(CodeDensityHeuristic), 0.25);
    reg.register(Box::new(ReasoningKeywordsHeuristic), 0.20);
    reg.register(Box::new(MathSymbolsHeuristic), 0.15);
    reg.register(Box::new(MultiStepHeuristic), 0.10);
    reg.register(Box::new(StructuralDepthHeuristic), 0.10);
    reg
}

pub fn score_prompt(prompt: &str) -> f32 {
    default_registry().score(prompt)
}
