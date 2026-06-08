//! JSON schema validation for CLI commands (Task 111)
use serde_json::Value;

pub fn validate_command(cmd: &str, args: &Value) -> Result<(), String> {
    match cmd {
        "ask" | "chat" => {
            if args.get("prompt").is_none() && args.get("message").is_none() {
                return Err("Missing 'prompt' or 'message' field".to_string());
            }
            Ok(())
        }
        "upload" | "download" => {
            if args.get("path").is_none() {
                return Err("Missing 'path' field".to_string());
            }
            Ok(())
        }
        _ => Ok(()), // Unknown commands pass through for now
    }
}

pub fn validate_cli_message(msg: &Value) -> Result<(), String> {
    if msg.get("cmd").is_none() && msg.get("prompt").is_none() {
        return Err("Message must contain 'cmd' or 'prompt'".to_string());
    }
    Ok(())
}
