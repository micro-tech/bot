//! System-level tools: status, beliefs, and introspection.

use serde_json::Value;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

const BELIEFS_FILE: &str = "beliefs.json";

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
                      send_email, read_email, check_inbox,\n\
                      system_status, list_tools,\n\
                      get_beliefs, set_belief,\n\
                      bayes_show",
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
       • read_log(log_file)               Read the tail of a log file from logs/\n\
       • write_note(title, content)       Save a markdown note to notes/\n\
       • read_note(title)                 Read a saved note\n\
       • list_notes()                     List all saved notes\n\
     \n\
     Communication:\n\
       • send_email(to, subject, body)    Send email via SMTP (falls back to outbox)\n\
       • read_email([folder, count])      Read recent emails via IMAP\n\
       • check_inbox([folder])            Check message count in a folder\n\
     \n\
     System:\n\
       • system_status()                  Log sizes, note/belief counts\n\
       • list_tools()                     This list\n\
     \n\
     Memory / beliefs:\n\
       • get_beliefs()                    Read all beliefs from beliefs.json\n\
       • set_belief(key, value)           Store a belief (persists across restarts)\n\
     \n\
     Bayesian reasoning:\n\
       • bayes_show()                     Show current Bayesian belief state\n\
     \n\
     Slash commands in chat (type directly):\n\
       /status   /tools   /notes   /beliefs   /log [file]   /help\n\
       /note <title>   /set <key>=<value>   /bayes [show|status|update <ev>]"
        .to_string()
}

// ── bayes_show ───────────────────────────────────────────────────────────────

const BAYES_STATE_FILE: &str = "beliefs_bayes.json";

/// Show the current Bayesian reasoner belief state.
/// Reads persisted state from `beliefs_bayes.json` if it exists,
/// otherwise returns the default priors.
pub fn bayes_show() -> String {
    let state: Option<serde_json::Map<String, Value>> = fs::read_to_string(BAYES_STATE_FILE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());

    match state {
        Some(map) if !map.is_empty() => {
            let mut lines = vec!["🧠  Bayesian Belief State (persisted):".to_string()];
            // Find top belief
            let mut entries: Vec<(&String, f64)> = map
                .iter()
                .filter_map(|(k, v)| v.as_f64().map(|f| (k, f)))
                .collect();
            entries.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            for (k, v) in &entries {
                let bar_len = (v * 20.0).round() as usize;
                let bar = "#".repeat(bar_len);
                lines.push(format!("  {:<12} {:.1}%  [{}]", k, v * 100.0, bar));
            }
            if let Some((top_k, top_v)) = entries.first() {
                lines.push(format!("\nTop belief: {} ({:.1}%)", top_k, top_v * 100.0));
            }
            lines.push(
                "\nUse /bayes update <evidence> to refine. Use /bayes reset to restart."
                    .to_string(),
            );
            lines.join("\n")
        }
        _ => "🧠  Bayesian Belief State (defaults — no updates applied yet):\n\
              positive    50.0%  [##########]\n\
              negative    30.0%  [######]\n\
              neutral     20.0%  [####]\n\
             \nTop belief: positive (50.0%)\n\
             Use /bayes update <evidence> to refine."
            .to_string(),
    }
}

/// Update the Bayesian state and persist it to `beliefs_bayes.json`.
pub fn bayes_update(evidence: &str) -> String {
    // Load or default
    let mut beliefs: std::collections::HashMap<String, f64> = fs::read_to_string(BAYES_STATE_FILE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| {
            let mut m = std::collections::HashMap::new();
            m.insert("positive".to_string(), 0.5);
            m.insert("negative".to_string(), 0.3);
            m.insert("neutral".to_string(), 0.2);
            m
        });

    // Simple likelihood bump: evidence nudges the top category
    // A real implementation would take explicit likelihoods; this is a
    // user-friendly approximation: positive evidence boosts "positive", etc.
    let ev_lower = evidence.to_lowercase();
    let bump_key =
        if ev_lower.contains("pos") || ev_lower.contains("good") || ev_lower.contains("yes") {
            "positive"
        } else if ev_lower.contains("neg") || ev_lower.contains("bad") || ev_lower.contains("no") {
            "negative"
        } else {
            "neutral"
        };

    // Bayesian update with fixed likelihoods: bump_key = 0.8, others = 0.1
    let likelihoods: std::collections::HashMap<&str, f64> = [
        ("positive", if bump_key == "positive" { 0.8 } else { 0.1 }),
        ("negative", if bump_key == "negative" { 0.8 } else { 0.1 }),
        ("neutral", if bump_key == "neutral" { 0.8 } else { 0.1 }),
    ]
    .into_iter()
    .collect();

    let evidence_prob: f64 = beliefs
        .iter()
        .map(|(h, prior)| likelihoods.get(h.as_str()).copied().unwrap_or(0.1) * prior)
        .sum::<f64>()
        .max(1e-10);

    for (hyp, prior) in beliefs.iter_mut() {
        let lik = likelihoods.get(hyp.as_str()).copied().unwrap_or(0.1);
        *prior = (lik * *prior) / evidence_prob;
    }

    // Normalise
    let total: f64 = beliefs.values().sum::<f64>().max(1e-10);
    for v in beliefs.values_mut() {
        *v /= total;
    }

    // Persist
    let json_val: Value =
        serde_json::to_value(&beliefs).unwrap_or(Value::Object(Default::default()));
    let pretty = serde_json::to_string_pretty(&json_val).unwrap_or_default();
    let _ = fs::write(BAYES_STATE_FILE, &pretty);

    // Find top
    let top = beliefs
        .iter()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(k, v)| format!("{} ({:.1}%)", k, v * 100.0))
        .unwrap_or_default();

    format!(
        "✅ Bayesian update applied for evidence '{}'.\nTop belief is now: {}\nUse /bayes show to see full state.",
        evidence, top
    )
}

/// Reset the Bayesian state back to default priors.
pub fn bayes_reset() -> String {
    match fs::remove_file(BAYES_STATE_FILE) {
        Ok(_) => "🔄 Bayesian state reset to defaults (positive=50%, negative=30%, neutral=20%)."
            .to_string(),
        Err(_) => "Bayesian state was already at defaults (no persisted state found).".to_string(),
    }
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

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_list_tools_contains_all_tools() {
        let result = list_tools();
        for tool in &[
            "read_log",
            "write_note",
            "get_beliefs",
            "system_status",
            "send_email",
            "read_email",
            "check_inbox",
        ] {
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

    #[test]
    fn test_system_status_mentions_email_tools() {
        let result = system_status();
        assert!(result.contains("read_email"), "got: {}", result);
        assert!(result.contains("check_inbox"), "got: {}", result);
    }
}
