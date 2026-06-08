//! CLI authentication (Task 80, 92)
use std::fs;
use std::path::Path;

const TOKEN_PATH: &str = "/etc/bot/token";

pub fn authenticate(token: &str) -> bool {
    // If no token file exists, allow all (dev mode)
    if !Path::new(TOKEN_PATH).exists() {
        return true;
    }

    match fs::read_to_string(TOKEN_PATH) {
        Ok(stored) => stored.trim() == token.trim(),
        Err(_) => false,
    }
}

pub fn read_token() -> Option<String> {
    fs::read_to_string(TOKEN_PATH).ok().map(|s| s.trim().to_string())
}

