use crate::bayesian::{BayesianReasoner, NaiveBayesMultinomial};

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BayesianHookConfig {
    pub priors: HashMap<String, f64>,
}

pub struct BayesianHook {
    reasoner: BayesianReasoner,
    nb_classifier: NaiveBayesMultinomial,
}

impl BayesianHook {
    pub fn new(config: BayesianHookConfig) -> Self {
        let priors = config.priors.clone();
        let reasoner = BayesianReasoner::new(priors);

        let mut nb = NaiveBayesMultinomial::new(1.0);
        // Load training data from memory or config
        nb.fit(&vec![]); // Placeholder - in real use, load from memory

        Self { reasoner, nb_classifier: nb }
    }

    pub fn pre_process(&mut self, msg: &str) -> String {
        let tokens: Vec<String> = msg.to_lowercase().split_whitespace().map(String::from).collect();
        let probs = self.nb_classifier.predict(&tokens);
        self.reasoner.update(msg.to_string(), probs);

        let (top, prob) = self.reasoner.top_belief();
        format!("Bayesian pre-hook: Intent '{}' ({:.1}%) | Original: {}", top, prob * 100.0, msg)
    }

    pub fn post_process(&self, response: &str, belief: &str) -> String {
        format!("{} | Belief: {}", response, belief)
    }
}

// Register in bot hooks system (call from main/bus)
