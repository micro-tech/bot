//! Read-only LAN discovery for the full network map (tasks 344–347).
//!
//! Gateway detection parses `/proc/net/route` (the default route is the
//! Starlink router, the map's root node); ARP neighbours come from
//! `/proc/net/arp`. The ping sweep (345) runs the `ping` binary by direct
//! exec (argv only, no shell) — the binary carries the file caps for raw
//! sockets, so Helix itself needs no `CAP_NET_RAW`. Hostname resolution
//! (346) is best-effort: getent, then mDNS, then NetBIOS, each probed at
//! runtime and never required.
//!
//! Everything here is read-only except the ICMP echo sweep, which sends
//! echo requests on the LAN /24 only, when the map tool is called. No
//! port scanning, no service probing, no config changes, no timers.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// One ARP neighbour: a recently-seen LAN device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArpEntry {
    /// IPv4 address, e.g. "192.168.1.196".
    pub ip: String,
    /// Hardware (MAC) address, e.g. "aa:bb:cc:dd:ee:ff".
    pub mac: String,
    /// Interface the entry was seen on, e.g. "ens18".
    pub interface: String,
    /// ARP flags field as printed (e.g. "0x2" = complete).
    pub flags: String,
}

/// How a host was found. A host found several ways carries several tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiscoveryMethod {
    Arp,
    Ping,
    Configured,
    Capture,
}

impl DiscoveryMethod {
    pub fn tag(self) -> &'static str {
        match self {
            DiscoveryMethod::Arp => "arp",
            DiscoveryMethod::Ping => "ping",
            DiscoveryMethod::Configured => "configured",
            DiscoveryMethod::Capture => "capture",
        }
    }
}

/// One node of the full network map, merged across discovery methods.
#[derive(Debug, Clone)]
pub struct DiscoveredHost {
    pub ip: String,
    pub mac: Option<String>,
    pub interface: Option<String>,
    pub hostname: Option<String>,
    pub methods: Vec<DiscoveryMethod>,
    /// True when this IP is one of Helix's own addresses ("this machine").
    pub is_helix: bool,
}

/// Read-only discovery snapshot: default gateway + ARP neighbours.
#[derive(Debug, Clone, Default)]
pub struct NetworkDiscovery {
    /// Default gateway IP (the Starlink router), if one was found.
    pub gateway: Option<String>,
    /// Recently-seen ARP neighbours (incomplete / loopback entries skipped).
    pub arp_entries: Vec<ArpEntry>,
}

/// Hosts Helix knows from its own config, shown on the map even when they
/// answer neither ARP nor ping.
pub struct ConfiguredHost {
    pub ip: &'static str,
    pub name: &'static str,
}

pub const KNOWN_CONFIG_HOSTS: &[ConfiguredHost] = &[
    ConfiguredHost {
        ip: "192.168.1.149",
        name: "Ollama server (gpt-oss:20b)",
    },
    ConfiguredHost {
        ip: "192.168.1.196",
        name: "desktop (gemma4:e4b)",
    },
];

// ── Gateway + ARP (task 344) ────────────────────────────────────────────────

/// Convert a `/proc/net/route` hex gateway (little-endian, e.g.
/// "0101A8C0") to dotted quad ("192.168.1.1"). Returns None for
/// "00000000" (no gateway).
fn hex_le_to_ipv4(hex: &str) -> Option<String> {
    let v = u32::from_str_radix(hex, 16).ok()?;
    if v == 0 {
        return None;
    }
    Some(format!(
        "{}.{}.{}.{}",
        v & 0xff,
        (v >> 8) & 0xff,
        (v >> 16) & 0xff,
        (v >> 24) & 0xff
    ))
}

/// Parse `/proc/net/route` content. The default route is the row whose
/// destination is 00000000; its gateway column is the router IP.
pub fn parse_proc_net_route(content: &str) -> Option<String> {
    for line in content.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 3 {
            continue;
        }
        if cols[1] != "00000000" {
            continue;
        }
        if let Some(gw) = hex_le_to_ipv4(cols[2]) {
            return Some(gw);
        }
    }
    None
}

/// Parse `ip route show default` output ("default via 192.168.1.1 dev
/// ..."). Fallback only — used when `/proc/net/route` is unreadable.
pub fn parse_ip_route_default(content: &str) -> Option<String> {
    for line in content.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.first() != Some(&"default") {
            continue;
        }
        if let Some(pos) = cols.iter().position(|c| *c == "via") {
            if let Some(ip) = cols.get(pos + 1) {
                if !ip.is_empty() {
                    return Some(ip.to_string());
                }
            }
        }
    }
    None
}

/// Parse `/proc/net/arp` content into neighbour entries. Skips incomplete
/// entries (MAC 00:00:00:00:00:00) and the loopback interface.
pub fn parse_proc_net_arp(content: &str) -> Vec<ArpEntry> {
    let mut out = Vec::new();
    for line in content.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 6 {
            continue;
        }
        let (ip, flags, mac, device) = (cols[0], cols[2], cols[3], cols[5]);
        if mac.eq_ignore_ascii_case("00:00:00:00:00:00") {
            continue;
        }
        if device == "lo" {
            continue;
        }
        out.push(ArpEntry {
            ip: ip.to_string(),
            mac: mac.to_string(),
            interface: device.to_string(),
            flags: flags.to_string(),
        });
    }
    out
}

/// Read-only discovery: default gateway + ARP neighbours. Pure `/proc`
/// reads; the `ip` fallback runs only if `/proc/net/route` is unreadable.
pub fn discover() -> NetworkDiscovery {
    let gateway = match std::fs::read_to_string("/proc/net/route") {
        Ok(content) => parse_proc_net_route(&content),
        Err(_) => gateway_from_ip_route(),
    };
    let arp_entries = std::fs::read_to_string("/proc/net/arp")
        .map(|c| parse_proc_net_arp(&c))
        .unwrap_or_default();
    NetworkDiscovery {
        gateway,
        arp_entries,
    }
}

/// Direct exec of `ip route show default` (argv only, no shell).
fn gateway_from_ip_route() -> Option<String> {
    let out = std::process::Command::new("ip")
        .args(["route", "show", "default"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_ip_route_default(&String::from_utf8_lossy(&out.stdout))
}

/// First three octets of an IPv4 address ("192.168.1.1" -> "192.168.1"),
/// the /24 the sweep covers. Returns None for anything that is not a
/// dotted-quad IPv4 address.
pub fn subnet_base(ip: &str) -> Option<String> {
    let parts: Vec<&str> = ip.split('.').collect();
    if parts.len() != 4 || parts.iter().any(|p| p.is_empty() || p.parse::<u8>().is_err()) {
        return None;
    }
    Some(format!("{}.{}.{}", parts[0], parts[1], parts[2]))
}

// ── Ping sweep (task 345) ───────────────────────────────────────────────────

/// Maximum parallel ping probes (task 345 cap: 32).
pub const SWEEP_WORKERS: usize = 32;

/// Probe one host with a single ICMP echo. Direct exec of the `ping`
/// binary (argv only, no shell, no user-controlled input — the IP comes
/// from our own computed /24 range). `ping` carries the file caps for
/// raw sockets, so Helix needs no `CAP_NET_RAW`. Returns true iff a
/// reply was received.
pub fn ping_host(ip: &str) -> bool {
    std::process::Command::new("ping")
        .args(["-c", "1", "-W", "1", ip])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Sweep a list of IPs with bounded concurrency. `probe` is the per-host
/// check (production: `ping_host`; tests inject a fake). Every host gets
/// one probe plus one re-ping on failure (task cap: no retries beyond one
/// re-ping). Returns the IPs that answered, sorted.
pub fn ping_sweep_ips<F: Fn(&str) -> bool + Sync>(ips: &[String], probe: F) -> Vec<String> {
    if ips.is_empty() {
        return Vec::new();
    }
    let live: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let next = AtomicUsize::new(0);
    let workers = SWEEP_WORKERS.min(ips.len());
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= ips.len() {
                    break;
                }
                let ip = &ips[i];
                if probe(ip) || probe(ip) {
                    live.lock().unwrap().push(ip.clone());
                }
            });
        }
    });
    let mut out = live.into_inner().unwrap();
    out.sort();
    out
}

/// Sweep one address range of a /24 ("192.168.1", 1..=254).
pub fn ping_sweep_range(base: &str, start: u8, end: u8) -> Vec<String> {
    let ips: Vec<String> = (start..=end).map(|i| format!("{}.{}", base, i)).collect();
    ping_sweep_ips(&ips, ping_host)
}

/// Sweep a whole /24 (.1–.254). Wall time is bounded by 32-way
/// concurrency and the 1s per-probe timeout: worst case ~16s (two passes
/// over a fully silent /24), under the 30s cap.
pub fn ping_sweep_subnet(base: &str) -> Vec<String> {
    ping_sweep_range(base, 1, 254)
}

// ── Hostname resolution (task 346) ──────────────────────────────────────────

/// Run a command with a wall-clock timeout. The child keeps running past
/// the timeout (the thread is abandoned, not killed) — acceptable for the
/// handful of short local lookups a map does; the caller never blocks
/// longer than `timeout`.
fn run_with_timeout(argv: &[&str], timeout: Duration) -> Option<String> {
    if argv.is_empty() {
        return None;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    std::thread::spawn(move || {
        let out = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output();
        let _ = tx.send(out.ok().and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).into_owned())
            } else {
                None
            }
        }));
    });
    rx.recv_timeout(timeout).ok().flatten()
}

/// Reverse DNS via NSS (`getent hosts <ip>` honours /etc/hosts and DNS).
/// Returns the first hostname, or None.
fn reverse_dns(ip: &str) -> Option<String> {
    let out = run_with_timeout(&["getent", "hosts", ip], Duration::from_secs(3))?;
    let mut it = out.split_whitespace();
    let _addr = it.next()?;
    it.next().map(|s| s.to_string())
}

/// mDNS via avahi (`avahi-resolve-address <ip>` -> "ip\tname.local").
/// Binary probed at runtime; absent -> None.
fn mdns_name(ip: &str) -> Option<String> {
    let out = run_with_timeout(&["avahi-resolve-address", ip], Duration::from_secs(3))?;
    let mut it = out.split_whitespace();
    let _addr = it.next()?;
    it.next().map(|s| s.to_string())
}

/// NetBIOS via nmblookup (`nmblookup -A <ip>`). First "<00>" name that
/// is not a <GROUP> entry. Binary probed at runtime; absent -> None.
fn netbios_name(ip: &str) -> Option<String> {
    let out = run_with_timeout(&["nmblookup", "-A", ip], Duration::from_secs(3))?;
    for line in out.lines() {
        if !line.contains("<00>") || line.contains("GROUP") {
            continue;
        }
        if let Some(name) = line.split_whitespace().next() {
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Best-effort hostname for one IP: reverse DNS, then mDNS, then
/// NetBIOS. Every failure returns None — a host without a name still
/// renders with IP + MAC.
pub fn resolve_hostname(ip: &str) -> Option<String> {
    reverse_dns(ip)
        .or_else(|| mdns_name(ip))
        .or_else(|| netbios_name(ip))
}

/// Resolve each IP once (the returned map IS the cache — one map call
/// never re-resolves the same IP). Parallel, 16 workers, 3s per lookup.
pub fn resolve_hostnames(ips: &[String]) -> HashMap<String, Option<String>> {
    if ips.is_empty() {
        return HashMap::new();
    }
    let out: Mutex<HashMap<String, Option<String>>> = Mutex::new(HashMap::new());
    let next = AtomicUsize::new(0);
    let workers = 16usize.min(ips.len());
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= ips.len() {
                    break;
                }
                let ip = &ips[i];
                let name = resolve_hostname(ip);
                out.lock().unwrap().insert(ip.clone(), name);
            });
        }
    });
    out.into_inner().unwrap()
}

// ── Aggregation (task 347) ──────────────────────────────────────────────────

/// Numeric sort key for an IPv4 address ("192.168.1.100" sorts after
/// "192.168.1.99", which a plain string sort gets wrong).
fn ip_sort_key(ip: &str) -> [u8; 4] {
    let mut key = [0u8; 4];
    for (i, part) in ip.split('.').enumerate().take(4) {
        key[i] = part.parse().unwrap_or(0);
    }
    key
}

/// Merge gateway + ARP + ping + hostnames into one node list, deduped by
/// IP. A host found by ARP and ping is one node with both tags.
/// Configured hosts are added with the `configured` tag even when they
/// answer neither (skipped only when they are on a different /24 than
/// the gateway). The gateway itself is NOT a device node (it is the
/// root, rendered separately). `helix_ips` marks "this machine".
pub fn aggregate_hosts(
    gateway: Option<&str>,
    arp: &[ArpEntry],
    ping_live: &[String],
    hostnames: &HashMap<String, Option<String>>,
    helix_ips: &[String],
) -> Vec<DiscoveredHost> {
    let mut by_ip: HashMap<String, DiscoveredHost> = HashMap::new();
    let add_method = |h: &mut DiscoveredHost, m: DiscoveryMethod| {
        if !h.methods.contains(&m) {
            h.methods.push(m);
        }
    };
    let new_host = |ip: &str| DiscoveredHost {
        ip: ip.to_string(),
        mac: None,
        interface: None,
        hostname: hostnames.get(ip).and_then(|h| h.clone()),
        methods: Vec::new(),
        is_helix: helix_ips.iter().any(|x| x == ip),
    };
    for e in arp {
        if Some(e.ip.as_str()) == gateway {
            continue;
        }
        let h = by_ip.entry(e.ip.clone()).or_insert_with(|| new_host(&e.ip));
        h.mac = Some(e.mac.clone());
        h.interface = Some(e.interface.clone());
        add_method(h, DiscoveryMethod::Arp);
    }
    for ip in ping_live {
        if Some(ip.as_str()) == gateway {
            continue;
        }
        let h = by_ip.entry(ip.clone()).or_insert_with(|| new_host(ip));
        add_method(h, DiscoveryMethod::Ping);
    }
    let gw_base = gateway.and_then(subnet_base);
    for c in KNOWN_CONFIG_HOSTS {
        if let Some(base) = &gw_base {
            if subnet_base(c.ip).as_deref() != Some(base.as_str()) {
                continue;
            }
        }
        let h = by_ip.entry(c.ip.to_string()).or_insert_with(|| new_host(c.ip));
        if h.hostname.is_none() {
            h.hostname = Some(c.name.to_string());
        }
        add_method(h, DiscoveryMethod::Configured);
    }
    let mut hosts: Vec<DiscoveredHost> = by_ip.into_values().collect();
    hosts.sort_by_key(|h| ip_sort_key(&h.ip));
    hosts
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROUTE_FIXTURE: &str = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\nens18\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\nens18\t0001A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n";

    const ARP_FIXTURE: &str = "IP address       HW type     Flags       HW address            Mask     Device\n192.168.1.1      0x1         0x2         aa:bb:cc:dd:ee:01     *        ens18\n192.168.1.196    0x1         0x2         aa:bb:cc:dd:ee:02     *        ens18\n192.168.1.200    0x1         0x0         00:00:00:00:00:00     *        ens18\n127.0.0.1        0x1         0x2         aa:bb:cc:dd:ee:03     *        lo\n";

    // ── Task 344 ────────────────────────────────────────────────────────────

    #[test]
    fn gateway_parses_default_route() {
        // 0101A8C0 is little-endian hex for 192.168.1.1.
        assert_eq!(
            parse_proc_net_route(ROUTE_FIXTURE).as_deref(),
            Some("192.168.1.1")
        );
    }

    #[test]
    fn gateway_none_without_default_route() {
        let no_default = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\nens18\t0001A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n";
        assert_eq!(parse_proc_net_route(no_default), None);
    }

    #[test]
    fn ip_route_default_parses() {
        assert_eq!(
            parse_ip_route_default("default via 192.168.1.1 dev ens18 proto dhcp metric 100\n")
                .as_deref(),
            Some("192.168.1.1")
        );
        assert_eq!(
            parse_ip_route_default(
                "192.168.1.0/24 dev ens18 proto kernel scope link src 192.168.1.168\n"
            ),
            None
        );
    }

    #[test]
    fn arp_parses_and_skips_incomplete_and_loopback() {
        let entries = parse_proc_net_arp(ARP_FIXTURE);
        assert_eq!(entries.len(), 2, "incomplete-MAC and lo rows must be skipped");
        assert_eq!(entries[0].ip, "192.168.1.1");
        assert_eq!(entries[0].mac, "aa:bb:cc:dd:ee:01");
        assert_eq!(entries[0].interface, "ens18");
        assert_eq!(entries[0].flags, "0x2");
        assert_eq!(entries[1].ip, "192.168.1.196");
        assert_eq!(entries[1].mac, "aa:bb:cc:dd:ee:02");
    }

    #[test]
    fn discover_reads_live_proc_files_without_panicking() {
        // Smoke test against the real /proc files in this environment:
        // must not panic, and any entries returned must be well-formed.
        let d = discover();
        assert!(d
            .arp_entries
            .iter()
            .all(|e| !e.ip.is_empty() && !e.mac.is_empty() && !e.interface.is_empty()));
    }

    #[test]
    fn subnet_base_validates_octets() {
        assert_eq!(subnet_base("192.168.1.1").as_deref(), Some("192.168.1"));
        assert_eq!(subnet_base("10.0.0.5").as_deref(), Some("10.0.0"));
        assert_eq!(subnet_base("not-an-ip"), None);
        assert_eq!(subnet_base("192.168.1.999"), None);
        assert_eq!(subnet_base("192.168.1"), None);
    }

    // ── Task 345 ────────────────────────────────────────────────────────────

    #[test]
    fn ping_sweep_finds_live_hosts_with_fake_probe() {
        let ips = vec![
            "127.0.0.1".to_string(),
            "127.0.0.2".to_string(),
            "192.0.2.1".to_string(),
        ];
        let live = ping_sweep_ips(&ips, |ip| ip == "127.0.0.1");
        assert_eq!(live, vec!["127.0.0.1".to_string()]);
    }

    #[test]
    fn ping_sweep_reprobes_silent_hosts_once() {
        // A host that answers only on the second probe is still found,
        // and is probed exactly twice (one re-ping, no more).
        let attempts = Mutex::new(HashMap::new());
        let live = ping_sweep_ips(&["10.0.0.5".to_string()], |ip: &str| {
            let mut m = attempts.lock().unwrap();
            let n = m.entry(ip.to_string()).or_insert(0);
            *n += 1;
            *n >= 2
        });
        assert_eq!(live, vec!["10.0.0.5".to_string()]);
        assert_eq!(
            *attempts.lock().unwrap().get("10.0.0.5").unwrap(),
            2,
            "exactly one re-ping"
        );
    }

    #[test]
    #[ignore]
    fn ping_sweep_real_binary_finds_localhost() {
        // Needs raw-socket permission for the ping binary — blocked in
        // the sandbox (SOCK_RAW: Operation not permitted), so ignored
        // here. Run on the R630 with `cargo test -- --ignored`.
        let live = ping_sweep_range("127.0.0", 1, 2);
        assert!(
            live.contains(&"127.0.0.1".to_string()),
            "localhost must answer: {:?}",
            live
        );
        let silent = ping_sweep_ips(&["192.0.2.1".to_string()], ping_host);
        assert!(silent.is_empty(), "192.0.2.1 must not answer");
    }

    // ── Task 346 ────────────────────────────────────────────────────────────

    #[test]
    fn reverse_dns_resolves_localhost() {
        assert_eq!(
            resolve_hostname("127.0.0.1").as_deref(),
            Some("localhost")
        );
    }

    #[test]
    fn unresolvable_host_returns_none_without_error() {
        // TEST-NET address: no PTR, no mDNS, no NetBIOS — must be None,
        // not an error, and the caller still renders the host.
        assert_eq!(resolve_hostname("192.0.2.123"), None);
    }

    #[test]
    fn missing_resolver_binaries_degrade_to_none() {
        // avahi-resolve-address and nmblookup are absent in this
        // environment; both probes must return None, not panic.
        assert_eq!(mdns_name("192.168.1.1"), None);
        assert_eq!(netbios_name("192.168.1.1"), None);
    }

    // ── Task 347 (aggregation) ──────────────────────────────────────────────

    #[test]
    fn aggregate_merges_methods_and_dedupes_by_ip() {
        let arp = vec![
            ArpEntry {
                ip: "192.168.1.196".to_string(),
                mac: "aa:bb:cc:dd:ee:02".to_string(),
                interface: "ens18".to_string(),
                flags: "0x2".to_string(),
            },
            ArpEntry {
                ip: "192.168.1.150".to_string(),
                mac: "aa:bb:cc:dd:ee:03".to_string(),
                interface: "ens18".to_string(),
                flags: "0x2".to_string(),
            },
            // Gateway in ARP must not become a device node.
            ArpEntry {
                ip: "192.168.1.1".to_string(),
                mac: "aa:bb:cc:dd:ee:01".to_string(),
                interface: "ens18".to_string(),
                flags: "0x2".to_string(),
            },
        ];
        let ping_live = vec![
            "192.168.1.196".to_string(),
            "192.168.1.150".to_string(),
            "192.168.1.168".to_string(),
        ];
        let mut hostnames = HashMap::new();
        hostnames.insert("192.168.1.196".to_string(), Some("desktop".to_string()));
        hostnames.insert("192.168.1.150".to_string(), None);
        let helix_ips = vec!["192.168.1.168".to_string()];
        let hosts = aggregate_hosts(
            Some("192.168.1.1"),
            &arp,
            &ping_live,
            &hostnames,
            &helix_ips,
        );
        // .149 configured, .150 arp+ping, .168 ping+helix, .196 arp+ping+configured.
        assert_eq!(hosts.len(), 4, "gateway must not be a node: {:?}", hosts);
        assert_eq!(hosts[0].ip, "192.168.1.149");
        assert!(hosts[0].methods.contains(&DiscoveryMethod::Configured));
        assert_eq!(
            hosts[0].hostname.as_deref(),
            Some("Ollama server (gpt-oss:20b)")
        );
        assert_eq!(hosts[1].ip, "192.168.1.150");
        assert!(hosts[1].methods.contains(&DiscoveryMethod::Arp));
        assert!(hosts[1].methods.contains(&DiscoveryMethod::Ping));
        assert_eq!(hosts[1].hostname, None);
        assert_eq!(hosts[2].ip, "192.168.1.168");
        assert!(hosts[2].is_helix, "Helix must be marked");
        assert!(hosts[2].methods.contains(&DiscoveryMethod::Ping));
        assert_eq!(hosts[3].ip, "192.168.1.196");
        assert!(hosts[3].methods.contains(&DiscoveryMethod::Arp));
        assert!(hosts[3].methods.contains(&DiscoveryMethod::Ping));
        assert!(hosts[3].methods.contains(&DiscoveryMethod::Configured));
        assert_eq!(hosts[3].hostname.as_deref(), Some("desktop"));
        assert_eq!(hosts[3].mac.as_deref(), Some("aa:bb:cc:dd:ee:02"));
    }

    #[test]
    fn aggregate_skips_configured_hosts_on_other_subnets() {
        let hosts = aggregate_hosts(Some("10.0.0.1"), &[], &[], &HashMap::new(), &[]);
        assert!(
            hosts.is_empty(),
            "192.168.1.x configured hosts are not on 10.0.0.0/24: {:?}",
            hosts
        );
    }
}
