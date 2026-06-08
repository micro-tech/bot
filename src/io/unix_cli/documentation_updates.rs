//! Documentation updates (Task 90)
pub const CLI_DOC: &str = r#"
Bot CLI Commands:
  help, ping, status
  ask, chat
  upload, download
  tools, memory-*, log-stream
"#;

pub fn print_cli_help() {
    println!("{}", CLI_DOC);
}
