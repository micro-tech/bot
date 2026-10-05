//! Long-term memory continuity helpers for the Unified Agent Runtime.
//!
//! This module defines the structured record types (`LessonLearned`,
//! `ImportantOutcome`) and the two hooks that move them between a run and
//! the `MemoryManager`:
//!
//! - `inject_long_term_context`, called by `Cpu::run_unified_agent` before a
//!   run: pulls recent episodic records, semantically related vector facts,
//!   and the belief count into the fresh `AgentState`.
//! - `record_lesson` / `record_important_outcome`, which stage typed deltas
//!   on the `AgentState` during (or after) a run, and `persist_memory_deltas`,
//!   called by `Cpu::run_unified_agent` when the run ends: each lesson and
//!   outcome becomes an episodic record plus a vector fact; anything else
//!   becomes a belief.
//!
//! The `MemoryManager` is disk-backed (`beliefs.json`, `episodes.jsonl`,
//! `vector_facts.json`), so records persisted here survive restarts — that
//! is the entire continuity mechanism. Nothing here invents identity; it
//! just files what happened where the next run can find it.

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
pub fn inject_long_term_context(state: &mut AgentState, memory: &mut MemoryManager, goal: &str) {
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

/// Create a LessonLearned record and stage it as a memory delta on the run
/// state. Staged deltas are written to the MemoryManager by
/// `persist_memory_deltas` when the run ends.
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

/// Create an ImportantOutcome record and stage it as a memory delta on the run
/// state. Staged deltas are written to the MemoryManager by
/// `persist_memory_deltas` when the run ends.
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
/// Called by `Cpu::run_unified_agent` when the agent run ends (success or halt).
/// Each lesson/outcome becomes an episodic record plus a vector fact; any other
/// delta kind is stored as a belief. The MemoryManager is disk-backed, so this
/// genuinely persists across restarts.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::episodic::EpisodicMemory;
    use crate::memory::vector::{Embedder, VectorMemory};
    use crate::memory::{MemoryHandle, MemoryManager};
    use std::collections::HashMap;

    /// Deterministic embedder: fixed 2-D vectors so search ranking is stable.
    struct StubEmbedder;
    impl Embedder for StubEmbedder {
        fn embed(&mut self, text: &str) -> Option<Vec<f32>> {
            if text.contains("rust") {
                Some(vec![1.0, 0.0])
            } else {
                Some(vec![0.0, 1.0])
            }
        }
    }

    fn test_memory() -> MemoryManager {
        MemoryManager {
            working: MemoryHandle::new(10),
            vector: VectorMemory::with_embedder(Box::new(StubEmbedder), None),
            episodic: EpisodicMemory::new(10),
            beliefs: HashMap::new(),
        }
    }

    #[test]
    fn inject_long_term_context_surfaces_episodes_and_facts() {
        // GIVEN a memory with an episode and a semantically matching fact:
        let mut memory = test_memory();
        memory.episodic.record("did the thing".to_string());
        memory
            .vector
            .add_fact("rust borrow checker insight".to_string());

        // WHEN context is injected for a related goal,
        // THEN episodes and related facts land in the run state:
        let mut state = AgentState::new();
        inject_long_term_context(&mut state, &mut memory, "rust lifetimes");
        assert!(state
            .messages
            .iter()
            .any(|m| m.contains("Recalled 1 recent episodes")));
        assert!(state
            .messages
            .iter()
            .any(|m| m.contains("semantically related past facts")));
        assert!(state.scratchpad.iter().any(|s| s.contains("did the thing")));
        assert!(state
            .scratchpad
            .iter()
            .any(|s| s.contains("borrow checker")));
    }

    #[test]
    fn record_and_persist_deltas_land_in_episodic_and_vector() {
        // GIVEN staged lesson + outcome deltas:
        let mut memory = test_memory();
        let mut state = AgentState::new();
        record_lesson(&mut state, "deploy", "never restart without a backup", 0.9);
        record_important_outcome(
            &mut state,
            "deploy",
            "shipped v2 at midnight",
            vec!["deploy".to_string()],
        );
        assert_eq!(state.memory_deltas.len(), 2);

        // WHEN persisted, THEN deltas drain and land in both long-term stores:
        persist_memory_deltas(&mut state, &mut memory);
        assert!(state.memory_deltas.is_empty());

        let episodes = memory.episodic.recent(5);
        assert!(episodes
            .iter()
            .any(|e| e.event.contains("LESSON [deploy] never restart without a backup")));
        assert!(episodes
            .iter()
            .any(|e| e.event.contains("OUTCOME [deploy] shipped v2 at midnight")));
        assert!(!memory.vector.search("never restart without a backup", 5).is_empty());
        assert!(!memory.vector.search("shipped v2 at midnight", 5).is_empty());
    }

    #[test]
    fn persist_memory_deltas_stores_unknown_kinds_as_beliefs() {
        // GIVEN a delta of an unrecognized kind,
        // WHEN persisted, THEN it becomes a belief (the documented fallback):
        let mut memory = test_memory();
        let mut state = AgentState::new();
        state.push_memory_delta(
            "operator_note".to_string(),
            serde_json::json!("remember the milk"),
        );
        persist_memory_deltas(&mut state, &mut memory);
        assert_eq!(
            memory.beliefs.get("operator_note"),
            Some(&serde_json::json!("remember the milk"))
        );
    }
}