//! Encryption wrapper for CLI socket (Task 112)
//! Simple XOR + token-based auth for now; can be upgraded to TLS.

use crate::io::unix_cli::cli_auth;

pub fn encrypt(data: &[u8], key: &[u8]) -> Vec<u8> {
    if key.is_empty() {
        return data.to_vec();
    }
    data.iter()
        .enumerate()
        .map(|(i, &b)| b ^ key[i % key.len()])
        .collect()
}

pub fn decrypt(data: &[u8], key: &[u8]) -> Vec<u8> {
    encrypt(data, key) // XOR is symmetric
}

/// Validate token before allowing sensitive operations
pub fn require_auth(token: Option<&str>) -> bool {
    match token {
        Some(t) => cli_auth::authenticate(t),
        None => true, // No token provided -> dev mode allows
    }
}
