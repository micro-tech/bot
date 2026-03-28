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

        // Register built-in skills here
        registry.register("noop", |params| {
            NodeResult::Value(Value::String("noop executed".into()))
        });

        // Example: register Bayesian skill if needed
        // registry.register("bayesian_predict", |params| { ... });

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
}

/// Implement the SkillInterface so HyEvo can call skills.
#[async_trait]
impl SkillInterface for SkillRegistry {
    async fn call(&self, name: &str, params: &Value) -> NodeResult {
        self.call(name, params)
    }
}
