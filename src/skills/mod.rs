pub mod bayesian;

use async_trait::async_trait;
use serde_json::Value;

use crate::cpu::interfaces::SkillInterface;
use crate::hy_evo::node::NodeResult;

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

/// Implement the SkillInterface so HyEvo can call skills.
#[async_trait]
impl SkillInterface for SkillRegistry {
    async fn call(&self, name: &str, params: &Value) -> NodeResult {
        self.call(name, params)
    }
}
