//! Memory Module - Short Term
pub mod short_term {
    use serde::{Serialize, Deserialize};
    use std::fs;
    use std::path::Path;

    #[derive(Serialize, Deserialize)]
    pub struct SessionData {
        pub context: Vec<String>,
    }

    pub fn save_session(data: &SessionData, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string(data)?;
        fs::write(path, json)?;
        Ok(())
    }

    pub fn load_session(path: &Path) -> Result<SessionData, Box<dyn std::error::Error>> {
        let json = fs::read_to_string(path)?;
        let data: SessionData = serde_json::from_str(&json)?;
        Ok(data)
    }
}

pub mod long_term {
    // Similar for persistent JSON
    pub fn save_fact(fact: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Append to memory_store.json
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::short_term::*;
    #[test]
    fn test_session() {
        let data = SessionData { context: vec![\"test\".to_string()] };
        // Test save/load
    }
}