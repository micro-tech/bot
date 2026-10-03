//! Tool-name namespacing for MCP (Model Context Protocol) tools.
//!
//! Every tool served by an external MCP server is exposed to the agent under
//! the name `mcp__<server>__<tool>`, e.g. `mcp__google__gmail_search`.
//! The double underscore is the separator; server names containing it are
//! rejected at config load (see [`crate::config::mcp::McpConfig::usable_servers`]).

/// Prefix marking a tool as MCP-provided.
pub const PREFIX: &str = "mcp__";

/// Separator between the server name and the upstream tool name.
pub const SEP: &str = "__";

/// Build the agent-visible tool name for an upstream MCP tool.
pub fn tool_name(server: &str, tool: &str) -> String {
    format!("{PREFIX}{server}{SEP}{tool}")
}

/// Split an agent-visible name back into `(server, upstream_tool)`.
/// Returns `None` when the name is not a well-formed MCP tool name.
pub fn parse(name: &str) -> Option<(String, String)> {
    let rest = name.strip_prefix(PREFIX)?;
    let (server, tool) = rest.split_once(SEP)?;
    if server.is_empty() || tool.is_empty() {
        return None;
    }
    Some((server.to_string(), tool.to_string()))
}

/// True when `name` looks like an MCP tool reference.
pub fn is_mcp_tool(name: &str) -> bool {
    parse(name).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let n = tool_name("google", "gmail_search");
        assert_eq!(n, "mcp__google__gmail_search");
        assert_eq!(parse(&n), Some(("google".into(), "gmail_search".into())));
    }

    #[test]
    fn rejects_non_mcp_names() {
        assert_eq!(parse("read_log"), None);
        assert_eq!(parse("mcp_"), None);
        assert_eq!(parse("mcp__"), None);
        assert!(!is_mcp_tool("read_log"));
    }

    #[test]
    fn rejects_empty_parts() {
        assert_eq!(parse("mcp____tool"), None);
        assert_eq!(parse("mcp__server__"), None);
    }

    #[test]
    fn tool_name_may_contain_single_underscores() {
        // Upstream tool names like `gmail_search` contain single underscores;
        // split_once(SEP) keeps them intact.
        let n = tool_name("google", "gmail_search");
        let (server, tool) = parse(&n).expect("must parse");
        assert_eq!(server, "google");
        assert_eq!(tool, "gmail_search");
    }
}
