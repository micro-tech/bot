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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_known_value() {
        let data = b"hello";
        let hash = sha256(data);
        assert_eq!(hash, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
    }

    #[test]
    fn test_verify_checksum() {
        let data = b"test";
        let hash = sha256(data);
        assert!(verify_checksum(data, &hash));
        assert!(!verify_checksum(data, "wronghash"));
    }
}
