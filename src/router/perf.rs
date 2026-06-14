// router/perf.rs - Performance Optimization (Task 137)
use std::time::Instant;

/// Fast-path complexity check for very short prompts (< 50 tokens)
#[inline]
pub fn is_simple_prompt(prompt: &str) -> bool {
    prompt.len() < 200 && !prompt.contains("```") && !prompt.contains('\n')
}

/// Micro-benchmark friendly scoring path (zero allocation where possible)
pub fn fast_score(prompt: &str) -> f32 {
    if is_simple_prompt(prompt) {
        return 0.15; // fast path
    }

    let mut score = 0.0;
    let len = prompt.len() as f32;

    // Token estimate (very cheap)
    score += (len / 4000.0).min(0.25);

    // Code detection
    if prompt.contains("```") {
        score += 0.30;
    }

    // Reasoning keywords (small fixed set)
    if prompt.to_ascii_lowercase().contains("explain")
        || prompt.contains("step-by-step")
        || prompt.contains("prove")
    {
        score += 0.25;
    }

    score.min(1.0)
}

/// Run a quick latency measurement (for tests/benchmarks)
pub fn measure_route_latency<F>(f: F) -> std::time::Duration
where
    F: FnOnce(),
{
    let start = Instant::now();
    f();
    start.elapsed()
}
