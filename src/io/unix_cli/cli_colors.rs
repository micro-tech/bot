//! Colored output support (Task 97)
use std::env;

pub fn color_enabled() -> bool {
    env::var("NO_COLOR").is_err() && env::var("CLICOLOR").unwrap_or_else(|_| "1".to_string()) != "0"
}

pub fn red(s: &str) -> String {
    if color_enabled() { format!("\x1b[31m{}\x1b[0m", s) } else { s.to_string() }
}

pub fn green(s: &str) -> String {
    if color_enabled() { format!("\x1b[32m{}\x1b[0m", s) } else { s.to_string() }
}

pub fn yellow(s: &str) -> String {
    if color_enabled() { format!("\x1b[33m{}\x1b[0m", s) } else { s.to_string() }
}

pub fn blue(s: &str) -> String {
    if color_enabled() { format!("\x1b[34m{}\x1b[0m", s) } else { s.to_string() }
}
