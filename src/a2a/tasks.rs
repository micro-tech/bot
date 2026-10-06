//! A2A task store: lifecycle management for agent tasks.
//!
//! Tasks move `submitted -> working -> completed | failed | canceled`,
//! with `input-required` reserved for the approval gate. The store is
//! in-memory (per process); each task owns a cancellation token and,
//! once execution starts, the JoinHandle of its worker.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::types::{now_rfc3339, Artifact, Message, Task, TaskState, TaskStatus};

/// A task plus its runtime bookkeeping.
pub struct StoredTask {
    pub task: Task,
    /// Fires when the task reaches a terminal state (for SSE waiters).
    pub done_tx: Option<oneshot::Sender<()>>,
    /// Set once execution is spawned; aborted on cancel.
    pub worker: Option<JoinHandle<()>>,
}

impl StoredTask {
    fn new(task: Task) -> (Self, oneshot::Receiver<()>) {
        let (tx, rx) = oneshot::channel();
        (
            Self {
                task,
                done_tx: Some(tx),
                worker: None,
            },
            rx,
        )
    }
}

/// Thread-safe in-memory task registry.
#[derive(Clone, Default)]
pub struct TaskStore {
    inner: Arc<Mutex<HashMap<String, StoredTask>>>,
}

impl TaskStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a task in `submitted` state. Returns the task and a
    /// receiver that fires when the task completes.
    pub fn create(
        &self,
        context_id: Option<String>,
        user_message: Message,
    ) -> (Task, oneshot::Receiver<()>) {
        let id = format!("a2a-{}", uuid::Uuid::new_v4().simple());
        let task = Task {
            id: id.clone(),
            context_id,
            status: TaskStatus {
                state: TaskState::Submitted,
                message: None,
                timestamp: Some(now_rfc3339()),
            },
            artifacts: vec![],
            history: vec![user_message],
        };
        let (stored, rx) = StoredTask::new(task.clone());
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, stored);
        (task, rx)
    }

    pub fn get(&self, id: &str) -> Option<Task> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .map(|s| s.task.clone())
    }

    pub fn list(&self) -> Vec<Task> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|s| s.task.clone())
            .collect()
    }

    /// Transition a task's state (no-op on unknown ids; terminal states
    /// are sticky — they cannot be moved out of).
    pub fn set_state(&self, id: &str, state: TaskState, message: Option<Message>) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(stored) = guard.get_mut(id) {
            if stored.task.status.state.is_terminal() {
                return;
            }
            stored.task.status = TaskStatus {
                state,
                message,
                timestamp: Some(now_rfc3339()),
            };
            if state.is_terminal() {
                if let Some(tx) = stored.task_done_sender() {
                    let _ = tx.send(());
                }
                stored.done_tx = None;
            }
        }
    }

    /// Attach the worker handle once execution is spawned.
    pub fn set_worker(&self, id: &str, handle: JoinHandle<()>) {
        if let Some(stored) = self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(id)
        {
            stored.worker = Some(handle);
        }
    }

    /// Append an artifact to a task.
    pub fn add_artifact(&self, id: &str, artifact: Artifact) {
        if let Some(stored) = self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(id)
        {
            stored.task.artifacts.push(artifact);
        }
    }

    /// Append a message to the task history.
    pub fn push_history(&self, id: &str, message: Message) {
        if let Some(stored) = self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(id)
        {
            stored.task.history.push(message);
        }
    }

    /// Cancel a task: abort the worker, mark canceled. Returns false when
    /// the task is unknown or already terminal.
    pub fn cancel(&self, id: &str) -> bool {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(stored) = guard.get_mut(id) else {
            return false;
        };
        if stored.task.status.state.is_terminal() {
            return false;
        }
        if let Some(handle) = stored.worker.take() {
            handle.abort();
        }
        stored.task.status = TaskStatus {
            state: TaskState::Canceled,
            message: None,
            timestamp: Some(now_rfc3339()),
        };
        if let Some(tx) = stored.task_done_sender() {
            let _ = tx.send(());
        }
        stored.done_tx = None;
        true
    }
}

impl StoredTask {
    fn task_done_sender(&mut self) -> Option<oneshot::Sender<()>> {
        self.done_tx.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a2a::types::{Part, TextPart};

    fn user_msg(text: &str) -> Message {
        Message {
            role: "user".to_string(),
            parts: vec![Part::Text(TextPart { text: text.into() })],
            message_id: "m1".into(),
        }
    }

    #[test]
    fn create_starts_submitted() {
        let store = TaskStore::new();
        let (task, _rx) = store.create(None, user_msg("hi"));
        assert_eq!(task.status.state, TaskState::Submitted);
        assert_eq!(task.history.len(), 1);
    }

    #[test]
    fn terminal_states_are_sticky() {
        let store = TaskStore::new();
        let (task, _rx) = store.create(None, user_msg("hi"));
        store.set_state(&task.id, TaskState::Completed, None);
        store.set_state(&task.id, TaskState::Working, None); // must not move
        let got = store.get(&task.id).unwrap();
        assert_eq!(got.status.state, TaskState::Completed);
    }

    #[test]
    fn cancel_unknown_returns_false() {
        let store = TaskStore::new();
        assert!(!store.cancel("nope"));
    }

    #[test]
    fn cancel_moves_to_canceled() {
        let store = TaskStore::new();
        let (task, _rx) = store.create(None, user_msg("hi"));
        assert!(store.cancel(&task.id));
        let got = store.get(&task.id).unwrap();
        assert_eq!(got.status.state, TaskState::Canceled);
        // Second cancel is a no-op (already terminal).
        assert!(!store.cancel(&task.id));
    }

    #[test]
    fn list_returns_all_tasks() {
        let store = TaskStore::new();
        store.create(None, user_msg("a"));
        store.create(None, user_msg("b"));
        assert_eq!(store.list().len(), 2);
    }
}
