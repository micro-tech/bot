//! Hierarchical Goal / Plan Tree structures for advanced autonomous agents.
//!
//! This enables Hermies/OpenClaw-style recursive goal decomposition:
//!   - A top-level goal can be broken into sub-goals
//!   - Each sub-goal can itself be decomposed
//!   - The RuntimeLoop can work on the current leaf while keeping parent context
//!
//! The structures are intentionally lightweight so they can be stored in AgentState
//! and serialized for persistence / debugging.

use serde::{Deserialize, Serialize};

/// Status of a plan node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlanStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Blocked,
}

/// A single node in the goal/plan tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanNode {
    /// Unique identifier for this node (can be used as a key in maps)
    pub id: String,

    /// Human-readable description of this goal / sub-goal
    pub description: String,

    /// Current status
    pub status: PlanStatus,

    /// Optional reasoning or context for why this node exists
    pub reasoning: Option<String>,

    /// Child sub-goals (order matters — first child is usually attempted first)
    pub children: Vec<PlanNode>,

    /// Optional result / output when this node completes
    pub result: Option<String>,

    /// Depth in the tree (0 = root)
    pub depth: u32,
}

impl PlanNode {
    /// Create a new root-level plan node.
    pub fn new_root(description: impl Into<String>) -> Self {
        Self {
            id: uuid_simple(),
            description: description.into(),
            status: PlanStatus::Pending,
            reasoning: None,
            children: Vec::new(),
            result: None,
            depth: 0,
        }
    }

    /// Create a child node under this node.
    pub fn add_child(&mut self, description: impl Into<String>) -> &mut PlanNode {
        let child = PlanNode {
            id: uuid_simple(),
            description: description.into(),
            status: PlanStatus::Pending,
            reasoning: None,
            children: Vec::new(),
            result: None,
            depth: self.depth + 1,
        };
        self.children.push(child);
        self.children.last_mut().unwrap()
    }

    /// Find a node by ID (depth-first search).
    pub fn find_mut(&mut self, id: &str) -> Option<&mut PlanNode> {
        if self.id == id {
            return Some(self);
        }
        for child in &mut self.children {
            if let Some(found) = child.find_mut(id) {
                return Some(found);
            }
        }
        None
    }

    /// Count total nodes in this subtree (including self).
    pub fn node_count(&self) -> usize {
        1 + self.children.iter().map(|c| c.node_count()).sum::<usize>()
    }

    /// Count how many nodes are still pending or in-progress.
    pub fn open_count(&self) -> usize {
        let self_open = matches!(self.status, PlanStatus::Pending | PlanStatus::InProgress);
        self_open as usize
            + self.children.iter().map(|c| c.open_count()).sum::<usize>()
    }

    /// Mark this node (and optionally children) as completed.
    pub fn mark_completed(&mut self, result: Option<String>) {
        self.status = PlanStatus::Completed;
        self.result = result;
    }

    /// Mark this node as failed.
    pub fn mark_failed(&mut self, reason: impl Into<String>) {
        self.status = PlanStatus::Failed;
        self.reasoning = Some(reason.into());
    }
}

/// Container for the full goal tree + navigation helpers.
/// Stored inside AgentState for the duration of a run.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GoalTree {
    /// The root goal (may be None until the planner decomposes the top-level goal)
    pub root: Option<PlanNode>,

    /// Which node ID is currently being worked on (the "focus")
    pub current_focus_id: Option<String>,

    /// Optional top-level goal string (for display / logging)
    pub top_level_goal: Option<String>,
}

impl GoalTree {
    pub fn new(top_level_goal: impl Into<String>) -> Self {
        Self {
            root: None,
            current_focus_id: None,
            top_level_goal: Some(top_level_goal.into()),
        }
    }

    /// Initialize the tree with a root node.
    pub fn set_root(&mut self, root: PlanNode) {
        self.current_focus_id = Some(root.id.clone());
        self.root = Some(root);
    }

    /// Get a reference to the currently focused node.
    pub fn current_focus(&self) -> Option<&PlanNode> {
        let id = self.current_focus_id.as_ref()?;
        self.root.as_ref()?.find(id) // we need a non-mut find
    }

    /// Advance focus to the first pending child of the current node (if any).
    /// Returns true if focus changed.
    pub fn advance_to_first_pending_child(&mut self) -> bool {
        if let Some(root) = &mut self.root {
            if let Some(current_id) = &self.current_focus_id {
                if let Some(current) = root.find_mut(current_id) {
                    for child in &current.children {
                        if child.status == PlanStatus::Pending {
                            self.current_focus_id = Some(child.id.clone());
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Mark the current focus node as completed and move focus upward or to siblings.
    pub fn complete_current(&mut self, result: Option<String>) -> bool {
        if let Some(root) = &mut self.root {
            if let Some(id) = &self.current_focus_id {
                if let Some(node) = root.find_mut(id) {
                    node.mark_completed(result);
                    // Try to move focus to a sibling or parent
                    // (simplified: just clear for now — a real impl would search upward)
                    self.current_focus_id = None;
                    return true;
                }
            }
        }
        false
    }

    /// Total number of nodes in the entire tree.
    pub fn total_nodes(&self) -> usize {
        self.root.as_ref().map_or(0, |r| r.node_count())
    }

    /// How many nodes are still open (pending or in-progress).
    pub fn open_nodes(&self) -> usize {
        self.root.as_ref().map_or(0, |r| r.open_count())
    }
}

// Simple UUID helper (no external dependency)
fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("node_{:x}", now as u64)
}

// Need a non-mut find helper on PlanNode
impl PlanNode {
    pub fn find(&self, id: &str) -> Option<&PlanNode> {
        if self.id == id {
            return Some(self);
        }
        for child in &self.children {
            if let Some(found) = child.find(id) {
                return Some(found);
            }
        }
        None
    }
}