use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 && args[1] == "--install" {
        install();
    } else {
        run_bot();
    }
}

fn install() {
    println!("Installing bot...");

    // Assume the binary is built in target/release/installer
    // In practice, this might need adjustment
    let current_exe = env::current_exe().unwrap();
    let install_path = Path::new("/usr/local/bin/bot");

    // Copy binary
    fs::copy(&current_exe, &install_path).expect("Failed to copy binary");

    // Make executable
    Command::new("chmod")
        .arg("+x")
        .arg(&install_path)
        .status()
        .expect("Failed to make executable");

    // Create systemd service file
    let service_content = r#"[Unit]
Description=Bot Service
After=network.target

[Service]
ExecStart=/usr/local/bin/bot
Restart=always
User=cobble  # Change to appropriate user

[Install]
WantedBy=multi-user.target
"#;

    let service_path = Path::new("/etc/systemd/system/bot.service");
    fs::write(&service_path, service_content).expect("Failed to write service file");

    // Enable and start service
    Command::new("systemctl")
        .arg("daemon-reload")
        .status()
        .expect("Failed to reload daemon");

    Command::new("systemctl")
        .arg("enable")
        .arg("bot")
        .status()
        .expect("Failed to enable service");

    Command::new("systemctl")
        .arg("start")
        .arg("bot")
        .status()
        .expect("Failed to start service");

    println!("Bot installed and started as a service.");
}

fn run_bot() {
    println!("Bot is running...");
    loop {
        println!("Bot active");
        std::thread::sleep(std::time::Duration::from_secs(10));
    }
}
