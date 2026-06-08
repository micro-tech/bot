//! Error handling for socket operations (Task 87)
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SocketError {
    #[error("Connection refused")]
    ConnectionRefused,
    #[error("Permission denied")]
    PermissionDenied,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn handle_socket_error(e: std::io::Error) -> SocketError {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => SocketError::PermissionDenied,
        std::io::ErrorKind::ConnectionRefused => SocketError::ConnectionRefused,
        _ => SocketError::Io(e),
    }
}
