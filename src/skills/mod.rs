//! Skills are registered in this module as closures mapping a skill name
//! to a function taking args and returning a NodeResult.
//!
//! A skill is a named entry in a `SkillRegistry`: it is registered
//! with a closure that takes JSON parameters and returns a NodeResult.
//! When a tool or service needs to invoke a skill, it calls
//! `SkillRegistry::call(name, params)`, which looks up the closure
//! and passes params. If a skill is not registered, `call` returns
//! `NodeResult::Error`.
//!
//! How to add a new skill (task 361 contract):
//! 1. Write the skill (struct + methods, or a plain function) in its own
//!    module under `src/skills/`.
//! 2. Register a closure in `SkillRegistry::new()` below — the closure is
//!    the skill's public surface; keep state behind an
//!    `Arc<Mutex<...>>` if the skill is stateful.
//! 3. If an LLM should be able to call it, add a tool definition in
//!    `src/tools/mod.rs` (or the owning tool module) that maps the tool's
//!    params onto `SkillRegistry::call`.
//! 4. Add unit tests in the skill's module AND a registry-level test here
//!    (registered name + a representative call returns a non-error).
//! 5. Document the skill in `docs/self-skills-contract.md` and, once it
//!    is exercised live, add an eval to the approved eval set.
//!
//! What is NOT the skill system (task 361 reconciliation):
//! - `SkillMarketplace` (`marketplace.rs`) is metadata-only (publish /
//!   discover / install of `SkillMetadata`); it loads no code and is never
//!   instantiated. It is not the skill system.
//! - `sandbox.rs` is a 5-line placeholder with no real sandbox (no WASM,
//!   no isolation). A full sandbox is a prerequisite for any
//!   self-deployment and is a separate follow-up (task 368, filed by
//!   task 366) — do not claim a sandbox exists.

pub mod bayesian;
pub mod marketplace;
pub mod sandbox;

use async_trait::async_trait;
use serde_json::Value;
use std::sync::{Arc, Mutex};

use crate::cpu::interfaces::SkillInterface;
use crate::hy_evo::node::NodeResult;
use crate::skills::bayesian::BayesianSkill;

/// A registry of all available skills.
/// Each skill is just a function that takes JSON params and returns a NodeResult.
pub struct SkillRegistry {
    skills: std::collections::HashMap<String, SkillFn>,
}

type SkillFn = Box<dyn Fn(&Value) -> NodeResult + Send + Sync>;

impl SkillRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            skills: std::collections::HashMap::new(),
        };

        // ── Built-in / noop ──────────────────────────────────────────────────
        registry.register("noop", |_params| {
            NodeResult::Value(Value::String("noop executed".into()))
        });

        // ── Shared tool bridge ───────────────────────────────────────────────
        registry.register("read_log", |params| {
            NodeResult::Text(crate::tools::execute("read_log", params))
        });
        registry.register("write_note", |params| {
            NodeResult::Text(crate::tools::execute("write_note", params))
        });
        registry.register("read_note", |params| {
            NodeResult::Text(crate::tools::execute("read_note", params))
        });
        registry.register("list_notes", |params| {
            NodeResult::Text(crate::tools::execute("list_notes", params))
        });
        registry.register("send_email", |params| {
            NodeResult::Text(crate::tools::execute("send_email", params))
        });
        registry.register("read_email", |params| {
            NodeResult::Text(crate::tools::execute("read_email", params))
        });
        registry.register("check_inbox", |params| {
            NodeResult::Text(crate::tools::execute("check_inbox", params))
        });
        registry.register("system_status", |params| {
            NodeResult::Text(crate::tools::execute("system_status", params))
        });
        registry.register("list_tools", |params| {
            NodeResult::Text(crate::tools::execute("list_tools", params))
        });
        registry.register("get_beliefs", |params| {
            NodeResult::Text(crate::tools::execute("get_beliefs", params))
        });
        registry.register("set_belief", |params| {
            NodeResult::Text(crate::tools::execute("set_belief", params))
        });

        // ── Bayesian intent reasoner (task 361) ────────────────────────────
        // Wired as the `bayes` registry skill: the closure maps the
        // `command` string param onto BayesianSkill::handle. State lives
        // behind Arc<Mutex<...>> so the closure stays 'static + Send + Sync.
        let bayes = Arc::new(Mutex::new(BayesianSkill::new()));
        registry.register("bayes", move |params| {
            let command = params.get("command").and_then(|v| v.as_str()).unwrap_or("");
            match bayes.lock() {
                Ok(mut skill) => NodeResult::Text(skill.handle(command)),
                Err(_) => NodeResult::Error("bayes skill lock poisoned".into()),
            }
        });

        registry
    }

    pub fn register<F>(&mut self, name: &str, func: F)
    where
        F: Fn(&Value) -> NodeResult + Send + Sync + 'static,
    {
        self.skills.insert(name.to_string(), Box::new(func));
    }

    pub fn call(&self, name: &str, params: &Value) -> NodeResult {
        if let Some(skill) = self.skills.get(name) {
            skill(params)
        } else {
            NodeResult::Error(format!("Unknown skill: {}", name))
        }
    }

    pub fn list_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.skills.keys().map(|s| s.as_str()).collect();
        names.sort();
        names
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Implement the SkillInterface so HyEvo can call skills.
#[async_trait]
impl SkillInterface for SkillRegistry {
    async fn call(&self, name: &str, params: &Value) -> NodeResult {
        self.call(name, params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bayes_skill_registered_and_handles_commands() {
        let registry = SkillRegistry::new();
        assert!(registry.list_names().contains(&"bayes"));

        let status = registry.call("bayes", &json!({"command": "bayes status"}));
        assert!(matches!(status, NodeResult::Text(_)));

        let updated = registry.call("bayes", &json!({"command": "bayes update cli 0.9"}));
        assert!(matches!(updated, NodeResult::Text(_)));

        // Unknown skill still errors.
        assert!(matches!(
            registry.call("nope", &Value::Null),
            NodeResult::Error(_)
        ));
    }
}
