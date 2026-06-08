//! JSON Lines protocol handler (Task 77)
use serde_json::Value;

pub fn parse_line(line: &str) -> Result<Value, String> {
    serde_json::from_str(line).map_err(|e| e.to_string())
}

pub fn format_response(v: &Value) -> String {
    v.to_string() + "\n"
}
