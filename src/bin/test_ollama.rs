use std::net::TcpStream;
use std::process::Command;
use std::time::Duration;

fn ping_host(host: &str) -> bool {
    let output = Command::new("ping").args(["-n", "1", host]).output();

    match output {
        Ok(out) => out.status.success(),
        Err(_) => false,
    }
}

fn check_port(host: &str, port: u16) -> bool {
    let addr = format!("{}:{}", host, port);
    TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_secs(2)).is_ok()
}

fn http_get(url: &str) -> Result<String, reqwest::Error> {
    let client = reqwest::blocking::Client::new();
    let resp = client.get(url).send()?.text()?;
    Ok(resp)
}

fn main() {
    let host = "192.168.1.149";
    let port = 11434;

    println!("🔍 Checking Ollama at {}:{}\n", host, port);

    // 1. Ping
    println!("1️⃣  Pinging host...");
    if ping_host(host) {
        println!("   ✅ Host reachable\n");
    } else {
        println!("   ❌ Host unreachable (ping failed)\n");
    }

    // 2. Port test
    println!("2️⃣  Checking TCP port {}...", port);
    if check_port(host, port) {
        println!("   ✅ Port open\n");
    } else {
        println!("   ❌ Port closed or blocked\n");
        return;
    }

    // 3. API test
    let base = format!("http://{}:{}", host, port);

    println!("3️⃣  Checking /api/tags...");
    match http_get(&format!("{}/api/tags", base)) {
        Ok(body) => println!("   ✅ API OK\n{}\n", body),
        Err(e) => println!("   ❌ API error: {}\n", e),
    }

    println!("4️⃣  Checking /v1/models...");
    match http_get(&format!("{}/v1/models", base)) {
        Ok(body) => println!("   ✅ OpenAI endpoint OK\n{}\n", body),
        Err(e) => println!("   ❌ OpenAI endpoint error: {}\n", e),
    }
}
