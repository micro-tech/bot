//! Resume support for file transfers (Task 114)
//! Chunked transfer with offset tracking.

use crate::utils::log_to_file;
use std::collections::HashMap;
use std::sync::Mutex;

static RESUME_STATE: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

pub fn init_resume_state() {
    let mut guard = RESUME_STATE.lock().unwrap();
    *guard = Some(HashMap::new());
}

pub fn set_offset(path: &str, offset: u64) {
    let mut guard = RESUME_STATE.lock().unwrap();
    if let Some(map) = guard.as_mut() {
        map.insert(path.to_string(), offset);
        log_to_file(&format!("Resume offset set for {}: {}", path, offset));
    }
}

pub fn get_offset(path: &str) -> u64 {
    let guard = RESUME_STATE.lock().unwrap();
    guard.as_ref().and_then(|m| m.get(path)).copied().unwrap_or(0)
}

pub fn clear_resume(path: &str) {
    let mut guard = RESUME_STATE.lock().unwrap();
    if let Some(map) = guard.as_mut() {
        map.remove(path);
        log_to_file(&format!("Resume state cleared for {}", path));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resume_offset_roundtrip() {
        init_resume_state();
        // M25: temp_dir(), not a hard-coded /tmp. NOTE: this module is
        // currently not wired into the crate (no `mod unix_cli`
        // declaration), so this test does not run — the fix is preventive,
        // for whenever the module is wired in.
        let p = std::env::temp_dir()
            .join("helix-resume-test.bin")
            .to_string_lossy()
            .into_owned();
        set_offset(&p, 12345);
        assert_eq!(get_offset(&p), 12345);
        clear_resume(&p);
        assert_eq!(get_offset(&p), 0);
    }
}
