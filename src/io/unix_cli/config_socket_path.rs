//! Socket path configuration (Task 76)
pub fn default_socket_path() -> String {
    if cfg!(windows) {
        r"\\.\pipe\helix".to_string()
    } else {
        "/var/run/helix.sock".to_string()
    }
}
