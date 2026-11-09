//! Authentication and token handling for the web server.
//!
//! Extracted from the monolithic `mod.rs` as part of the structural
//! decomposition (Task 318).

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;
use subtle::ConstantTimeEq;

/// Constant-time comparison to avoid timing attacks.
pub fn constant_time_token_eq(provided: &str, expected: &str) -> bool {
    if provided.len() != expected.len() {
        return false;
    }
    provided.as_bytes().ct_eq(expected.as_bytes()).into()
}

/// Check if the Authorization header (or X-Web-Token) contains a valid token.
pub fn http_token_ok(token: &str, headers: &HeaderMap) -> bool {
    if token.is_empty() {
        // No token configured → allow (dev / local setups)
        return true;
    }

    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer ").map(|s| s.trim()));

    let x_token = headers
        .get("x-web-token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim());

    bearer.or(x_token).is_some_and(|t| constant_time_token_eq(t, token))
}

/// Middleware that enforces the web token (if configured).
///
/// Returns 401 UNAUTHORIZED for missing or wrong token when a token is set.
pub async fn require_web_token(
    State(expected): State<Arc<String>>,
    req: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    if expected.is_empty() {
        // No token configured in this environment — pass through.
        return Ok(next.run(req).await);
    }

    if http_token_ok(&expected, req.headers()) {
        Ok(next.run(req).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

/// Stricter variant used for OKF write operations (can be separate token in future).
pub async fn require_okf_write_token(
    State(expected): State<Arc<String>>,
    req: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    if expected.is_empty() {
        return Ok(next.run(req).await);
    }

    if http_token_ok(&expected, req.headers()) {
        Ok(next.run(req).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

/// Heuristic: treat obviously secret-looking strings as secrets (for redaction).
pub fn is_secret_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k.contains("secret")
        || k.contains("token")
        || k.contains("key")
        || k.contains("password")
        || k.contains("credential")
}

/// Redact sensitive values when echoing config back to the UI.
pub fn redacted_config_str(config_str: &str) -> String {
    const REDACTED: &str = "<redacted>";

    let mut out = String::new();
    for line in config_str.lines() {
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim();
            if is_secret_key(key) {
                // preserve the key, redact the value
                out.push_str(&format!("{} = \"{}\"\n", key, REDACTED));
                continue;
            }
            // keep original line if not a secret
            out.push_str(line);
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    #[test]
    fn constant_time_eq_matches() {
        assert!(constant_time_token_eq("abc", "abc"));
        assert!(!constant_time_token_eq("abc", "abd"));
        assert!(!constant_time_token_eq("abc", "ab"));
    }

    #[test]
    fn secret_detection() {
        assert!(is_secret_key("api_key"));
        assert!(is_secret_key("auth_token"));
        assert!(is_secret_key("db_password"));
        assert!(is_secret_key("api-key"));
        assert!(is_secret_key("credential"));
        assert!(!is_secret_key("timeout"));
        assert!(!is_secret_key("port"));
        assert!(!is_secret_key("theme"));
    }

    #[test]
    fn redaction_replaces_secrets() {
        let cfg = r#"
[web]
token = "s3cr3t"
port = 8080
"#;
        let red = redacted_config_str(cfg);
        assert!(red.contains("token = \"<redacted>\""));
        assert!(red.contains("port = 8080"));
        assert!(!red.contains("s3cr3t"));
    }
}
