//! Socket path configuration (Task 76)
pub fn default_socket_path() -> String {
    if cfg!(windows) {
        r"\\.\pipe\bot".to_string()
    } else {
        "/var/run/bot.sock".to_string()
    }
}
