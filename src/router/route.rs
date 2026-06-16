// router/route.rs - Core deterministic routing logic (Task 129 + 146)

use crate::router::{LLMBackend, RoutingContext, RouterConfig};

/// Main routing decision function.
/// This is the heart of the LLM Router.
pub fn route(ctx: &RoutingContext, config: &RouterConfig) -> LLMBackend {
    // 1. User override always wins
    if let Some(backend) = &ctx.user_override {
        return backend.clone();
    }

    // 2. Check schedule windows first (highest priority after override)
    if let Some(profile) = check_schedule(ctx, config) {
        if let Some(backend) = select_from_profile(&profile, ctx) {
            return backend;
        }
    }

    // 3. Complexity-based routing
    let complexity = if ctx.complexity_score > 0.0 {
        ctx.complexity_score
    } else {
        calculate_complexity(ctx)
    };

    // 4. Apply load & health awareness (simplified for now)
    if complexity >= config.complexity.global_threshold {
        // High complexity → prefer stronger models
        if ctx.has_code || complexity > 0.85 {
            return LLMBackend::Grok;
        }
        return LLMBackend::Gemini;
    }

    // 5. Default to local Ollama for everything else
    LLMBackend::LocalOllama
}

fn check_schedule(ctx: &RoutingContext, config: &RouterConfig) -> Option<String> {
    let now = ctx.timestamp;
    let hour = now.hour();

    for window in &config.schedule.windows {
        if hour >= window.start_hour && hour < window.end_hour {
            if let Some(days) = &window.days {
                let day_name = now.weekday().to_string();
                if days.iter().any(|d| d.eq_ignore_ascii_case(&day_name)) {
                    return Some(window.profile.clone());
                }
            } else {
                return Some(window.profile.clone());
            }
        }
    }
    None
}

fn select_from_profile(profile_name: &str, ctx: &RoutingContext) -> Option<LLMBackend> {
    // Simple profile mapping - can be extended
    match profile_name.to_lowercase().as_str() {
        "high_quality" | "reasoning" => Some(LLMBackend::Grok),
        "fast" | "local" => Some(LLMBackend::LocalOllama),
        "balanced" => {
            if ctx.has_code {
                Some(LLMBackend::Gemini)
            } else {
                Some(LLMBackend::LocalOllama)
            }
        }
        _ => None,
    }
}

fn calculate_complexity(ctx: &RoutingContext) -> f32 {
    let mut score = 0.0;

    // Token weight
    score += (ctx.token_estimate as f32 / 4000.0).min(1.0) * 0.25;

    // Code presence
    if ctx.has_code {
        score += 0.30;
    }

    // Base complexity from context
    score += ctx.complexity_score * 0.45;

    score.clamp(0.0, 1.0)
}

// Re-export for convenience
pub use crate::router::complexity::score_prompt;
