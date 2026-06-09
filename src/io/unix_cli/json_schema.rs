//! JSON schema validation for CLI commands (Task 111)
use crate::utils::log_to_file;
use serde_json::Value;

pub fn validate_command(cmd: &str, args: &Value) -> Result<(), String> {
    match cmd {
        "ask" | "chat" => {
            if args.get("prompt").is_none() && args.get("message").is_none() {
                let msg = "Missing 'prompt' or 'message' field".to_string();
                log_to_file(&format!("Schema validation failed for {}: {}", cmd, msg));
                return Err(msg);
            }
            Ok(())
        }
        "upload" | "download" => {
            if args.get("path").is_none() {
                let msg = "Missing 'path' field".to_string();
                log_to_file(&format!("Schema validation failed for {}: {}", cmd, msg));
                return Err(msg);
            }
            Ok(())
        }
        _ => Ok(()), // Unknown commands pass through for now
    }
}

pub fn validate_cli_message(msg: &Value) -> Result<(), String> {
    if msg.get("cmd").is_none() && msg.get("prompt").is_none() {
        let msg = "Message must contain 'cmd' or 'prompt'".to_string();
        log_to_file(&msg);
        return Err(msg);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_validate_ask_missing_prompt() {
        let args = json!({});
        assert!(validate_command("ask", &args).is_err());
    }

    #[test]
    fn test_validate_ask_with_prompt() {
        let args = json!({"prompt": "hello"});
        assert!(validate_command("ask", &args).is_ok());
    }

    #[test]
    fn test_validate_upload_missing_path() {
        let args = json!({});
        assert!(validate_command("upload", &args).is_err());
    }
}
