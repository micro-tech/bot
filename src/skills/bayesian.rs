use crate::bayesian::{BayesianReasoner, NaiveBayesMultinomial};
use std::collections::HashMap;

#[derive(Debug)]
pub struct BayesianSkill {
    reasoner: BayesianReasoner,
}

impl BayesianSkill {
    pub fn new() -> Self {
        let mut priors = HashMap::new();
        priors.insert("positive".to_string(), 0.5);
        priors.insert("negative".to_string(), 0.3);
        priors.insert("neutral".to_string(), 0.2);
        Self {
            reasoner: BayesianReasoner::new(priors),
        }
    }

    pub fn handle(&mut self, command: &str) -> String {
        if command.starts_with("bayes update ") {
            let parts: Vec<&str> = command.splitn(3, ' ').collect();
            if parts.len() == 3 {
                let evidence = parts[1].to_string();
                let lik_str = parts[2];
                // Parse lik map, simple mock
                let mut lik = HashMap::new();
                lik.insert(evidence.clone(), 0.8);
                self.reasoner.update(evidence, lik);
                let (top, prob) = self.reasoner.top_belief();
                format!("Updated! Top belief: {} ({:.1}%)", top, prob * 100.0)
            } else {
                "Usage: bayes update <evidence> <likelihood_json>".to_string()
            }
        } else if command == "bayes status" {
            let (top, prob) = self.reasoner.top_belief();
            format!("Current top: {} ({:.1}%)", top, prob * 100.0)
        } else {
            "Bayesian skill loaded. Commands: bayes update <ev> <lik>, bayes status".to_string()
        }
    }

    pub fn train_intent(&mut self, training_data: Vec<(Vec<String>, String)>) {
        // Integrate NB
        let mut nb = NaiveBayesMultinomial::new(1.0);
        nb.fit(&training_data);
        // Save to memory
    }
}

// Register in bot skills system (e.g., tools or mcp)
