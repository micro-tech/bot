//! System-level tools: status, beliefs, email, introspection.

use log::info;
use serde_json::Value;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

const BELIEFS_FILE: &str = "beliefs.json";

// ── send_email ────────────────────────────────────────────────────────────────

/// Placeholder email tool.  Logs the request; does not actually send.
/// TODO: wire up `lettre` or a SendGrid / Mailgun webhook.
pub fn send_email(args: &Value) -> String {
    let to = args["to"].as_str().unwrap_or("(none)");
    let subject = args["subject"].as_str().unwrap_or("(no subject)");
    let body = args["body"].as_str().unwrap_or("");

    info!(
        "send_email tool called → to='{}' subject='{}' body_len={} (placeholder, not sent)",
        to,
        subject,
        body.len()
    );

    // Append to a simple outbox file so nothing is truly lost
    let entry = format!("---\nTo: {}\nSubject: {}\n\n{}\n", to, subject, body);
    let _ = fs::create_dir_all("logs");
    let _ = append_to_file("logs/email_outbox.md", &entry);

    format!(
        "📧 Email to '{}' (subject: '{}') queued in logs/email_outbox.md — \
         SMTP not yet configured.",
        to, subject
    )
}

// ── system_status ─────────────────────────────────────────────────────────────

pub fn system_status() -> String {
    let unix_now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Log file sizes
    let log_files = [
        "logs/chat_log.md",
        "logs/error_log.md",
        "logs/email_outbox.md",
    ];
    let log_info: Vec<String> = log_files
        .iter()
        .map(|p| match fs::metadata(p) {
            Ok(m) => format!("  • {} — {} bytes", p, m.len()),
            Err(_) => format!("  • {} — not found", p),
        })
        .collect();

    // Note count
    let note_count = fs::read_dir("notes")
        .map(|d| d.flatten().count())
        .unwrap_or(0);

    // Beliefs count
    let belief_count = fs::read_to_string(BELIEFS_FILE)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Map<String, Value>>(&s).ok())
        .map(|m| m.len())
        .unwrap_or(0);

    format!(
        "🖥️  System Status\n\
         Unix timestamp : {}\n\n\
         Log files:\n{}\n\n\
         Notes saved    : {}\n\
         Beliefs stored : {}\n\n\
         Tools available: read_log, write_note, read_note, list_notes,\n\
                          send_email, system_status, list_tools,\n\
                          get_beliefs, set_belief",
        unix_now,
        log_info.join("\n"),
        note_count,
        belief_count
    )
}

// ── list_tools ────────────────────────────────────────────────────────────────

pub fn list_tools() -> String {
    "🛠️  Available tools & skills:\n\
     \n\
     File tools:\n\
       • read_log(log_file)           Read the tail of a log file from logs/\n\
       • write_note(title, content)   Save a markdown note to notes/\n\
       • read_note(title)             Read a saved note\n\
       • list_notes()                 List all saved notes\n\
     \n\
     Communication:\n\
       • send_email(to, subject, body)  Queue an email (logs/email_outbox.md)\n\
     \n\
     System:\n\
       • system_status()              Log sizes, note/belief counts\n\
       • list_tools()                 This list\n\
     \n\
     Memory / beliefs:\n\
       • get_beliefs()                Read all beliefs from beliefs.json\n\
       • set_belief(key, value)       Store a belief (persists across restarts)\n\
     \n\
     Slash commands in chat (type directly):\n\
       /status   /tools   /notes   /beliefs\n\
       /note <title>   /set <key>=<value>"
        .to_string()
}

// ── get_beliefs ───────────────────────────────────────────────────────────────

pub fn get_beliefs() -> String {
    match fs::read_to_string(BELIEFS_FILE) {
        Ok(content) if !content.trim().is_empty() => {
            match serde_json::from_str::<Value>(&content) {
                Ok(json) => serde_json::to_string_pretty(&json).unwrap_or_else(|_| content),
                Err(_) => content,
            }
        }
        _ => "{}  (no beliefs stored yet — use set_belief to add some)".to_string(),
    }
}

// ── set_belief ────────────────────────────────────────────────────────────────

pub fn set_belief(args: &Value) -> String {
    let key = args["key"].as_str().unwrap_or("").trim().to_string();
    let value = args["value"].as_str().unwrap_or("").to_string();

    if key.is_empty() {
        return "Error: 'key' field is required and must not be empty.".to_string();
    }

    // Load existing beliefs
    let mut beliefs: serde_json::Map<String, Value> = fs::read_to_string(BELIEFS_FILE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    beliefs.insert(key.clone(), Value::String(value.clone()));

    let pretty =
        serde_json::to_string_pretty(&Value::Object(beliefs)).unwrap_or_else(|_| "{}".to_string());

    match fs::write(BELIEFS_FILE, &pretty) {
        Ok(_) => format!(
            "✅ Belief '{}' = '{}' saved to {}",
            key, value, BELIEFS_FILE
        ),
        Err(e) => format!("Error saving belief: {}", e),
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn append_to_file(path: &str, content: &str) -> std::io::Result<()> {
    use std::io::Write;
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
    fn test_list_tools_contains_all_tools() {
        let result = list_tools();
        for tool in &["read_log", "write_note", "get_beliefs", "system_status"] {
            assert!(
                result.contains(tool),
                "missing '{}' in list_tools output",
                tool
            );
        }
    }

    #[test]
    fn test_get_beliefs_does_not_panic() {
        // beliefs.json may or may not exist
        let _ = get_beliefs();
    }

    #[test]
    fn test_set_belief_missing_key() {
        let result = set_belief(&json!({"key": "", "value": "test"}));
        assert!(result.contains("Error"), "got: {}", result);
    }

    #[test]
    fn test_system_status_does_not_panic() {
        let _ = system_status();
    }
}
