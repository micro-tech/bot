//! Long-term memory continuity helpers for the Unified Agent Runtime.
//!
//! This module provides the mechanisms that make the agent "remember" across
//! sessions — the key ingredient for Hermies/OpenClaw-style persistent identity.
//!
//! Core responsibilities:
//!   - Inject relevant episodic + vector memories at the start of a run
//!   - Define LessonLearned / ImportantOutcome memory record types
//!   - Persist key outcomes back to long-term memory after FinalAnswer

use crate::agents::agent_state::AgentState;
use crate::memory::MemoryManager;
use log::info;
use serde::{Deserialize, Serialize};


/// A lesson the agent learned from a previous run.
/// Stored in episodic memory and surfaced during future runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LessonLearned {
    pub goal: String,
    pub lesson: String,
    pub timestamp: String,
    pub confidence: f32, // 0.0 – 1.0
}

/// An important outcome / result from a previous agent run.
/// Used for cross-session continuity and "I remember doing X" behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportantOutcome {
    pub goal: String,
    pub result_summary: String,
    pub timestamp: String,
    pub tags: Vec<String>,
}

/// Inject relevant long-term memories into the working state at the beginning of a run.
/// This gives the agent immediate access to what it has learned before.
pub fn inject_long_term_context(state: &mut AgentState, memory: &MemoryManager, goal: &str) {
    // 1. Pull recent episodic memories
    let recent = memory.episodic.get_recent(5);
    if !recent.is_empty() {
        state.messages.push(format!(
            "[Memory] Recalled {} recent episodes",
            recent.len()
        ));
        for entry in recent.iter().take(3) {
            state.scratchpad.push(format!("Past: {}", entry));
        }
    }

    // 2. Semantic search in vector memory for related past experiences
    let related = memory.search_facts(goal, 4);
    if !related.is_empty() {
        state.messages.push(format!(
            "[Memory] Found {} semantically related past facts",
            related.len()
        ));
        for fact in related.iter().take(3) {
            state.scratchpad.push(format!("Related: {}", fact));
        }
    }

    // 3. Surface any stored beliefs that might be relevant
    if !memory.beliefs.is_empty() {
        let belief_count = memory.beliefs.len();
        state.messages.push(format!("[Memory] Loaded {} persistent beliefs", belief_count));
    }

    info!(
        "[MemoryContinuity] Injected long-term context for goal: {}",
        goal
    );
}

/// Create a LessonLearned record and push it as a memory delta.
pub fn record_lesson(state: &mut AgentState, goal: &str, lesson: &str, confidence: f32) {
    let lesson = LessonLearned {
        goal: goal.to_string(),
        lesson: lesson.to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        confidence,
    };

    if let Ok(json) = serde_json::to_value(&lesson) {
        state.push_memory_delta("lesson_learned".to_string(), json);
        info!("[Memory] Recorded lesson: {}", lesson.lesson);
    }
}

/// Create an ImportantOutcome record and push it as a memory delta.
pub fn record_important_outcome(state: &mut AgentState, goal: &str, summary: &str, tags: Vec<String>) {
    let outcome = ImportantOutcome {
        goal: goal.to_string(),
        result_summary: summary.to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        tags,
    };

    if let Ok(json) = serde_json::to_value(&outcome) {
        state.push_memory_delta("important_outcome".to_string(), json);
        info!("[Memory] Recorded important outcome for goal: {}", goal);
    }
}

/// Persist any pending memory deltas (lessons, outcomes, etc.) into the MemoryManager.
/// Called when the agent reaches FinalAnswer or halts.
pub fn persist_memory_deltas(state: &mut AgentState, memory: &mut MemoryManager) {
    if state.memory_deltas.is_empty() {
        return;
    }

    for (kind, value) in state.memory_deltas.drain(..) {
        match kind.as_str() {
            "lesson_learned" => {
                if let Ok(lesson) = serde_json::from_value::<LessonLearned>(value.clone()) {
                    memory.episodic.record(format!(
                        "LESSON [{}] {}",
                        lesson.goal, lesson.lesson
                    ));
                    // Also store in vector memory for semantic retrieval
                    let _ = memory.vector.add_fact(format!(
                        "Lesson about {}: {}",
                        lesson.goal, lesson.lesson
                    ));
                }
            }
            "important_outcome" => {
                if let Ok(outcome) = serde_json::from_value::<ImportantOutcome>(value.clone()) {
                    memory.episodic.record(format!(
                        "OUTCOME [{}] {}",
                        outcome.goal, outcome.result_summary
                    ));
                    let _ = memory.vector.add_fact(format!(
                        "Outcome for {}: {}",
                        outcome.goal, outcome.result_summary
                    ));
                }
            }
            _ => {
                // Generic belief or context
                memory.set_belief(&kind, value);
            }
        }
    }

    info!("[MemoryContinuity] Persisted memory deltas to long-term store");
}
