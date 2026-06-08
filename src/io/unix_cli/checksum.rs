//! Checksum calculation and verification for file transfers (Task 113)
use crate::utils::log_to_file;
use sha2::{Sha256, Digest};

pub fn sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

pub fn verify_checksum(data: &[u8], expected: &str) -> bool {
    let actual = sha256(data);
    let ok = actual == expected;
    if !ok {
        log_to_file(&format!("Checksum mismatch: expected {}, got {}", expected, actual));
    }
    ok
}

pub fn checksum_for_file(path: &str) -> Result<String, String> {
    use std::fs;
    match fs::read(path) {
        Ok(bytes) => Ok(sha256(&bytes)),
        Err(e) => {
            let msg = format!("Checksum read error for {}: {}", path, e);
            log_to_file(&msg);
            Err(msg)
        }
    }
}
