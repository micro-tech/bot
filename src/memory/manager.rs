use crate::cpu::interfaces::MemoryInterface;
use crate::memory::MemoryHandle;
use crate::memory::NodeResult;
use crate::memory::episodic::EpisodicMemory;
use crate::memory::vector::VectorMemory;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Single source of truth for agent beliefs (task 203).
///
/// `beliefs.json` in the working directory is the ONE store. Both
/// `MemoryManager` (whose beliefs get injected into prompts) and the
/// `get_beliefs`/`set_belief` tools read and write this file through the
/// helpers below — one file, one serialization, no split brain.
pub const BELIEFS_FILE: &str = "beliefs.json";

/// Load the belief map from `path`.
///
/// Missing or empty file -> empty map. Corrupt JSON -> warn and return empty.
/// Never panics on operator data: a broken beliefs file must not kill the bot.
pub fn load_beliefs_from(path: &Path) -> HashMap<String, Value> {
    match fs::read_to_string(path) {
        Ok(content) if !content.trim().is_empty() => match serde_json::from_str(&content) {
            Ok(map) => map,
            Err(e) => {
                log::warn!(
                    "beliefs file {} is corrupt ({}); starting with empty beliefs",
                    path.display(),
                    e
                );
                HashMap::new()
            }
        },
        _ => HashMap::new(),
    }
}

/// Load beliefs from the canonical [`BELIEFS_FILE`].
pub fn load_beliefs() -> HashMap<String, Value> {
    load_beliefs_from(Path::new(BELIEFS_FILE))
}

/// Persist the belief map to `path` as pretty JSON (the one serialization).
pub fn save_beliefs_to(path: &Path, beliefs: &HashMap<String, Value>) -> std::io::Result<()> {
    let pretty = serde_json::to_string_pretty(beliefs)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    fs::write(path, pretty)
}

/// Persist beliefs to the canonical [`BELIEFS_FILE`].
pub fn save_beliefs(beliefs: &HashMap<String, Value>) -> std::io::Result<()> {
    save_beliefs_to(Path::new(BELIEFS_FILE), beliefs)
}

/// Durably write `bytes` to `path`: write to a temp file in the same
/// directory, fsync it, atomically rename over the target, then fsync the
/// directory entry. A crash at any point leaves either the old or the new
/// complete file — never a torn write.
pub(crate) fn durable_write_json(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        use std::io::Write;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    // Fsync the directory so the rename itself is durable.
    if let Some(parent) = path.parent() {
        if let Ok(dir) = fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

/// Serializes the (few) tests that touch the real ./beliefs.json.
/// Production code never touches this lock.
#[cfg(test)]
pub static BELIEFS_FILE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub struct MemoryManager {
    pub working: MemoryHandle,
    pub vector: VectorMemory,
    pub episodic: EpisodicMemory,
    pub beliefs: HashMap<String, Value>,
}

impl MemoryManager {
    pub fn new(working_max: usize, episodic_max: usize) -> Self {
        Self {
            working: MemoryHandle::new(working_max),
            // Disk-backed after task 204: the bot's memory survives restarts.
            vector: VectorMemory::load_or_new(),
            episodic: EpisodicMemory::load_or_new(episodic_max),
            // Beliefs live in beliefs.json (single source of truth, task 203):
            // whatever the operator stored via the tools is live from birth.
            beliefs: load_beliefs(),
        }
    }

    pub fn record_user_message(&mut self, text: &str) {
        let _ = self
            .working
            .write("context", Value::String(text.to_string()));

        self.episodic.record(format!("user_msg: {}", text));
    }

    pub fn search_facts(&mut self, query: &str, k: usize) -> Vec<String> {
        self.vector.search(query, k)
    }

    pub fn set_belief(&mut self, key: &str, value: Value) {
        self.beliefs.insert(key.to_string(), value);
        // Write-through: beliefs.json is the single source of truth, so a
        // belief set here is visible to the tools (and the next process).
        // A failed write is logged, never fatal — the in-memory map is intact.
        if let Err(e) = save_beliefs(&self.beliefs) {
            log::error!("failed to persist beliefs to {}: {}", BELIEFS_FILE, e);
        }
    }

    /// Reload beliefs from disk, picking up anything the `set_belief` tool
    /// (or the operator's editor) wrote directly to beliefs.json.
    /// Called before prompt injection so prompts never see stale beliefs.
    pub fn refresh_beliefs(&mut self) {
        self.beliefs = load_beliefs();
    }

    pub fn get_belief(&self, key: &str) -> Option<&Value> {
        self.beliefs.get(key)
    }

    pub fn get_all_beliefs(&self) -> &HashMap<String, Value> {
        &self.beliefs
    }

    /// Generate a Mermaid graph visualization of beliefs
    pub fn visualize_beliefs(&self) -> String {
        let mut mermaid = "graph TD;\n".to_string();
        for (key, value) in &self.beliefs {
            let val_str = value.to_string().replace("\"", "'");
            mermaid.push_str(&format!("    {}[\"{}: {}\"];\n", key.replace(" ", "_").replace("-", "_"), key, val_str));
        }
        mermaid
    }
}

impl MemoryInterface for MemoryManager {
    fn read(&mut self, key: &str) -> NodeResult {
        self.working.read(key)
    }

    fn write(&mut self, key: &str, value: Value) -> NodeResult {
        self.working.write(key, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_memory_manager_new() {
        let manager = MemoryManager::new(10, 20);
        // Basic test to ensure creation
        assert_eq!(manager.working.max_len, 10);
        // You can add more assertions based on the actual structure
    }

    /// Unique scratch file per test so parallel test threads never collide.
    fn scratch_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "helix-beliefs-test-{}-{}",
            std::process::id(),
            name
        ))
    }

    #[test]
    fn load_beliefs_from_missing_file_returns_empty_belief_map() {
        // GIVEN a path that does not exist:
        let path = scratch_path("missing.json");
        let _ = std::fs::remove_file(&path);

        // WHEN we load beliefs from it,
        // THEN we get an empty map (no panic, no error):
        let beliefs = load_beliefs_from(&path);
        assert!(beliefs.is_empty());
    }

    #[test]
    fn load_beliefs_from_corrupt_file_returns_empty_map_without_panic() {
        // GIVEN a beliefs file containing garbage:
        let path = scratch_path("corrupt.json");
        std::fs::write(&path, "{ this is not json !!!").unwrap();

        // WHEN we load it,
        // THEN we get an empty map instead of a panic (warn is logged):
        let beliefs = load_beliefs_from(&path);
        assert!(beliefs.is_empty());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_beliefs_to_then_load_beliefs_from_round_trip_preserves_entries() {
        // GIVEN a belief map with several entries:
        let path = scratch_path("roundtrip.json");
        let mut map = HashMap::new();
        map.insert("sky".to_string(), json!("blue"));
        map.insert("answer".to_string(), json!(42));

        // WHEN we save it and load it back,
        // THEN every entry survives the trip:
        save_beliefs_to(&path, &map).expect("save must succeed");
        let loaded = load_beliefs_from(&path);
        assert_eq!(loaded, map);

        let _ = std::fs::remove_file(&path);
    }

    /// Guard: the ONE test allowed to touch the real ./beliefs.json.
    /// Removes the file on entry and on drop so the repo is never polluted.
    struct DefaultBeliefsFileGuard;
    impl DefaultBeliefsFileGuard {
        fn arm() -> Self {
            let _ = std::fs::remove_file(BELIEFS_FILE);
            Self
        }
    }
    impl Drop for DefaultBeliefsFileGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(BELIEFS_FILE);
        }
    }

    #[test]
    fn manager_set_belief_write_through_persists_to_beliefs_json_on_disk() {
        // GIVEN a clean slate (guarded: no other test may touch this file):
        let _lock = BELIEFS_FILE_TEST_LOCK.lock().unwrap();
        let _guard = DefaultBeliefsFileGuard::arm();

        // WHEN we set a belief through the manager,
        // THEN beliefs.json on disk contains it (write-through):
        let mut manager = MemoryManager::new(10, 20);
        manager.set_belief("test_key", json!("test_value"));
        let raw = std::fs::read_to_string(BELIEFS_FILE).expect("beliefs.json must exist");
        assert!(raw.contains("test_key"), "file must contain the key");

        // WHEN a brand-new manager is constructed (process-equivalent restart),
        // THEN it loads the belief back from disk (single source of truth):
        drop(manager);
        let fresh = MemoryManager::new(10, 20);
        assert_eq!(fresh.get_belief("test_key"), Some(&json!("test_value")));
        // (guard drops here and removes beliefs.json)
    }

    #[test]
    fn manager_new_with_corrupt_beliefs_file_starts_empty_without_panic() {
        // GIVEN a corrupt beliefs.json on disk:
        let _lock = BELIEFS_FILE_TEST_LOCK.lock().unwrap();
        let _guard = DefaultBeliefsFileGuard::arm();
        std::fs::write(BELIEFS_FILE, "{ nope, not json").unwrap();

        // WHEN a manager is constructed,
        // THEN it starts with empty beliefs instead of panicking (warn logged):
        let manager = MemoryManager::new(10, 20);
        assert!(manager.get_all_beliefs().is_empty());
        // (guard drops here and removes beliefs.json)
    }
}
