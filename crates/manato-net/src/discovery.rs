//! LAN lobby discovery and STUN address discovery.
//!
//! Discovery is a small UDP request/response protocol, separate from the lobby
//! protocol. Browsers broadcast a fixed request; responders reply with a
//! bounded bincode-encoded [`DiscoveryInfo`] prefixed by [`DISCOVERY_MAGIC`].

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use bincode::Options as _;
use serde::{Deserialize, Serialize};

use crate::lobby::LobbyPhase;

/// UDP request broadcast by discovery browsers.
pub const DISCOVERY_REQUEST: &[u8] = b"MANATO_DISCOVER_V1";
/// Prefix for encoded discovery responses.
pub const DISCOVERY_MAGIC: &[u8] = b"MANATO_LOBBY_V1:";
/// Default LAN discovery port.
pub const DISCOVERY_PORT: u16 = 27015;
const MAX_RESPONSES: usize = 64;

/// Advertised lobby information.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryInfo {
    pub name: String,
    /// Address clients should use for the lobby relay.
    pub address: SocketAddr,
    pub players: u16,
    pub spectators: u16,
    pub phase: LobbyPhase,
}

impl DiscoveryInfo {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = DISCOVERY_MAGIC.to_vec();
        let body = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .serialize(self)
            .unwrap_or_default();
        out.extend_from_slice(&body);
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let body = bytes.strip_prefix(DISCOVERY_MAGIC)?;
        if body.len() > 4096 {
            return None;
        }
        let decoder = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .with_limit(body.len() as u64);
        let info: Self = decoder.deserialize(body).ok()?;
        // Reject hostile names before they can reach UI consumers.
        if info.name.chars().count() > 64 {
            return None;
        }
        Some(info)
    }
}

/// Host-side responder that answers discovery broadcast requests.
pub struct DiscoveryResponder {
    socket: UdpSocket,
}

impl DiscoveryResponder {
    pub fn bind(addr: SocketAddr) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(Self { socket })
    }

    /// Convenience constructor for the default LAN discovery port.
    pub fn bind_default() -> std::io::Result<Self> {
        Self::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT)))
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// Answers pending requests with the current lobby information.
    pub fn poll(&self, info: &DiscoveryInfo) -> std::io::Result<usize> {
        let response = info.encode();
        let mut buffer = [0u8; 64];
        let mut sent = 0;
        loop {
            match self.socket.recv_from(&mut buffer) {
                Ok((len, addr)) => {
                    if &buffer[..len] == DISCOVERY_REQUEST
                        && self.socket.send_to(&response, addr).is_ok()
                    {
                        sent += 1;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
        Ok(sent)
    }
}

/// Client-side browser that queries discovery responders.
#[derive(Default)]
pub struct DiscoveryBrowser {
    socket: Option<UdpSocket>,
    lobbies: Vec<DiscoveryInfo>,
}

impl DiscoveryBrowser {
    pub fn bind(addr: SocketAddr) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket: Some(socket),
            lobbies: Vec::new(),
        })
    }

    pub fn new() -> std::io::Result<Self> {
        Self::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)))
    }

    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.socket.as_ref().and_then(|s| s.local_addr().ok())
    }

    pub fn query(&self, target: SocketAddr) -> std::io::Result<()> {
        if let Some(socket) = &self.socket {
            socket.send_to(DISCOVERY_REQUEST, target)?;
        }
        Ok(())
    }

    /// Drains replies, de-duplicating by relay address.
    pub fn poll(&mut self) -> std::io::Result<usize> {
        let Some(socket) = &self.socket else {
            return Ok(0);
        };
        let mut buffer = [0u8; 4096];
        let mut added = 0;
        loop {
            match socket.recv_from(&mut buffer) {
                Ok((len, _)) => {
                    let Some(info) = DiscoveryInfo::decode(&buffer[..len]) else {
                        continue;
                    };
                    if self.lobbies.len() >= MAX_RESPONSES
                        || self
                            .lobbies
                            .iter()
                            .any(|known| known.address == info.address)
                    {
                        continue;
                    }
                    self.lobbies.push(info);
                    added += 1;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
        Ok(added)
    }

    pub fn lobbies(&self) -> &[DiscoveryInfo] {
        &self.lobbies
    }

    pub fn clear(&mut self) {
        self.lobbies.clear();
    }
}

/// Matchmaking helper over [`DiscoveryBrowser`]. Lobbies are ranked by
/// availability (Open before Starting before InMatch; Closed excluded), then
/// by player count, spectator count, and name.
pub struct Matchmaker {
    browser: DiscoveryBrowser,
}

impl Matchmaker {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            browser: DiscoveryBrowser::new()?,
        })
    }

    /// Queries `target` and collects responses for `timeout`.
    pub fn discover(target: SocketAddr, timeout: Duration) -> std::io::Result<Vec<DiscoveryInfo>> {
        let mut browser = DiscoveryBrowser::new()?;
        browser.query(target)?;
        let started = std::time::Instant::now();
        while started.elapsed() < timeout {
            browser.poll()?;
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(browser.lobbies().to_vec())
    }

    pub fn refresh(&mut self, target: SocketAddr) -> std::io::Result<usize> {
        self.browser.query(target)?;
        self.browser.poll()
    }

    pub fn lobbies(&self) -> &[DiscoveryInfo] {
        self.browser.lobbies()
    }

    /// Best joinable lobby, or `None` when every lobby is closed.
    pub fn best(&self) -> Option<&DiscoveryInfo> {
        best_lobby(self.browser.lobbies())
    }
}

/// Ranks discovered lobbies for matchmaking.
pub fn best_lobby(lobbies: &[DiscoveryInfo]) -> Option<&DiscoveryInfo> {
    lobbies
        .iter()
        .filter(|lobby| lobby.phase != LobbyPhase::Closed)
        .max_by(|a, b| {
            let rank = |phase: LobbyPhase| match phase {
                LobbyPhase::Open => 2,
                LobbyPhase::Starting => 1,
                LobbyPhase::InMatch => 0,
                LobbyPhase::Closed => -1,
            };
            rank(a.phase)
                .cmp(&rank(b.phase))
                .then_with(|| a.players.cmp(&b.players))
                .then_with(|| a.spectators.cmp(&b.spectators))
                .then_with(|| b.name.cmp(&a.name))
        })
}

/// Builds an RFC 5389 STUN binding request.
pub fn stun_binding_request(transaction_id: [u8; 12]) -> [u8; 20] {
    let mut request = [0u8; 20];
    request[0..2].copy_from_slice(&0x0001u16.to_be_bytes());
    request[2..4].copy_from_slice(&0u16.to_be_bytes());
    request[4..8].copy_from_slice(&0x2112_A442u32.to_be_bytes());
    request[8..20].copy_from_slice(&transaction_id);
    request
}

/// Parses the XOR-MAPPED-ADDRESS (or MAPPED-ADDRESS) from a STUN response.
pub fn parse_stun_mapped(bytes: &[u8]) -> Option<SocketAddr> {
    const MAGIC: u32 = 0x2112_A442;
    if bytes.len() < 20 {
        return None;
    }
    let message_type = u16::from_be_bytes([bytes[0], bytes[1]]);
    if message_type != 0x0101 {
        return None;
    }
    if u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) != MAGIC {
        return None;
    }
    let length = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
    if 20 + length > bytes.len() {
        return None;
    }
    let mut offset = 20;
    let end = 20 + length;
    while offset + 4 <= end {
        let attr_type = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
        let attr_len = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]) as usize;
        let value = bytes.get(offset + 4..offset + 4 + attr_len)?;
        if attr_type == 0x0020 || attr_type == 0x0001 {
            if value.len() < 8 {
                return None;
            }
            let family = value[1];
            let port = u16::from_be_bytes([value[2], value[3]]);
            let port = if attr_type == 0x0020 {
                port ^ (MAGIC >> 16) as u16
            } else {
                port
            };
            let ip = match family {
                0x01 => {
                    if value.len() < 8 {
                        return None;
                    }
                    let raw = u32::from_be_bytes([value[4], value[5], value[6], value[7]]);
                    let raw = if attr_type == 0x0020 {
                        raw ^ MAGIC
                    } else {
                        raw
                    };
                    IpAddr::V4(Ipv4Addr::from(raw))
                }
                0x02 => {
                    if value.len() < 20 {
                        return None;
                    }
                    let mut raw = [0u8; 16];
                    raw.copy_from_slice(&value[4..20]);
                    if attr_type == 0x0020 {
                        let mask = [
                            0x21, 0x12, 0xA4, 0x42, bytes[8], bytes[9], bytes[10], bytes[11],
                            bytes[12], bytes[13], bytes[14], bytes[15], bytes[16], bytes[17],
                            bytes[18], bytes[19],
                        ];
                        for (byte, mask) in raw.iter_mut().zip(mask) {
                            *byte ^= mask;
                        }
                    }
                    IpAddr::V6(Ipv6Addr::from(raw))
                }
                _ => return None,
            };
            return Some(SocketAddr::new(ip, port));
        }
        offset += 4 + attr_len + (attr_len % 4);
    }
    None
}

/// Sends one STUN binding request and returns the public address.
pub fn stun_public_address(
    server: SocketAddr,
    timeout: Duration,
) -> std::io::Result<Option<SocketAddr>> {
    let socket = UdpSocket::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)))?;
    socket.set_read_timeout(Some(timeout))?;
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut transaction = [0u8; 12];
    transaction[..8].copy_from_slice(&id.to_be_bytes());
    transaction[8..].copy_from_slice(&std::process::id().to_be_bytes());
    socket.send_to(&stun_binding_request(transaction), server)?;
    let mut buffer = [0u8; 1024];
    match socket.recv_from(&mut buffer) {
        Ok((len, _)) => Ok(parse_stun_mapped(&buffer[..len])),
        Err(error)
            if error.kind() == std::io::ErrorKind::WouldBlock
                || error.kind() == std::io::ErrorKind::TimedOut =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> DiscoveryInfo {
        DiscoveryInfo {
            name: "Local Match".into(),
            address: "127.0.0.1:7000".parse().unwrap(),
            players: 2,
            spectators: 1,
            phase: LobbyPhase::Open,
        }
    }

    #[test]
    fn discovery_round_trip_and_validation() {
        let encoded = info().encode();
        assert_eq!(DiscoveryInfo::decode(&encoded), Some(info()));
        assert!(DiscoveryInfo::decode(b"not a lobby").is_none());
        let mut long = info();
        long.name = "x".repeat(65);
        assert!(DiscoveryInfo::decode(&long.encode()).is_none());
    }

    #[test]
    fn matchmaking_ranks_open_lobbies_first() {
        let mut closed = info();
        closed.name = "closed".into();
        closed.address = "127.0.0.1:7001".parse().unwrap();
        closed.phase = LobbyPhase::Closed;
        let mut match_lobby = info();
        match_lobby.name = "match".into();
        match_lobby.address = "127.0.0.1:7002".parse().unwrap();
        match_lobby.phase = LobbyPhase::InMatch;
        match_lobby.players = 4;
        let mut open = info();
        open.name = "open".into();
        open.address = "127.0.0.1:7003".parse().unwrap();
        open.players = 1;
        let lobbies = vec![closed, match_lobby, open.clone()];
        assert_eq!(best_lobby(&lobbies), Some(&open));
        assert_eq!(best_lobby(&[info()]), Some(&info()));
        assert!(best_lobby(&[]).is_none());
    }

    #[test]
    fn responder_answers_browser_queries() {
        let responder =
            DiscoveryResponder::bind("127.0.0.1:0".parse().unwrap()).expect("responder");
        let address = responder.local_addr().expect("addr");
        let mut browser = DiscoveryBrowser::new().expect("browser");
        assert_eq!(browser.lobbies().len(), 0);
        browser.query(address).expect("query");
        for _ in 0..50 {
            responder.poll(&info()).expect("poll responder");
            if browser.poll().expect("poll browser") > 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(browser.lobbies(), &[info()]);
        // Duplicate responses are ignored.
        browser.query(address).expect("query");
        for _ in 0..20 {
            responder.poll(&info()).expect("poll responder");
            browser.poll().expect("poll browser");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(browser.lobbies().len(), 1);
    }

    fn stun_response(ip: Ipv4Addr, port: u16) -> Vec<u8> {
        const MAGIC: u32 = 0x2112_A442;
        let transaction = [7u8; 12];
        let mut out = Vec::new();
        out.extend_from_slice(&0x0101u16.to_be_bytes());
        out.extend_from_slice(&12u16.to_be_bytes());
        out.extend_from_slice(&MAGIC.to_be_bytes());
        out.extend_from_slice(&transaction);
        out.extend_from_slice(&0x0020u16.to_be_bytes());
        out.extend_from_slice(&8u16.to_be_bytes());
        out.push(0);
        out.push(0x01);
        out.extend_from_slice(&(port ^ (MAGIC >> 16) as u16).to_be_bytes());
        let raw = u32::from(ip) ^ MAGIC;
        out.extend_from_slice(&raw.to_be_bytes());
        out
    }

    #[test]
    fn parses_xor_mapped_address() {
        let response = stun_response(Ipv4Addr::new(203, 0, 113, 7), 62000);
        assert_eq!(
            parse_stun_mapped(&response),
            Some("203.0.113.7:62000".parse().unwrap())
        );
        assert!(parse_stun_mapped(&response[..10]).is_none());
        let mut wrong = response.clone();
        wrong[0] = 0x02;
        assert!(parse_stun_mapped(&wrong).is_none());
    }

    #[test]
    fn stun_request_has_magic_and_length() {
        let request = stun_binding_request([9u8; 12]);
        assert_eq!(&request[0..2], &0x0001u16.to_be_bytes());
        assert_eq!(&request[2..4], &0u16.to_be_bytes());
        assert_eq!(&request[4..8], &0x2112_A442u32.to_be_bytes());
        assert_eq!(&request[8..20], &[9u8; 12]);
    }
}
