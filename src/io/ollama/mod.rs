// Ollama Handler with Tool Calling
use crate::bus::{Bus, Message};
use log::info;
use reqwest::Client;
use serde_json::{json, Value};


pub fn handle_ollama_message(message: Message, bus: &mut Bus) -> Option<String> {
    info!("Ollama msg: {}", message.data);
    let tools = vec![
        json!({
            "type": "function",
            "function": {
                "name": "read_log",
                "description": "Read project logs",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "log_file": {"type": "string"}
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "send_email",
                "description": "Send alert email",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "to": {"type": "string"},
                        "subject": {"type": "string"},
                        "body": {"type": "string"}
                    }
                }
            }
        })
    ];
    call_ollama_tools(&message.data, json!(tools)).ok()
}

fn call_ollama_tools(prompt: &str, tools: Value) -> Result<String, Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let client = Client::new();
        let mut messages = vec![json!({"role": "user", "content": prompt})];
        loop {
            let resp = client.post("http://192.168.1.149:11434/api/chat")
                .json(&json!({
                    "model": "llama3",
                    "messages": messages,
                    "tools": tools
                }))
                .send()
                .await?
                .json::<Value>()
                .await?;

            let msg = &resp["message"];
            messages.push(msg.clone());

            if let Some(tool_calls) = msg["tool_calls"].as_array() {
                for tool in tool_calls {
                    let name = tool["function"]["name"].as_str().unwrap_or("");
                    let args = tool["function"]["arguments"].clone();
                    let tool_resp = execute_tool(name, args);
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": tool["id"],
                        "content": tool_resp
                    }));
                }
            } else {
                return Ok(msg["content"].as_str().unwrap_or("").to_string());
            }
        }
    })
}

fn execute_tool(name: &str, args: Value) -> String {
    match name {
        "read_log" => {
            let file = args["log_file"].as_str().unwrap_or("logs/bus_log.md");
            std::fs::read_to_string(file).unwrap_or("Log not found".to_string())
        },
        "send_email" => {
            // Placeholder
            "Email sent".to_string()
        },
        _ => "Unknown tool".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Bus;

    #[test]
    fn test_handle() {
        let mut bus = Bus::new();
        let message = Message {
            to: "ollama".to_string(),
            from: "test".to_string(),
            data: "Read the bus log".to_string(),
            timestamp: 0,
        };
        let response = handle_ollama_message(message, &mut bus);
        assert!(response.is_some());
    }

    #[test]
    fn test_tool_exec() {
        let args = json!({"log_file": "logs/bus_log.md"});
        let result = execute_tool("read_log", args);
        assert!(!result.is_empty());
    }
}