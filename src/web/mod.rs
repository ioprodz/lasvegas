pub mod http;
pub mod websocket;

use std::net::{IpAddr, Ipv4Addr, UdpSocket};

/// Best-effort detection of the machine's outbound LAN IP.
///
/// Uses the standard "connected UDP socket" trick: connecting a UDP socket
/// to a routable address makes the kernel pick the egress interface; no
/// packets are actually sent.
pub fn lan_ip() -> IpAddr {
    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(_) => return IpAddr::V4(Ipv4Addr::LOCALHOST),
    };
    if socket.connect("8.8.8.8:80").is_err() {
        return IpAddr::V4(Ipv4Addr::LOCALHOST);
    }
    socket
        .local_addr()
        .map(|a| a.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

/// Render a bind address using the LAN IP when bound to `0.0.0.0`.
pub fn display_addr(addr: &str) -> String {
    if let Some(port) = addr.strip_prefix("0.0.0.0:") {
        format!("{}:{}", lan_ip(), port)
    } else {
        addr.to_string()
    }
}
