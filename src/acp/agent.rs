//! HelixAcpAgent — the ACP-facing adapter over Helix's agent loop.
//!
//! This is the Helix analogue of grok-cli's `GrokAcpAgent`: it owns the ACP
//! session table and drives one prompt turn through Helix's
//! [`RuntimeLoop`](crate::agents::runtime_loop::RuntimeLoop). The turn loop
//! itself is NOT reimplemented here — the planner, rule layer, and tool
//! supervisor are Helix's, so any future planner upgrade (e.g. a real LLM
//! planner replacing `SimplePlanner`) flows into ACP automatically.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use log::{debug, info, warn};

use crate::agents::agent_state::AgentState;
use crate::agents::planner::Planner;
use crate::agents::planner_output::PlannerOutput;
use crate::agents::rule_layer::RuleLayer;
use crate::agents::runtime_loop::RuntimeLoop;
use async_trait::async_trait;

/// Minimal v1 planner for the ACP surface.
///
/// Why not `SimplePlanner`: it is an explicit placeholder whose dummy
/// `noop` tool call is rejected by the real `ToolSupervisorV2`, so every
/// ACP turn would die on step one. This planner does one real Helix tool
/// call (`system_status`, which always works) and then answers, exercising
/// the full loop — planner, supervisor, trace, halting — end to end.
/// When Helix grows a real LLM planner, it slots in here and ACP gets it
/// for free; the adapter itself is planner-agnostic.
struct AcpPlanner {
    goal: String,
}

#[async_trait]
impl Planner for AcpPlanner {
    async fn decide(&self, state: &AgentState) -> PlannerOutput {
        if state.step_count == 0 {
            return PlannerOutput::ToolCall {
                tool_name: "system_status".to_string(),
                args_json: serde_json::json!({}),
                reasoning: Some("ACP v1: grounding turn with a real tool call".to_string()),
            };
        }
        let tools = crate::acp::tools::describe_tools_for_prompt();
        PlannerOutput::FinalAnswer {
            message: format!(
                "Helix (via ACP) received: \"{}\"\n\n\
                 Turn complete after {} step(s). {}\n\n\
                 I can run tools on this host — ask me to check the inbox, \
                 read logs, or run a command.",
                self.goal, state.step_count, tools,
            ),
            reasoning: None,
        }
    }
}

/// One ACP session: working directory + conversation history.
#[derive(Debug, Clone)]
pub struct AcpSession {
    /// Opaque session id handed to the ACP client.
    pub id: String,
    /// Working directory the client reported in `session/new`.
    pub cwd: PathBuf,
    /// Prior user prompts (for continuity across turns).
    pub history: Vec<String>,
}

/// A tool call that happened during a turn, for `session/update` streaming.
#[derive(Debug, Clone)]
pub struct ToolCallRecord {
    pub name: String,
    pub args: serde_json::Value,
    pub result_summary: String,
}

/// The outcome of one `session/prompt` turn.
#[derive(Debug, Clone)]
pub struct AgentRunResult {
    /// The agent's final message for this turn.
    pub final_message: String,
    /// Tool calls executed, in order (for ACP tool-call notifications).
    pub tool_calls: Vec<ToolCallRecord>,
    /// Steps the runtime loop took.
    pub steps: u32,
    /// Whether the run ended in error.
    pub errored: bool,
    /// Whether a FinalAnswer was produced (a tool failure earlier in the
    /// turn can set `errored` while the turn still completed).
    pub had_final_answer: bool,
}

/// The ACP agent. Owns sessions; each prompt turn gets a fresh
/// [`RuntimeLoop`] so concurrent sessions never share planner state.
pub struct HelixAcpAgent {
    sessions: Arc<Mutex<HashMap<String, AcpSession>>>,
    max_steps: u32,
    next_session: Arc<Mutex<u64>>,
}

impl HelixAcpAgent {
    pub fn new(max_steps: u32) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            max_steps,
            next_session: Arc::new(Mutex::new(1)),
        }
    }

    /// Create a session and return its id.
    pub fn new_session(&self, cwd: PathBuf) -> String {
        let mut n = self.next_session.lock().unwrap_or_else(|e| e.into_inner());
        let id = format!("helix-acp-{}", *n);
        *n += 1;
        drop(n);

        let session = AcpSession {
            id: id.clone(),
            cwd,
            history: Vec::new(),
        };
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.clone(), session);
        info!("ACP session created: {}", id);
        id
    }

    /// Fetch a snapshot of a session, if it exists.
    pub fn get_session(&self, id: &str) -> Option<AcpSession> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    /// List known session ids (for `session/list`).
    pub fn list_sessions(&self) -> Vec<String> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    /// Record a user prompt in the session history.
    fn record_prompt(&self, session_id: &str, prompt: &str) {
        if let Some(s) = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(session_id)
        {
            s.history.push(prompt.to_string());
        }
    }

    /// Run one prompt turn through Helix's runtime loop.
    ///
    /// Builds an [`AgentState`] seeded with the session history + the new
    /// prompt, drives it with the standard planner stack, then extracts the
    /// final answer and the tool calls from the runtime trace.
    pub async fn run_prompt(&self, session_id: &str, prompt: &str) -> AgentRunResult {
        self.record_prompt(session_id, prompt);

        let session = self.get_session(session_id);
        let cwd = session
            .as_ref()
            .map(|s| s.cwd.clone())
            .unwrap_or_else(|| PathBuf::from("."));
        let history: Vec<String> = session
            .as_ref()
            .map(|s| s.history.clone())
            .unwrap_or_default();

        debug!(
            "ACP prompt turn: session={} cwd={} prompt_len={}",
            session_id,
            cwd.display(),
            prompt.len()
        );

        // Seed the agent state: history first, then the new prompt as the goal.
        let mut state = AgentState::new();
        for h in &history {
            state.messages.push(format!("user: {}", h));
        }
        state.messages.push(format!("user: {}", prompt));

        // Standard Helix planner stack for the ACP surface (see AcpPlanner).
        let planner = AcpPlanner {
            goal: prompt.to_string(),
        };
        // Task 210: the rule layer is now wired into RuntimeLoop::run, so an
        // empty allowlist would reject the planner's system_status grounding
        // call. AcpPlanner only ever calls system_status (see its docs) —
        // if it ever grows more tools, extend this list.
        let rule_layer = RuleLayer::new(vec!["system_status".to_string()]);
        let mut runtime = RuntimeLoop::new(planner, rule_layer, self.max_steps);

        let final_state = runtime.run(state).await;

        // Extract the final answer: last FinalAnswer in the trace wins.
        let mut final_message = String::new();
        let mut had_final_answer = false;
        let mut tool_calls = Vec::new();
        for step in &runtime.runtime_trace.steps {
            if let Some(name) = &step.tool_name {
                tool_calls.push(ToolCallRecord {
                    name: name.clone(),
                    args: step.tool_args.clone().unwrap_or(serde_json::Value::Null),
                    result_summary: truncate_for_display(
                        &step
                            .tool_result
                            .as_ref()
                            .map(|v| v.to_string())
                            .unwrap_or_default(),
                    ),
                });
            }
            if let Some(PlannerOutput::FinalAnswer { message, .. }) = &step.planner_output {
                final_message = message.clone();
                had_final_answer = true;
            }
        }

        // NOTE: AgentState::halt() sets last_error on NORMAL completion too
        // ("Terminal step reached"); only the "Tool failed: ..." halt reason
        // is a genuine error.
        let errored = final_state
            .last_error
            .as_deref()
            .is_some_and(|e| e.contains("failed"));
        if final_message.is_empty() {
            final_message = final_state
                .last_error
                .clone()
                .unwrap_or_else(|| "The agent completed without producing a final answer.".to_string());
            warn!("ACP turn produced no FinalAnswer; using fallback message");
        }

        AgentRunResult {
            final_message,
            tool_calls,
            steps: final_state.step_count,
            errored,
            had_final_answer,
        }
    }
}

/// Keep tool-result summaries short for ACP notifications.
fn truncate_for_display(s: &str) -> String {
    const MAX: usize = 500;
    if s.len() <= MAX {
        s.to_string()
    } else {
        format!("{}… [truncated]", &s[..MAX])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_returns_unique_ids() {
        // GIVEN a fresh agent:
        let agent = HelixAcpAgent::new(10);
        // WHEN we create two sessions:
        let a = agent.new_session(PathBuf::from("/tmp"));
        let b = agent.new_session(PathBuf::from("/tmp"));
        // THEN the ids differ and both are listed:
        assert_ne!(a, b);
        let listed = agent.list_sessions();
        assert!(listed.contains(&a));
        assert!(listed.contains(&b));
    }

    #[test]
    fn get_session_returns_none_for_unknown() {
        // GIVEN a fresh agent:
        let agent = HelixAcpAgent::new(10);
        // WHEN we ask for a bogus id:
        // THEN we get None, not a panic:
        assert!(agent.get_session("nope").is_none());
    }

    #[tokio::test]
    async fn run_prompt_produces_final_answer() {
        // GIVEN an agent and a session:
        let agent = HelixAcpAgent::new(10);
        let sid = agent.new_session(PathBuf::from("/tmp"));
        // WHEN we run a prompt turn:
        let result = agent.run_prompt(&sid, "hello helix").await;
        // THEN we get a final message (via the standard planner stack):
        assert!(!result.final_message.is_empty());
        assert!(result.final_message.contains("hello helix"));
        // AND the prompt was recorded in session history:
        let sess = agent.get_session(&sid).expect("session exists");
        assert_eq!(sess.history, vec!["hello helix".to_string()]);
    }

    #[tokio::test]
    async fn run_prompt_unknown_session_still_runs() {
        // GIVEN an agent with no such session:
        let agent = HelixAcpAgent::new(10);
        // WHEN we run a prompt anyway:
        let result = agent.run_prompt("ghost", "hi").await;
        // THEN it degrades gracefully (CWD defaults, no panic):
        assert!(!result.final_message.is_empty());
    }
}
