//! LAN discovery: viewer broadcasts "CASTY?" over UDP, hosts answer with JSON.
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

pub const PORT: u16 = 45454;
const PROBE: &[u8] = b"CASTY?";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Device {
    pub name: String,
    pub ip: String,
    pub port: u16,
}

/// Host side: answer probes forever. Runs on its own thread.
#[allow(dead_code)]
pub fn respond_forever(name: String, port: u16) {
    let sock = match UdpSocket::bind(("0.0.0.0", PORT)) {
        Ok(s) => s,
        Err(e) => return eprintln!("casty: discovery bind failed: {e}"),
    };
    let mut buf = [0u8; 64];
    loop {
        let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
        if &buf[..n] == PROBE {
            let reply = serde_json::json!({ "name": name, "port": port }).to_string();
            let _ = sock.send_to(reply.as_bytes(), from);
        }
    }
}

/// Viewer side: broadcast a probe and collect replies until `timeout`.
pub fn probe(timeout: Duration) -> Vec<Device> {
    let mut found = Vec::new();
    let Ok(sock) = UdpSocket::bind(("0.0.0.0", 0)) else { return found };
    let _ = sock.set_broadcast(true);
    let _ = sock.set_read_timeout(Some(Duration::from_millis(200)));
    let mut targets = vec![SocketAddr::from((Ipv4Addr::BROADCAST, PORT))];
    // ponytail: assume /24 for the directed broadcast; the global broadcast covers the rest
    if let Some(ip) = local_ipv4() {
        let o = ip.octets();
        targets.push(SocketAddr::from((Ipv4Addr::new(o[0], o[1], o[2], 255), PORT)));
    }
    for t in &targets {
        let _ = sock.send_to(PROBE, t);
    }
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; 256];
    while Instant::now() < deadline {
        let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
        #[derive(Deserialize)]
        struct Reply {
            name: String,
            port: u16,
        }
        if let Ok(r) = serde_json::from_slice::<Reply>(&buf[..n]) {
            let d = Device { name: r.name, ip: from.ip().to_string(), port: r.port };
            if !found.contains(&d) {
                found.push(d);
            }
        }
    }
    found
}

fn local_ipv4() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    s.connect(("192.0.2.1", 9)).ok()?; // nothing is sent; this only selects the route
    match s.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(v4) if !v4.is_loopback() => Some(v4),
        _ => None,
    }
}
