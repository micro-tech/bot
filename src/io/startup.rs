// Task 82: Add socket server to main startup
use crate::io::unix_socket::UnixSocketServer;
use crate::config::socket::SocketConfig;
use crate::bus::Bus;

pub async fn start_unix_socket(bus: Bus, config: SocketConfig) {
    let server = UnixSocketServer::new(config, bus);
    if let Err(e) = server.run().await {
        eprintln!("UNIX socket server failed: {}", e);
    }
}
