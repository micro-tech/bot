use super::genome::WorkflowGenome;
use serde::{Deserialize, Serialize};

/// Metrics collected during workflow execution.
/// The CPU will populate this after running a workflow.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionMetrics {
    pub latency_ms: u64,
    pub llm_calls: u32,
    pub skill_calls: u32,
    pub memory_ops: u32,
    pub bus_ops: u32,
    pub errors: u32,
    pub success: bool,
    pub success_count: u32, // Added for compatibility
    pub user_score: Option<f32>,
}

/// Configurable scoring weights.
/// You can expose these in config.toml later.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoringWeights {
    pub latency_weight: f32,
    pub llm_weight: f32,
    pub error_weight: f32,
    pub success_bonus: f32,
    pub user_score_weight: f32,
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            latency_weight: -0.001, // lower latency = better
            llm_weight: -1.0,       // fewer LLM calls = better
            error_weight: -5.0,     // errors heavily penalized
            success_bonus: 10.0,    // successful workflows rewarded
            user_score_weight: 2.0, // user feedback matters
        }
    }
}

/// Main scoring engine
pub struct ScoringEngine {
    pub weights: ScoringWeights,
}

impl ScoringEngine {
    pub fn new(weights: ScoringWeights) -> Self {
        Self { weights }
    }

    /// Compute a fitness score for a workflow based on execution metrics.
    pub fn score(&self, genome: &mut WorkflowGenome, metrics: &ExecutionMetrics) -> f64 {
        let mut score = 0.0;

        // Lower latency = higher score
        score += (metrics.latency_ms as f32 * self.weights.latency_weight) as f64;

        // Penalize LLM calls (expensive)
        score += (metrics.llm_calls as f32 * self.weights.llm_weight) as f64;

        // Penalize errors heavily
        score += (metrics.errors as f32 * self.weights.error_weight) as f64;

        // Reward success
        if metrics.success {
            score += self.weights.success_bonus as f64;
        }

        // User feedback (thumbs up/down)
        if let Some(user_score) = metrics.user_score {
            score += (user_score * self.weights.user_score_weight) as f64;
        }

        // Store score in genome
        genome.score = score;

        score
    }
}
