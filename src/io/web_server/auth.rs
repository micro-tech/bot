//! Authentication and token handling for the web server.
//!
//! Extracted from the monolithic `mod.rs` as part of the structural
//! decomposition (Task 318). This module is the single home for all web
//! auth helpers (task 321 dedupe): `mod.rs` must not re-define them.
//!
//! Behavior preserved from `mod.rs` (the richer side — do not regress):
//! - `http_token_ok` accepts both `Bearer ` and lowercase `bearer `.
//! - `require_okf_write_token` accepts the web token OR grok-cli's OKF
//!   `auth_token` (dual-token). A single-token check would break
//!   grok-cli OKF writes.
//! - `require_web_token` returns `(StatusCode, &'static str)` with a body.
//! - Redaction uses `***REDACTED***`, the wide fragment list, and preserves
//!   comments / formatting / indentation.

use axum::{
    body::Body,
    extract::State,
    http::{header::AUTHORIZATION, HeaderMap, Request, StatusCode},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;
use subtle::ConstantTimeEq;

/// Placeholder substituted for secret values in config text sent to clients.
pub const REDACTED: &str = "***REDACTED***";

/// Constant-time token comparison.
///
/// Plain `==` bails on the first mismatched byte, leaking the token
/// byte-by-byte to a timing oracle. On a loopback homelab service this is
/// theoretical — but the fix is one line, so there's no excuse.
pub fn constant_time_token_eq(provided: &str, expected: &str) -> bool {
    if provided.len() != expected.len() {
        return false;
    }
    provided.as_bytes().ct_eq(expected.as_bytes()).into()
}

/// Check an HTTP request's token against the configured web token.
/// `Authorization: Bearer <token>` (or lowercase `bearer `) or
/// `X-Web-Token: <token>` are accepted.
/// When no token is configured this returns true (startup logs a warning).
pub fn http_token_ok(token: &str, headers: &HeaderMap) -> bool {
    if token.is_empty() {
        return true;
    }
    let bearer = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer ").or_else(|| s.strip_prefix("bearer ")));
    let x_token = headers.get("X-Web-Token").and_then(|v| v.to_str().ok());
    bearer
        .or(x_token)
        .is_some_and(|t| constant_time_token_eq(t, token))
}

/// Axum middleware: gate mutating routes behind the web token.
pub async fn require_web_token(
    State(token): State<Arc<String>>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, (StatusCode, &'static str)> {
    if http_token_ok(&token, req.headers()) {
        Ok(next.run(req).await)
    } else {
        Err((StatusCode::UNAUTHORIZED, "missing or invalid web token"))
    }
}

/// Auth state for the v1 OKF write endpoints (task 208).
/// grok-cli sends the OKF `auth_token` as a Bearer token; it never sees the web
/// token, so these endpoints accept EITHER credential.
#[derive(Clone)]
pub struct OkfWriteAuth {
    pub web_token: Arc<String>,
    pub okf_token: String,
}

/// Axum middleware: gate POST /okf/traces and POST /okf/bundles/{bundle}/concepts
/// behind the web token OR the OKF auth_token Bearer. Fail-open when neither is
/// configured, matching the house posture (same as http_token_ok).
pub async fn require_okf_write_token(
    State(auth): State<OkfWriteAuth>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, (StatusCode, &'static str)> {
    if auth.web_token.is_empty() && auth.okf_token.is_empty() {
        return Ok(next.run(req).await);
    }
    if http_token_ok(&auth.web_token, req.headers()) {
        return Ok(next.run(req).await);
    }
    let bearer = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer ").or_else(|| s.strip_prefix("bearer ")));
    if !auth.okf_token.is_empty()
        && bearer.is_some_and(|b| constant_time_token_eq(b, &auth.okf_token))
    {
        return Ok(next.run(req).await);
    }
    Err((StatusCode::UNAUTHORIZED, "missing or invalid token"))
}

/// Key-name fragments (case-insensitive) treated as secrets for redaction.
/// Bare "key" / "credential" are included deliberately: a redactor would
/// rather over-redact ("monkey" gets masked — harmless) than leak `api-key`.
pub fn is_secret_key(key: &str) -> bool {
    let k = key.to_lowercase();
    [
        "token",
        "secret",
        "password",
        "passwd",
        "api_key",
        "apikey",
        "key",
        "credential",
        "auth",
        "private_key",
    ]
    .iter()
    .any(|frag| k.contains(frag))
}

/// Render a display-safe copy of the raw config TOML with secret values
/// replaced by `***REDACTED***`.
///
/// Line-based (not a TOML re-serialize) so comments, ordering, and formatting
/// survive for the config editor. Best-effort: only `key = value` lines whose
/// key looks secret are masked — the real security boundary is the web token +
/// loopback bind (task 183); this is defense-in-depth for the WS push.
pub fn redacted_config_str(config_str: &str) -> String {
    config_str
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            // Skip section headers, comments, blank lines, and lines without '='.
            if trimmed.is_empty()
                || trimmed.starts_with('[')
                || trimmed.starts_with('#')
                || !trimmed.contains('=')
            {
                return line.to_string();
            }
            let raw_key = trimmed
                .split_once('=')
                .map(|(k, _)| k.trim())
                .unwrap_or("");
            let key = raw_key.trim_matches('"').trim_matches('\'');
            if key.is_empty() || !is_secret_key(key) {
                return line.to_string();
            }
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            format!("{}{} = \"{}\"", indent, raw_key, REDACTED)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    // NOTE: no `use axum::http::HeaderMap;` here — `super::*` already
    // brings it in (it is imported at the top of this module), so a
    // redundant import would warn as unused (task 321).

    #[test]
    fn constant_time_eq_matches() {
        assert!(constant_time_token_eq("abc", "abc"));
        assert!(!constant_time_token_eq("abc", "abd"));
        assert!(!constant_time_token_eq("abc", "ab"));
    }

    #[test]
    fn http_token_ok_accepts_lowercase_bearer() {
        // mod.rs behavior (task 321): lowercase `bearer ` is accepted too.
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "bearer s3cret".parse().unwrap());
        assert!(http_token_ok("s3cret", &headers));
        assert!(!http_token_ok("other", &headers));
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
        assert!(red.contains("token = \"***REDACTED***\""));
        assert!(red.contains("port = 8080"));
        assert!(!red.contains("s3cr3t"));
    }
}
