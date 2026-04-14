//! Email tools — SMTP send (lettre + rustls) and IMAP read (native-tls).
//!
//! ## Configuration — add to .env
//!
//! ```
//! # SMTP (sending)
//! SMTP_HOST=smtp.gmail.com
//! SMTP_PORT=587            # 587 = STARTTLS  |  465 = SSL
//! SMTP_USER=you@gmail.com
//! SMTP_PASSWORD=app-password
//! SMTP_FROM=you@gmail.com  # optional, defaults to SMTP_USER
//!
//! # IMAP (reading)
//! IMAP_HOST=imap.gmail.com
//! IMAP_PORT=993
//! IMAP_USER=you@gmail.com
//! IMAP_PASSWORD=app-password
//! ```
//!
//! Gmail: enable 2-FA and create an App Password at
//! <https://myaccount.google.com/apppasswords>.
//! See `Doc's/email_setup.md` for full setup instructions.

use lettre::{
    Message, SmtpTransport, Transport, message::header::ContentType,
    transport::smtp::authentication::Credentials,
};
use log::{error, info, warn};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::net::TcpStream;

// ── send_email ────────────────────────────────────────────────────────────────

/// Send a real email via SMTP (lettre + rustls).
///
/// Falls back to `logs/email_outbox.md` when SMTP env vars are absent.
pub fn send_email(args: &Value) -> String {
    let to = args["to"].as_str().unwrap_or("").trim().to_string();
    let subject = args["subject"]
        .as_str()
        .unwrap_or("(no subject)")
        .to_string();
    let body = args["body"].as_str().unwrap_or("").to_string();

    if to.is_empty() {
        return "Error: 'to' address is required.".to_string();
    }

    // ── Read SMTP config from environment ─────────────────────────────────
    let smtp_host = std::env::var("SMTP_HOST").unwrap_or_default();
    let smtp_user = std::env::var("SMTP_USER").unwrap_or_default();
    let smtp_pass = std::env::var("SMTP_PASSWORD").unwrap_or_default();
    let smtp_from = std::env::var("SMTP_FROM")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| smtp_user.clone());
    let smtp_port: u16 = std::env::var("SMTP_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(587);

    // ── Fallback when SMTP not configured ─────────────────────────────────
    if smtp_host.is_empty() || smtp_user.is_empty() || smtp_pass.is_empty() {
        warn!("SMTP not configured — saving email to outbox file");
        let entry = format!("---\nTo: {}\nSubject: {}\n\n{}\n\n", to, subject, body);
        let _ = fs::create_dir_all("logs");
        let _ = append_to_file("logs/email_outbox.md", &entry);
        return "⚠️  SMTP not configured — email saved to logs/email_outbox.md\n\n\
             To enable real sending, add to .env:\n\
             SMTP_HOST=smtp.gmail.com\n\
             SMTP_PORT=587\n\
             SMTP_USER=you@gmail.com\n\
             SMTP_PASSWORD=your-app-password\n\
             SMTP_FROM=you@gmail.com"
            .to_string();
    }

    // ── Parse mailbox addresses ───────────────────────────────────────────
    let from_box = match smtp_from.parse::<lettre::message::Mailbox>() {
        Ok(m) => m,
        Err(e) => return format!("Invalid SMTP_FROM '{}': {}", smtp_from, e),
    };
    let to_box = match to.parse::<lettre::message::Mailbox>() {
        Ok(m) => m,
        Err(e) => return format!("Invalid 'to' address '{}': {}", to, e),
    };

    // ── Build the email message ───────────────────────────────────────────
    let email = match Message::builder()
        .from(from_box)
        .to(to_box)
        .subject(&subject)
        .header(ContentType::TEXT_PLAIN)
        .body(body)
    {
        Ok(m) => m,
        Err(e) => return format!("Error building email: {}", e),
    };

    // ── Connect via SMTP ──────────────────────────────────────────────────
    let creds = Credentials::new(smtp_user, smtp_pass);

    // Port 465 = direct TLS (SSL), everything else = STARTTLS
    let mailer = if smtp_port == 465 {
        match SmtpTransport::relay(&smtp_host) {
            Ok(b) => b.port(smtp_port).credentials(creds).build(),
            Err(e) => return format!("SMTP relay error: {}", e),
        }
    } else {
        match SmtpTransport::starttls_relay(&smtp_host) {
            Ok(b) => b.port(smtp_port).credentials(creds).build(),
            Err(e) => return format!("SMTP STARTTLS error: {}", e),
        }
    };

    match mailer.send(&email) {
        Ok(_) => {
            info!("Email sent → to='{}' subject='{}'", to, subject);
            format!("✅ Email sent to '{}' (subject: '{}')", to, subject)
        }
        Err(e) => {
            error!("SMTP send failed: {}", e);
            format!(
                "❌ SMTP error: {}\n\n\
                 Check SMTP_HOST / SMTP_USER / SMTP_PASSWORD in .env.\n\
                 See Doc's/email_setup.md for help.",
                e
            )
        }
    }
}

// ── check_inbox ───────────────────────────────────────────────────────────────

/// Return the message count for a folder (defaults to INBOX).
pub fn check_inbox(args: &Value) -> String {
    let folder = args["folder"].as_str().unwrap_or("INBOX");

    let (host, user, pass, port) = match imap_config() {
        Ok(cfg) => cfg,
        Err(msg) => return msg,
    };

    let mut session = match imap_connect(&host, port, &user, &pass) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let result = session.select(folder);
    let _ = session.logout();

    match result {
        Ok(mb) => format!(
            "📬 {} — {} total messages, {} recent",
            folder, mb.exists, mb.recent
        ),
        Err(e) => format!("Error selecting '{}': {}", folder, e),
    }
}

// ── read_email ────────────────────────────────────────────────────────────────

/// Return the last N emails (subject + from + date) from a folder.
///
/// Args:
/// - `folder`  — IMAP folder name (default: `"INBOX"`)
/// - `count`   — number of messages to return (default: `5`, max: `20`)
pub fn read_email(args: &Value) -> String {
    let folder = args["folder"].as_str().unwrap_or("INBOX");
    let count: u32 = args["count"]
        .as_u64()
        .map(|n| n.min(20) as u32)
        .unwrap_or(5);

    let (host, user, pass, port) = match imap_config() {
        Ok(cfg) => cfg,
        Err(msg) => return msg,
    };

    let mut session = match imap_connect(&host, port, &user, &pass) {
        Ok(s) => s,
        Err(e) => return e,
    };

    // Select folder to get total count
    let exists: u32 = match session.select(folder) {
        Ok(mb) => mb.exists,
        Err(e) => {
            let _ = session.logout();
            return format!("Error selecting '{}': {}", folder, e);
        }
    };

    if exists == 0 {
        let _ = session.logout();
        return format!("📭 {} is empty.", folder);
    }

    // Fetch the last `count` messages by sequence number
    let start = if exists > count {
        exists - count + 1
    } else {
        1
    };
    let fetch_range = format!("{}:{}", start, exists);

    let messages = match session.fetch(&fetch_range, "RFC822.HEADER") {
        Ok(m) => m,
        Err(e) => {
            let _ = session.logout();
            return format!("Error fetching messages: {}", e);
        }
    };

    let _ = session.logout();

    if messages.is_empty() {
        return format!("📭 No messages found in {}.", folder);
    }

    let mut lines = vec![format!(
        "📬 {} — showing {} of {} messages (newest first):\n",
        folder,
        messages.len(),
        exists
    )];

    for (i, msg) in messages.iter().rev().enumerate() {
        let headers = msg
            .header()
            .and_then(|b| std::str::from_utf8(b).ok())
            .unwrap_or("");

        let from = extract_header(headers, "From").unwrap_or_else(|| "(unknown sender)".into());
        let subject = extract_header(headers, "Subject").unwrap_or_else(|| "(no subject)".into());
        let date = extract_header(headers, "Date").unwrap_or_else(|| "(no date)".into());

        lines.push(format!(
            "{}. From:    {}\n   Subject: {}\n   Date:    {}",
            i + 1,
            from,
            subject,
            date
        ));
    }

    lines.join("\n")
}

// ── internal helpers ──────────────────────────────────────────────────────────

fn imap_config() -> Result<(String, String, String, u16), String> {
    let host = std::env::var("IMAP_HOST").unwrap_or_default();
    let user = std::env::var("IMAP_USER").unwrap_or_default();
    let pass = std::env::var("IMAP_PASSWORD").unwrap_or_default();
    let port: u16 = std::env::var("IMAP_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(993);

    if host.is_empty() || user.is_empty() || pass.is_empty() {
        return Err("⚠️  IMAP not configured.\n\n\
             Add to .env:\n\
             IMAP_HOST=imap.gmail.com\n\
             IMAP_PORT=993\n\
             IMAP_USER=you@gmail.com\n\
             IMAP_PASSWORD=your-app-password\n\n\
             Gmail: enable 2-FA and use an App Password.\n\
             See Doc's/email_setup.md for full setup instructions."
            .to_string());
    }
    Ok((host, user, pass, port))
}

type ImapSession = imap::Session<native_tls::TlsStream<TcpStream>>;

fn imap_connect(host: &str, port: u16, user: &str, pass: &str) -> Result<ImapSession, String> {
    let tls = native_tls::TlsConnector::builder()
        .build()
        .map_err(|e| format!("TLS error: {}", e))?;

    let client = imap::connect((host, port), host, &tls)
        .map_err(|e| format!("IMAP connect to {}:{} failed: {}", host, port, e))?;

    client
        .login(user, pass)
        .map_err(|(e, _)| format!("IMAP login failed for '{}': {}", user, e))
}

/// Extract a named header value (case-insensitive) from a raw header block.
fn extract_header(headers: &str, name: &str) -> Option<String> {
    let search = format!("{}:", name.to_lowercase());
    for line in headers.lines() {
        if line.to_lowercase().starts_with(&search) {
            return Some(line[search.len()..].trim().to_string());
        }
    }
    None
}

fn append_to_file(path: &str, content: &str) -> std::io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(content.as_bytes())
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_send_email_missing_to() {
        let result = send_email(&json!({"to": "", "subject": "Hi", "body": "Test"}));
        assert!(result.contains("Error"), "got: {}", result);
    }

    #[test]
    fn test_send_email_no_smtp_queues_to_outbox() {
        unsafe {
            std::env::remove_var("SMTP_HOST");
            std::env::remove_var("SMTP_USER");
            std::env::remove_var("SMTP_PASSWORD");
        }
        let result = send_email(&json!({
            "to": "test@example.com",
            "subject": "Test",
            "body": "Hello"
        }));
        assert!(
            result.contains("SMTP not configured") || result.contains("email_outbox"),
            "got: {}",
            result
        );
    }

    #[test]
    fn test_check_inbox_no_config() {
        unsafe {
            std::env::remove_var("IMAP_HOST");
            std::env::remove_var("IMAP_USER");
            std::env::remove_var("IMAP_PASSWORD");
        }
        let result = check_inbox(&json!({}));
        assert!(result.contains("IMAP not configured"), "got: {}", result);
    }

    #[test]
    fn test_read_email_no_config() {
        unsafe {
            std::env::remove_var("IMAP_HOST");
            std::env::remove_var("IMAP_USER");
            std::env::remove_var("IMAP_PASSWORD");
        }
        let result = read_email(&json!({"count": 5}));
        assert!(result.contains("IMAP not configured"), "got: {}", result);
    }

    #[test]
    fn test_extract_header_subject() {
        let raw = "From: alice@example.com\r\nSubject: Hello World\r\nDate: Mon, 1 Jan 2024\r\n";
        assert_eq!(
            extract_header(raw, "Subject"),
            Some("Hello World".to_string())
        );
    }

    #[test]
    fn test_extract_header_missing() {
        let raw = "From: alice@example.com\r\n";
        assert_eq!(extract_header(raw, "X-Missing"), None);
    }
}
