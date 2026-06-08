//! Vector Memory for RAG
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone)]
pub struct Fact {
    pub id: u32,
    pub text: String,
    pub embedding: Vec<f32>, // Dummy 384 dim
}

pub struct VectorMemory {
    facts: HashMap<u32, Fact>,
    next_id: u32,
}

impl VectorMemory {
    pub fn new() -> Self {
        Self { facts: HashMap::new(), next_id: 0 }
    }

    pub fn add_fact(&mut self, text: String) -> u32 {
        let embedding = dummy_embed(&text); // Placeholder embed
        let id = self.next_id;
        self.facts.insert(id, Fact { id, text, embedding });
        self.next_id += 1;
        id
    }

    pub fn search(&self, query: &str, top_k: usize) -> Vec<String> {
        if self.facts.is_empty() {
            return Vec::new();
        }

        let query_emb = dummy_embed(query);
        let mut scores: Vec<(f32, &String)> = self.facts.values()
            .map(|f| (cosine_sim(&query_emb, &f.embedding), &f.text))
            .collect();

        scores.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scores.into_iter().take(top_k).map(|(_, text)| text.clone()).collect()
    }
}

fn dummy_embed(text: &str) -> Vec<f32> {
    vec![text.len() as f32; 384] // Dummy
}

fn cosine_sim(a: &[f32], b: &[f32]) -> f32 {
    let dot = a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
    let norm_a = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (norm_a * norm_b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_search() {
        let mut mem = VectorMemory::new();
        mem.add_fact("Rust is great".to_string());
        mem.add_fact("Tokio async".to_string());
        let results = mem.search("Rust", 2);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_add_fact() {
        let mut mem = VectorMemory::new();
        let id = mem.add_fact("test fact".to_string());
        assert_eq!(id, 0);
        assert_eq!(mem.facts.len(), 1);
        assert_eq!(mem.facts[&0].text, "test fact");
        assert_eq!(mem.next_id, 1);
    }

    #[test]
    fn test_search_empty() {
        let mem = VectorMemory::new();
        let results = mem.search("query", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_cosine_sim() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = cosine_sim(&a, &b);
        assert!((sim - 1.0).abs() < 1e-6);
    }
}