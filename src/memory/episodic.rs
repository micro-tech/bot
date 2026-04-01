// src/memory/episodic.rs
use crate::utils::now_ms;

#[derive(Debug, Clone)]
pub struct Episode {
    pub id: u64,
    pub event: String,
    pub timestamp: u64,
}

pub struct EpisodicMemory {
    episodes: Vec<Episode>,
    next_id: u64,
    max_len: usize,
}

impl EpisodicMemory {
    pub fn new(max_len: usize) -> Self {
        Self {
            episodes: Vec::new(),
            next_id: 0,
            max_len,
        }
    }

    pub fn record(&mut self, event: String) -> u64 {
        let id = self.next_id;
        self.next_id += 1;

        let episode = Episode {
            id,
            event,
            timestamp: now_ms(),
        };

        self.episodes.push(episode);

        // bound size
        if self.episodes.len() > self.max_len {
            let overflow = self.episodes.len() - self.max_len;
            self.episodes.drain(0..overflow);
        }

        id
    }

    pub fn recent(&self, n: usize) -> Vec<Episode> {
        let len = self.episodes.len();
        let start = len.saturating_sub(n);
        self.episodes[start..].to_vec()
    }
}
