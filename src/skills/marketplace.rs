//! Skill Marketplace
//!
//! Allows publishing, discovering, and installing skills at runtime.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub category: String,
}

pub struct SkillMarketplace {
    skills: HashMap<String, SkillMetadata>,
}

impl SkillMarketplace {
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
        }
    }

    /// Publish a new skill to the marketplace
    pub fn publish(&mut self, meta: SkillMetadata) {
        println!("📦 Published skill to marketplace: {} v{}", meta.name, meta.version);
        self.skills.insert(meta.name.clone(), meta);
    }

    /// Discover skills by category or search term
    pub fn discover(&self, query: &str) -> Vec<&SkillMetadata> {
        self.skills
            .values()
            .filter(|s| {
                s.name.contains(query)
                    || s.description.contains(query)
                    || s.category.contains(query)
            })
            .collect()
    }

    /// Install / load a skill by name
    pub fn install(&self, name: &str) -> Option<&SkillMetadata> {
        self.skills.get(name)
    }

    pub fn list_all(&self) -> Vec<&SkillMetadata> {
        self.skills.values().collect()
    }
}

impl Default for SkillMarketplace {
    fn default() -> Self {
        Self::new()
    }
}
