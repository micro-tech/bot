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

    let current_exe = env::current_exe().unwrap();
    let install_path = Path::new("/usr/local/bin/bot");
    let config_dir = Path::new("/etc/bot");
    let config_path = config_dir.join("config.toml");

    // Create config directory
    fs::create_dir_all(config_dir).expect("Failed to create /etc/bot directory");

    // Copy binary
    fs::copy(&current_exe, install_path).expect("Failed to copy binary");

    // Make executable
    Command::new("chmod")
        .arg("+x")
        .arg(install_path)
        .status()
        .expect("Failed to make executable");

    // Copy config.toml if it exists next to the installer
    let source_config = Path::new("config.toml");
    if source_config.exists() {
        fs::copy(source_config, &config_path).expect("Failed to copy config.toml");
        println!("Copied config.toml to {}", config_path.display());
    } else {
        println!("Warning: config.toml not found in current directory. You will need to create one manually.");
    }

    // Create systemd service file with WorkingDirectory set to /etc/bot
    let service_content = format!(
        r#"[Unit]
Description=Bot Service
After=network.target

[Service]
ExecStart=/usr/local/bin/bot
WorkingDirectory=/etc/bot
Restart=always
User=cobble  # Change to appropriate user

[Install]
WantedBy=multi-user.target
"#
    );

    let service_path = Path::new("/etc/systemd/system/bot.service");
    fs::write(service_path, service_content).expect("Failed to write service file");

    // Reload, enable and start
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
    println!("Config file location: {}", config_path.display());
}

fn run_bot() {
    println!("Bot is running...");
    loop {
        println!("Bot active");
        std::thread::sleep(std::time::Duration::from_secs(10));
    }
}
