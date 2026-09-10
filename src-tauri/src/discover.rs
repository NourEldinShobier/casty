//! LAN discovery: viewer broadcasts "CASTY?" over UDP, hosts answer with JSON.
//! If broadcast is swallowed (a VPN on the phone), fall back to scanning the /24 over TCP.
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

pub const PORT: u16 = 45454;
const PROBE: &[u8] = b"CASTY?";
const STREAM_PORT: u16 = 45455;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Device {
    pub name: String,
    pub ip: String,
    pub port: u16,
}

/// Host side: answer probes forever. Runs on its own thread; only returns if the bind fails.
#[allow(dead_code)]
pub fn respond_forever(name: String, port: u16) -> Result<(), String> {
    let sock = UdpSocket::bind(("0.0.0.0", PORT)).map_err(|e| match e.kind() {
        std::io::ErrorKind::AddrInUse => "Casty is already running on this machine.".to_string(),
        _ => format!("Discovery port {PORT} could not be opened: {e}"),
    })?;
    let mut buf = [0u8; 64];
    loop {
        let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
        if &buf[..n] == PROBE {
            let reply = serde_json::json!({ "name": name, "port": port }).to_string();
            let _ = sock.send_to(reply.as_bytes(), from);
        }
    }
}

/// Viewer side: broadcast a probe and collect replies until `timeout`; scan the subnet if nothing answers.
pub fn probe(timeout: Duration) -> Vec<Device> {
    let mut found = Vec::new();
    let Ok(sock) = UdpSocket::bind(("0.0.0.0", 0)) else { return found };
    let _ = sock.set_broadcast(true);
    let _ = sock.set_read_timeout(Some(Duration::from_millis(200)));
    // A global broadcast leaves only the default-route interface, which a VPN tunnel usually owns.
    // Directed broadcasts reach every other network this machine is really on.
    let mut targets = vec![SocketAddr::from((Ipv4Addr::BROADCAST, PORT))];
    for n in subnets() {
        targets.push(SocketAddr::from((Ipv4Addr::new(n[0], n[1], n[2], 255), PORT)));
    }
    for t in &targets {
        let _ = sock.send_to(PROBE, t);
    }
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; 256];
    while Instant::now() < deadline {
        let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
        if let Some((name, port)) = parse_reply(&buf[..n]) {
            let d = Device { name, ip: from.ip().to_string(), port };
            if !found.contains(&d) {
                found.push(d);
            }
        }
    }
    if found.is_empty() {
        found = scan_subnets();
    }
    found
}

/// Try each network this machine is on, stopping at the first that answers.
fn scan_subnets() -> Vec<Device> {
    for net in subnets().into_iter().take(3) {
        let found = scan_one(net);
        if !found.is_empty() {
            return found;
        }
    }
    Vec::new()
}

/// ponytail: /24 only, one subnet at a time, so at most 254 threads are alive at once.
fn scan_one(net: [u8; 3]) -> Vec<Device> {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    let handles: Vec<_> = (1..=254u8)
        .map(|last| {
            std::thread::spawn(move || {
                let addr = SocketAddr::from((Ipv4Addr::new(net[0], net[1], net[2], last), STREAM_PORT));
                let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(600)).ok()?;
                let _ = s.set_read_timeout(Some(Duration::from_millis(600)));
                s.write_all(b"GET /info HTTP/1.0\r\n\r\n").ok()?;
                let mut buf = String::new();
                let _ = s.read_to_string(&mut buf);
                let (name, port) = parse_info(&buf)?;
                Some(Device { name, ip: addr.ip().to_string(), port })
            })
        })
        .collect();
    handles.into_iter().filter_map(|h| h.join().ok().flatten()).collect()
}

#[derive(Deserialize)]
struct Reply {
    name: String,
    port: u16,
}

fn parse_reply(json: &[u8]) -> Option<(String, u16)> {
    let r: Reply = serde_json::from_slice(json).ok()?;
    Some((r.name, r.port))
}

fn parse_info(http: &str) -> Option<(String, u16)> {
    let body = http.rsplit("\r\n\r\n").next()?;
    parse_reply(body.trim().as_bytes())
}

fn usable(ip: Ipv4Addr) -> bool {
    !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified() && !ip.is_broadcast()
}

/// Every /24 this machine has an address on, the default route first, deduped.
fn subnets() -> Vec<[u8; 3]> {
    let mut out: Vec<[u8; 3]> = Vec::new();
    let mut add = |ip: Ipv4Addr| {
        let o = ip.octets();
        let net = [o[0], o[1], o[2]];
        if !out.contains(&net) {
            out.push(net);
        }
    };
    if let Some(ip) = route_ipv4() {
        add(ip);
    }
    #[cfg(not(target_os = "android"))]
    {
        for (_, ip) in local_ip_address::list_afinet_netifas().unwrap_or_default() {
            if let std::net::IpAddr::V4(v4) = ip {
                if usable(v4) {
                    add(v4);
                }
            }
        }
    }
    out
}

fn route_ipv4() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    s.connect(("192.0.2.1", 9)).ok()?; // nothing is sent; this only selects the route
    match s.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(v4) if usable(v4) => Some(v4),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_info_response() {
        let http = "HTTP/1.0 200 OK\r\ncontent-type: application/json\r\n\r\n{\"name\":\"razerblade\",\"port\":45455}";
        assert_eq!(super::parse_info(http), Some(("razerblade".into(), 45455)));
        assert_eq!(super::parse_info("HTTP/1.0 404\r\n\r\nnope"), None);
    }
}
