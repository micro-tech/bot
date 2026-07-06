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

    /// Alias used by memory_continuity for ergonomic access.
    pub fn get_recent(&self, n: usize) -> Vec<String> {
        self.recent(n)
            .into_iter()
            .map(|e| e.event)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let mem = EpisodicMemory::new(10);
        assert!(mem.episodes.is_empty());
        assert_eq!(mem.next_id, 0);
        assert_eq!(mem.max_len, 10);
    }

    #[test]
    fn test_record() {
        let mut mem = EpisodicMemory::new(10);
        let id = mem.record("test event".to_string());
        assert_eq!(id, 0);
        assert_eq!(mem.episodes.len(), 1);
        assert_eq!(mem.episodes[0].event, "test event");
        assert_eq!(mem.next_id, 1);
    }

    #[test]
    fn test_recent() {
        let mut mem = EpisodicMemory::new(10);
        mem.record("event1".to_string());
        mem.record("event2".to_string());
        mem.record("event3".to_string());
        let recent = mem.recent(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].event, "event2");
        assert_eq!(recent[1].event, "event3");
    }

    #[test]
    fn test_max_len() {
        let mut mem = EpisodicMemory::new(2);
        mem.record("event1".to_string());
        mem.record("event2".to_string());
        mem.record("event3".to_string());
        assert_eq!(mem.episodes.len(), 2);
        assert_eq!(mem.episodes[0].event, "event2");
        assert_eq!(mem.episodes[1].event, "event3");
    }
}