//! STUN/TURN wire protocol and a minimal TURN server.
//!
//! Implements STUN message framing, XOR addresses, long-term MESSAGE-INTEGRITY
//! (HMAC-SHA1 with MD5 long-term keys), and TURN Allocate, Refresh,
//! CreatePermission, Send indications, and ChannelData forwarding.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};

use hmac::{Hmac, Mac};
use md5::{Digest, Md5};
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

pub const MAGIC: u32 = 0x2112_A442;
pub const BINDING_REQUEST: u16 = 0x0001;
pub const BINDING_SUCCESS: u16 = 0x0101;
pub const BINDING_ERROR: u16 = 0x0111;
pub const ALLOCATE_REQUEST: u16 = 0x0003;
pub const ALLOCATE_SUCCESS: u16 = 0x0103;
pub const ALLOCATE_ERROR: u16 = 0x0113;
pub const REFRESH_REQUEST: u16 = 0x0004;
pub const REFRESH_SUCCESS: u16 = 0x0104;
pub const CREATE_PERMISSION_REQUEST: u16 = 0x0008;
pub const CREATE_PERMISSION_SUCCESS: u16 = 0x0108;
pub const CHANNEL_BIND_REQUEST: u16 = 0x0009;
pub const CHANNEL_BIND_SUCCESS: u16 = 0x0109;
pub const SEND_INDICATION: u16 = 0x0016;
pub const DATA_INDICATION: u16 = 0x0017;

pub const ATTR_MAPPED_ADDRESS: u16 = 0x0001;
pub const ATTR_USERNAME: u16 = 0x0006;
pub const ATTR_MESSAGE_INTEGRITY: u16 = 0x0008;
pub const ATTR_ERROR_CODE: u16 = 0x0009;
pub const ATTR_CHANNEL_NUMBER: u16 = 0x000C;
pub const ATTR_LIFETIME: u16 = 0x000D;
pub const ATTR_XOR_PEER_ADDRESS: u16 = 0x0012;
pub const ATTR_DATA: u16 = 0x0013;
pub const ATTR_REALM: u16 = 0x0014;
pub const ATTR_NONCE: u16 = 0x0015;
pub const ATTR_XOR_RELAYED_ADDRESS: u16 = 0x0016;
pub const ATTR_REQUESTED_TRANSPORT: u16 = 0x0019;
pub const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
pub const ATTR_SOFTWARE: u16 = 0x8022;
pub const ATTR_FINGERPRINT: u16 = 0x8028;

pub const DEFAULT_LIFETIME: u32 = 600;

#[derive(Debug, Clone)]
pub struct Message {
    pub message_type: u16,
    pub transaction: [u8; 12],
    pub attributes: Vec<(u16, Vec<u8>)>,
}

impl Message {
    pub fn new(message_type: u16, transaction: [u8; 12]) -> Self {
        Self {
            message_type,
            transaction,
            attributes: Vec::new(),
        }
    }

    pub fn with(mut self, ty: u16, value: Vec<u8>) -> Self {
        self.attributes.push((ty, value));
        self
    }

    pub fn attribute(&self, ty: u16) -> Option<&[u8]> {
        self.attributes
            .iter()
            .find(|(kind, _)| *kind == ty)
            .map(|(_, value)| value.as_slice())
    }

    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 20 {
            return None;
        }
        let message_type = u16::from_be_bytes([bytes[0], bytes[1]]);
        if message_type & 0xC000 != 0 {
            return None;
        }
        let length = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
        if u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) != MAGIC {
            return None;
        }
        if 20 + length > bytes.len() {
            return None;
        }
        let mut transaction = [0u8; 12];
        transaction.copy_from_slice(&bytes[8..20]);
        let mut attributes = Vec::new();
        let mut offset = 20;
        let end = 20 + length;
        while offset + 4 <= end {
            let ty = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
            let len = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]) as usize;
            if offset + 4 + len > end {
                return None;
            }
            attributes.push((ty, bytes[offset + 4..offset + 4 + len].to_vec()));
            offset += 4 + len + ((4 - len % 4) % 4);
        }
        Some(Self {
            message_type,
            transaction,
            attributes,
        })
    }

    /// Encodes without MESSAGE-INTEGRITY.
    pub fn encode(&self) -> Vec<u8> {
        let mut attributes = Vec::new();
        for (ty, value) in &self.attributes {
            push_attribute(&mut attributes, *ty, value);
        }
        build_message(self.message_type, &self.transaction, &attributes)
    }

    /// Encodes with a long-term MESSAGE-INTEGRITY computed using `key`.
    pub fn encode_authenticated(&self, key: &[u8]) -> Vec<u8> {
        let mut attributes = Vec::new();
        for (ty, value) in &self.attributes {
            push_attribute(&mut attributes, *ty, value);
        }
        let integrity_offset = attributes.len();
        push_attribute(&mut attributes, ATTR_MESSAGE_INTEGRITY, &[0u8; 20]);
        let mut message = build_message(self.message_type, &self.transaction, &attributes);
        let mac = hmac_sha1(key, &message);
        // Header length already includes the integrity attribute.
        let value_offset = 20 + integrity_offset + 4;
        message[value_offset..value_offset + 20].copy_from_slice(&mac);
        message
    }

    /// Verifies MESSAGE-INTEGRITY with a long-term key.
    pub fn verify_integrity(&self, key: &[u8], raw: &[u8]) -> bool {
        let Some(integrity) = self.attribute(ATTR_MESSAGE_INTEGRITY) else {
            return false;
        };
        if integrity.len() != 20 || raw.len() < 20 {
            return false;
        }
        // Locate the integrity attribute in the raw message and zero its value.
        let mut copy = raw.to_vec();
        let mut offset = 20;
        let end = 20 + u16::from_be_bytes([raw[2], raw[3]]) as usize;
        while offset + 4 <= end {
            let ty = u16::from_be_bytes([copy[offset], copy[offset + 1]]);
            let len = u16::from_be_bytes([copy[offset + 2], copy[offset + 3]]) as usize;
            if ty == ATTR_MESSAGE_INTEGRITY && len == 20 {
                copy[offset + 4..offset + 24].fill(0);
                let mac = hmac_sha1(key, &copy[..end.min(copy.len())]);
                return mac == integrity;
            }
            offset += 4 + len + ((4 - len % 4) % 4);
        }
        false
    }
}

fn push_attribute(out: &mut Vec<u8>, ty: u16, value: &[u8]) {
    out.extend_from_slice(&ty.to_be_bytes());
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value);
    let padding = (4 - value.len() % 4) % 4;
    out.extend(std::iter::repeat_n(0u8, padding));
}

fn build_message(message_type: u16, transaction: &[u8; 12], attributes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(20 + attributes.len());
    out.extend_from_slice(&message_type.to_be_bytes());
    out.extend_from_slice(&(attributes.len() as u16).to_be_bytes());
    out.extend_from_slice(&MAGIC.to_be_bytes());
    out.extend_from_slice(transaction);
    out.extend_from_slice(attributes);
    out
}

fn hmac_sha1(key: &[u8], message: &[u8]) -> [u8; 20] {
    let mut mac = HmacSha1::new_from_slice(key).expect("hmac accepts any key");
    mac.update(message);
    mac.finalize().into_bytes().into()
}

/// TURN long-term key: MD5(`username:realm:password`).
pub fn long_term_key(username: &str, realm: &str, password: &str) -> [u8; 16] {
    let mut hasher = Md5::new();
    hasher.update(format!("{username}:{realm}:{password}").as_bytes());
    hasher.finalize().into()
}

pub fn encode_xor_address(addr: SocketAddr, transaction: &[u8; 12]) -> Vec<u8> {
    let mut value = Vec::new();
    value.push(0);
    match addr.ip() {
        IpAddr::V4(ip) => {
            value.push(0x01);
            value.extend_from_slice(&(addr.port() ^ (MAGIC >> 16) as u16).to_be_bytes());
            let raw = u32::from(ip) ^ MAGIC;
            value.extend_from_slice(&raw.to_be_bytes());
        }
        IpAddr::V6(ip) => {
            value.push(0x02);
            value.extend_from_slice(&(addr.port() ^ (MAGIC >> 16) as u16).to_be_bytes());
            let mut mask = [0u8; 16];
            mask[..4].copy_from_slice(&MAGIC.to_be_bytes());
            mask[4..].copy_from_slice(transaction);
            let raw = ip.octets();
            for index in 0..16 {
                value.push(raw[index] ^ mask[index]);
            }
        }
    }
    value
}

pub fn decode_xor_address(value: &[u8], transaction: &[u8; 12]) -> Option<SocketAddr> {
    if value.len() < 8 {
        return None;
    }
    let port = u16::from_be_bytes([value[2], value[3]]) ^ (MAGIC >> 16) as u16;
    match value[1] {
        0x01 => {
            let raw = u32::from_be_bytes([value[4], value[5], value[6], value[7]]) ^ MAGIC;
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(raw)), port))
        }
        0x02 => {
            if value.len() < 20 {
                return None;
            }
            let mut mask = [0u8; 16];
            mask[..4].copy_from_slice(&MAGIC.to_be_bytes());
            mask[4..].copy_from_slice(transaction);
            let mut raw = [0u8; 16];
            for index in 0..16 {
                raw[index] = value[4 + index] ^ mask[index];
            }
            Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(raw)), port))
        }
        _ => None,
    }
}

pub fn error_code(code: u16, reason: &str) -> Vec<u8> {
    let mut value = vec![0, 0];
    value.extend_from_slice(&code.to_be_bytes());
    value.extend_from_slice(reason.as_bytes());
    value
}

fn authenticate_request(
    message: &Message,
    raw: &[u8],
    realm: &str,
    nonce: &str,
    users: &HashMap<String, String>,
) -> Option<String> {
    let username = String::from_utf8(message.attribute(ATTR_USERNAME)?.to_vec()).ok()?;
    let request_realm = String::from_utf8(message.attribute(ATTR_REALM)?.to_vec()).ok()?;
    let request_nonce = String::from_utf8(message.attribute(ATTR_NONCE)?.to_vec()).ok()?;
    if request_realm != realm || request_nonce != nonce {
        return None;
    }
    let password = users.get(&username)?;
    let key = long_term_key(&username, realm, password);
    message.verify_integrity(&key, raw).then_some(username)
}

fn encode_authenticated_reply(
    message: Message,
    realm: &str,
    username: &str,
    password: &str,
) -> Vec<u8> {
    let key = long_term_key(username, realm, password);
    message.encode_authenticated(&key)
}

/// Minimal TURN allocation state.
struct Allocation {
    relay: UdpSocket,
    permissions: HashSet<SocketAddr>,
    channel: Option<(u16, SocketAddr)>,
    expires: std::time::Instant,
}

/// TURN server with long-term authentication.
pub struct TurnServer {
    socket: UdpSocket,
    realm: String,
    nonce: String,
    users: HashMap<String, String>,
    allocations: HashMap<SocketAddr, Allocation>,
}

impl TurnServer {
    pub fn bind(
        addr: SocketAddr,
        realm: impl Into<String>,
        users: impl IntoIterator<Item = (String, String)>,
    ) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            realm: realm.into(),
            nonce: "manato-nonce".into(),
            users: users.into_iter().collect(),
            allocations: HashMap::new(),
        })
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    fn authenticate(&self, message: &Message, raw: &[u8]) -> Option<String> {
        authenticate_request(message, raw, &self.realm, &self.nonce, &self.users)
    }

    fn reply(&self, target: SocketAddr, message: Message, username: &str) {
        if let Some(password) = self.users.get(username) {
            let key = long_term_key(username, &self.realm, password);
            let _ = self
                .socket
                .send_to(&message.encode_authenticated(&key), target);
        }
    }

    /// Drains client, relay, and ChannelData traffic.
    pub fn poll(&mut self) -> std::io::Result<usize> {
        let mut buffer = vec![0u8; 65536];
        let mut handled = 0;
        loop {
            match self.socket.recv_from(&mut buffer) {
                Ok((len, source)) => {
                    let data = &buffer[..len];
                    // ChannelData: first byte 0x40-0x7f.
                    if len >= 4 && (data[0] & 0xC0) == 0x40 {
                        let channel = u16::from_be_bytes([data[0], data[1]]);
                        let size = u16::from_be_bytes([data[2], data[3]]) as usize;
                        if let Some(allocation) = self.allocations.get(&source) {
                            if let Some((bound, peer)) = allocation.channel {
                                if bound == channel && 4 + size <= len {
                                    let _ = allocation.relay.send_to(&data[4..4 + size], peer);
                                    handled += 1;
                                }
                            }
                        }
                        continue;
                    }
                    let Some(message) = Message::parse(data) else {
                        continue;
                    };
                    match message.message_type {
                        BINDING_REQUEST => {
                            let mut reply = Message::new(BINDING_SUCCESS, message.transaction)
                                .with(
                                    ATTR_XOR_MAPPED_ADDRESS,
                                    encode_xor_address(source, &message.transaction),
                                );
                            reply
                                .attributes
                                .push((ATTR_SOFTWARE, b"manato-turn".to_vec()));
                            let _ = self.socket.send_to(&reply.encode(), source);
                            handled += 1;
                        }
                        ALLOCATE_REQUEST => {
                            let Some(username) = self.authenticate(&message, data) else {
                                let reply = Message::new(ALLOCATE_ERROR, message.transaction)
                                    .with(ATTR_ERROR_CODE, error_code(401, "Unauthorized"))
                                    .with(ATTR_REALM, self.realm.as_bytes().to_vec())
                                    .with(ATTR_NONCE, self.nonce.as_bytes().to_vec());
                                let _ = self.socket.send_to(&reply.encode(), source);
                                continue;
                            };
                            let relay = UdpSocket::bind("0.0.0.0:0")?;
                            relay.set_nonblocking(true)?;
                            let relay_addr = relay.local_addr()?;
                            self.allocations.insert(
                                source,
                                Allocation {
                                    relay,
                                    permissions: HashSet::new(),
                                    channel: None,
                                    expires: std::time::Instant::now()
                                        + std::time::Duration::from_secs(600),
                                },
                            );
                            let reply = Message::new(ALLOCATE_SUCCESS, message.transaction)
                                .with(
                                    ATTR_XOR_RELAYED_ADDRESS,
                                    encode_xor_address(relay_addr, &message.transaction),
                                )
                                .with(ATTR_LIFETIME, DEFAULT_LIFETIME.to_be_bytes().to_vec())
                                .with(ATTR_SOFTWARE, b"manato-turn".to_vec());
                            self.reply(source, reply, &username);
                            handled += 1;
                        }
                        REFRESH_REQUEST => {
                            let Some(username) = self.authenticate(&message, data) else {
                                continue;
                            };
                            if let Some(allocation) = self.allocations.get_mut(&source) {
                                allocation.expires =
                                    std::time::Instant::now() + std::time::Duration::from_secs(600);
                                let reply = Message::new(REFRESH_SUCCESS, message.transaction)
                                    .with(ATTR_LIFETIME, DEFAULT_LIFETIME.to_be_bytes().to_vec());
                                self.reply(source, reply, &username);
                                handled += 1;
                            }
                        }
                        CREATE_PERMISSION_REQUEST => {
                            let Some(username) = self.authenticate(&message, data) else {
                                continue;
                            };
                            if let Some(allocation) = self.allocations.get_mut(&source) {
                                for (ty, value) in &message.attributes {
                                    if *ty == ATTR_XOR_PEER_ADDRESS {
                                        if let Some(peer) =
                                            decode_xor_address(value, &message.transaction)
                                        {
                                            allocation.permissions.insert(peer);
                                        }
                                    }
                                }
                                let reply =
                                    Message::new(CREATE_PERMISSION_SUCCESS, message.transaction);
                                self.reply(source, reply, &username);
                                handled += 1;
                            }
                        }
                        CHANNEL_BIND_REQUEST => {
                            let Some(username) = self.authenticate(&message, data) else {
                                continue;
                            };
                            if let Some(allocation) = self.allocations.get_mut(&source) {
                                let channel =
                                    message.attribute(ATTR_CHANNEL_NUMBER).and_then(|value| {
                                        value
                                            .get(..2)
                                            .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
                                    });
                                let peer =
                                    message.attribute(ATTR_XOR_PEER_ADDRESS).and_then(|value| {
                                        decode_xor_address(value, &message.transaction)
                                    });
                                if let (Some(channel), Some(peer)) = (channel, peer) {
                                    allocation.channel = Some((channel, peer));
                                    allocation.permissions.insert(peer);
                                    let reply =
                                        Message::new(CHANNEL_BIND_SUCCESS, message.transaction);
                                    self.reply(source, reply, &username);
                                    handled += 1;
                                }
                            }
                        }
                        SEND_INDICATION => {
                            // Indications are not authenticated in TURN; forward if permitted.
                            let Some(allocation) = self.allocations.get(&source) else {
                                continue;
                            };
                            let peer = message
                                .attribute(ATTR_XOR_PEER_ADDRESS)
                                .and_then(|value| decode_xor_address(value, &message.transaction));
                            let payload = message.attribute(ATTR_DATA);
                            if let (Some(peer), Some(payload)) = (peer, payload) {
                                if allocation.permissions.contains(&peer) {
                                    let _ = allocation.relay.send_to(payload, peer);
                                    handled += 1;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
        // Relay → client: wrap inbound peer data as ChannelData or DataIndication.
        let clients: Vec<SocketAddr> = self.allocations.keys().copied().collect();
        for client in clients {
            let Some(allocation) = self.allocations.get(&client) else {
                continue;
            };
            loop {
                match allocation.relay.recv_from(&mut buffer) {
                    Ok((len, peer)) => {
                        if allocation.channel.is_some_and(|(_, bound)| bound == peer) {
                            let (channel, _) = allocation.channel.expect("checked");
                            let mut frame = Vec::with_capacity(len + 4);
                            frame.extend_from_slice(&channel.to_be_bytes());
                            frame.extend_from_slice(&(len as u16).to_be_bytes());
                            frame.extend_from_slice(&buffer[..len]);
                            let _ = self.socket.send_to(&frame, client);
                        } else if allocation.permissions.contains(&peer) {
                            let reply = Message::new(DATA_INDICATION, [0u8; 12])
                                .with(ATTR_XOR_PEER_ADDRESS, encode_xor_address(peer, &[0u8; 12]))
                                .with(ATTR_DATA, buffer[..len].to_vec());
                            let _ = self.socket.send_to(&reply.encode(), client);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) => return Err(error),
                }
            }
        }
        self.allocations
            .retain(|_, allocation| allocation.expires > std::time::Instant::now());
        Ok(handled)
    }
}

struct TcpAllocation {
    relay: UdpSocket,
    permissions: HashSet<SocketAddr>,
    channel: Option<(u16, SocketAddr)>,
    expires: std::time::Instant,
}

struct TcpConnection {
    stream: std::net::TcpStream,
    buffer: Vec<u8>,
    allocation: Option<TcpAllocation>,
    closed: bool,
}

/// TURN server over TCP with the same STUN framing and ChannelData.
pub struct TurnTcpServer {
    listener: std::net::TcpListener,
    realm: String,
    nonce: String,
    users: HashMap<String, String>,
    connections: Vec<TcpConnection>,
}

impl TurnTcpServer {
    pub fn bind(
        addr: SocketAddr,
        realm: impl Into<String>,
        users: impl IntoIterator<Item = (String, String)>,
    ) -> std::io::Result<Self> {
        let listener = std::net::TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            realm: realm.into(),
            nonce: "manato-nonce".into(),
            users: users.into_iter().collect(),
            connections: Vec::new(),
        })
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    fn handle(
        realm: &str,
        nonce: &str,
        users: &HashMap<String, String>,
        message: &Message,
        raw: &[u8],
        source: SocketAddr,
        allocation: &mut Option<TcpAllocation>,
    ) -> Vec<Vec<u8>> {
        match message.message_type {
            BINDING_REQUEST => {
                let reply = Message::new(BINDING_SUCCESS, message.transaction)
                    .with(
                        ATTR_XOR_MAPPED_ADDRESS,
                        encode_xor_address(source, &message.transaction),
                    )
                    .with(ATTR_SOFTWARE, b"manato-turn".to_vec());
                vec![reply.encode()]
            }
            ALLOCATE_REQUEST => {
                let Some(username) = authenticate_request(message, raw, realm, nonce, users) else {
                    let reply = Message::new(ALLOCATE_ERROR, message.transaction)
                        .with(ATTR_ERROR_CODE, error_code(401, "Unauthorized"))
                        .with(ATTR_REALM, realm.as_bytes().to_vec())
                        .with(ATTR_NONCE, nonce.as_bytes().to_vec());
                    return vec![reply.encode()];
                };
                let Ok(relay) = UdpSocket::bind("0.0.0.0:0") else {
                    return Vec::new();
                };
                let _ = relay.set_nonblocking(true);
                let Ok(relay_addr) = relay.local_addr() else {
                    return Vec::new();
                };
                *allocation = Some(TcpAllocation {
                    relay,
                    permissions: HashSet::new(),
                    channel: None,
                    expires: std::time::Instant::now() + std::time::Duration::from_secs(600),
                });
                let reply = Message::new(ALLOCATE_SUCCESS, message.transaction)
                    .with(
                        ATTR_XOR_RELAYED_ADDRESS,
                        encode_xor_address(relay_addr, &message.transaction),
                    )
                    .with(ATTR_LIFETIME, DEFAULT_LIFETIME.to_be_bytes().to_vec())
                    .with(ATTR_SOFTWARE, b"manato-turn".to_vec());
                let password = users.get(&username).expect("authenticated");
                vec![encode_authenticated_reply(
                    reply, realm, &username, password,
                )]
            }
            REFRESH_REQUEST => {
                let Some(username) = authenticate_request(message, raw, realm, nonce, users) else {
                    return Vec::new();
                };
                if let Some(allocation) = allocation {
                    allocation.expires =
                        std::time::Instant::now() + std::time::Duration::from_secs(600);
                    let reply = Message::new(REFRESH_SUCCESS, message.transaction)
                        .with(ATTR_LIFETIME, DEFAULT_LIFETIME.to_be_bytes().to_vec());
                    let password = users.get(&username).expect("authenticated");
                    vec![encode_authenticated_reply(
                        reply, realm, &username, password,
                    )]
                } else {
                    Vec::new()
                }
            }
            CREATE_PERMISSION_REQUEST => {
                let Some(username) = authenticate_request(message, raw, realm, nonce, users) else {
                    return Vec::new();
                };
                let Some(allocation) = allocation else {
                    return Vec::new();
                };
                for (ty, value) in &message.attributes {
                    if *ty == ATTR_XOR_PEER_ADDRESS {
                        if let Some(peer) = decode_xor_address(value, &message.transaction) {
                            allocation.permissions.insert(peer);
                        }
                    }
                }
                let reply = Message::new(CREATE_PERMISSION_SUCCESS, message.transaction);
                let password = users.get(&username).expect("authenticated");
                vec![encode_authenticated_reply(
                    reply, realm, &username, password,
                )]
            }
            CHANNEL_BIND_REQUEST => {
                let Some(username) = authenticate_request(message, raw, realm, nonce, users) else {
                    return Vec::new();
                };
                let Some(allocation) = allocation else {
                    return Vec::new();
                };
                let channel = message.attribute(ATTR_CHANNEL_NUMBER).and_then(|value| {
                    value
                        .get(..2)
                        .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
                });
                let peer = message
                    .attribute(ATTR_XOR_PEER_ADDRESS)
                    .and_then(|value| decode_xor_address(value, &message.transaction));
                let (Some(channel), Some(peer)) = (channel, peer) else {
                    return Vec::new();
                };
                allocation.channel = Some((channel, peer));
                allocation.permissions.insert(peer);
                let reply = Message::new(CHANNEL_BIND_SUCCESS, message.transaction);
                let password = users.get(&username).expect("authenticated");
                vec![encode_authenticated_reply(
                    reply, realm, &username, password,
                )]
            }
            SEND_INDICATION => {
                if let Some(allocation) = allocation {
                    let peer = message
                        .attribute(ATTR_XOR_PEER_ADDRESS)
                        .and_then(|value| decode_xor_address(value, &message.transaction));
                    let payload = message.attribute(ATTR_DATA);
                    if let (Some(peer), Some(payload)) = (peer, payload) {
                        if allocation.permissions.contains(&peer) {
                            let _ = allocation.relay.send_to(payload, peer);
                        }
                    }
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    pub fn poll(&mut self) -> std::io::Result<usize> {
        let mut handled = 0;
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(true)?;
                    self.connections.push(TcpConnection {
                        stream,
                        buffer: Vec::new(),
                        allocation: None,
                        closed: false,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
        let mut buffer = [0u8; 8192];
        for connection in &mut self.connections {
            loop {
                match connection.stream.read(&mut buffer) {
                    Ok(0) => {
                        connection.closed = true;
                        break;
                    }
                    Ok(len) => connection.buffer.extend_from_slice(&buffer[..len]),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        connection.closed = true;
                        break;
                    }
                }
            }
            while let Some(frame) = take_tcp_frame(&mut connection.buffer) {
                if frame.len() >= 4 && (frame[0] & 0xC0) == 0x40 {
                    if let Some(allocation) = &connection.allocation {
                        let channel = u16::from_be_bytes([frame[0], frame[1]]);
                        let len = u16::from_be_bytes([frame[2], frame[3]]) as usize;
                        if let Some((bound, peer)) = allocation.channel {
                            if bound == channel && 4 + len <= frame.len() {
                                let _ = allocation.relay.send_to(&frame[4..4 + len], peer);
                                handled += 1;
                            }
                        }
                    }
                    continue;
                }
                let Some(message) = Message::parse(&frame) else {
                    continue;
                };
                let Ok(source) = connection.stream.peer_addr() else {
                    continue;
                };
                let replies = Self::handle(
                    &self.realm,
                    &self.nonce,
                    &self.users,
                    &message,
                    &frame,
                    source,
                    &mut connection.allocation,
                );
                for reply in replies {
                    if connection.stream.write_all(&reply).is_err() {
                        connection.closed = true;
                        break;
                    }
                    handled += 1;
                }
            }
            if let Some(allocation) = &connection.allocation {
                loop {
                    match allocation.relay.recv_from(&mut buffer) {
                        Ok((len, peer)) => {
                            let mut frame = Vec::new();
                            if allocation.channel.is_some_and(|(_, bound)| bound == peer) {
                                let channel = allocation.channel.expect("checked").0;
                                frame.extend_from_slice(&channel.to_be_bytes());
                                frame.extend_from_slice(&(len as u16).to_be_bytes());
                            } else if allocation.permissions.contains(&peer) {
                                let reply = Message::new(DATA_INDICATION, [0u8; 12])
                                    .with(
                                        ATTR_XOR_PEER_ADDRESS,
                                        encode_xor_address(peer, &[0u8; 12]),
                                    )
                                    .with(ATTR_DATA, buffer[..len].to_vec());
                                frame = reply.encode();
                            } else {
                                continue;
                            }
                            frame.extend_from_slice(&buffer[..len]);
                            if connection.stream.write_all(&frame).is_err() {
                                connection.closed = true;
                                break;
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(_) => break,
                    }
                }
            }
        }
        let now = std::time::Instant::now();
        for connection in &mut self.connections {
            if let Some(allocation) = &connection.allocation {
                if allocation.expires <= now {
                    connection.allocation = None;
                }
            }
        }
        self.connections.retain(|connection| !connection.closed);
        Ok(handled)
    }
}

fn take_tcp_frame(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    if buffer.is_empty() {
        return None;
    }
    let needed = if buffer[0] & 0xC0 == 0x40 {
        if buffer.len() < 4 {
            return None;
        }
        4 + u16::from_be_bytes([buffer[2], buffer[3]]) as usize
    } else {
        if buffer.len() < 20 {
            return None;
        }
        20 + u16::from_be_bytes([buffer[2], buffer[3]]) as usize
    };
    if buffer.len() < needed {
        return None;
    }
    Some(buffer.drain(..needed).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    const TX: [u8; 12] = [1; 12];

    #[test]
    fn message_round_trip_attributes_and_integrity() {
        let key = long_term_key("alice", "manato", "secret");
        let message = Message::new(BINDING_REQUEST, TX)
            .with(ATTR_USERNAME, b"alice".to_vec())
            .with(ATTR_REALM, b"manato".to_vec())
            .with(ATTR_NONCE, b"nonce".to_vec());
        let encoded = message.encode_authenticated(&key);
        let parsed = Message::parse(&encoded).expect("parse");
        assert_eq!(parsed.message_type, BINDING_REQUEST);
        assert_eq!(parsed.transaction, TX);
        assert_eq!(parsed.attribute(ATTR_USERNAME), Some(&b"alice"[..]));
        assert!(parsed.verify_integrity(&key, &encoded));
        assert!(!parsed.verify_integrity(&long_term_key("alice", "manato", "bad"), &encoded));
        let mut tampered = encoded.clone();
        tampered[20] ^= 0xff;
        assert!(!parsed.verify_integrity(&key, &tampered));
    }

    #[test]
    fn xor_addresses_round_trip_v4_and_v6() {
        for address in [
            "203.0.113.7:62000".parse().unwrap(),
            "[2001:db8::1]:3478".parse().unwrap(),
        ] {
            let value = encode_xor_address(address, &TX);
            assert_eq!(decode_xor_address(&value, &TX), Some(address));
        }
        assert!(decode_xor_address(&[0, 9, 0, 0], &TX).is_none());
    }

    #[test]
    fn turn_allocate_permission_and_channel_forwarding() {
        let mut server = TurnServer::bind(
            "127.0.0.1:0".parse().unwrap(),
            "manato",
            [("alice".to_string(), "secret".to_string())],
        )
        .expect("server");
        let server_addr = server.local_addr().expect("addr");
        let client = UdpSocket::bind("127.0.0.1:0").expect("client");
        client.set_nonblocking(true).expect("nonblocking");
        let peer = UdpSocket::bind("127.0.0.1:0").expect("peer");
        peer.set_nonblocking(true).expect("nonblocking");
        let peer_addr = peer.local_addr().expect("peer addr");
        let key = long_term_key("alice", "manato", "secret");
        let auth = |message: Message| {
            message
                .with(ATTR_USERNAME, b"alice".to_vec())
                .with(ATTR_REALM, b"manato".to_vec())
                .with(ATTR_NONCE, b"manato-nonce".to_vec())
                .encode_authenticated(&key)
        };
        let allocate = auth(
            Message::new(ALLOCATE_REQUEST, TX).with(ATTR_REQUESTED_TRANSPORT, vec![17, 0, 0, 0]),
        );
        client.send_to(&allocate, server_addr).expect("allocate");
        let mut relayed = None;
        let started = Instant::now();
        let mut buffer = [0u8; 4096];
        while started.elapsed() < Duration::from_secs(1) {
            server.poll().expect("poll");
            if let Ok((len, _)) = client.recv_from(&mut buffer) {
                let message = Message::parse(&buffer[..len]).expect("reply");
                if message.message_type == ALLOCATE_SUCCESS {
                    relayed = message
                        .attribute(ATTR_XOR_RELAYED_ADDRESS)
                        .and_then(|value| decode_xor_address(value, &TX));
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let relayed = relayed.expect("relayed address");
        let permission = auth(Message::new(CREATE_PERMISSION_REQUEST, [2; 12]).with(
            ATTR_XOR_PEER_ADDRESS,
            encode_xor_address(peer_addr, &[2; 12]),
        ));
        client
            .send_to(&permission, server_addr)
            .expect("permission");
        let channel = 0x4001u16;
        let bind = auth(
            Message::new(CHANNEL_BIND_REQUEST, [3; 12])
                .with(ATTR_CHANNEL_NUMBER, channel.to_be_bytes().to_vec())
                .with(
                    ATTR_XOR_PEER_ADDRESS,
                    encode_xor_address(peer_addr, &[3; 12]),
                ),
        );
        client.send_to(&bind, server_addr).expect("bind");
        for _ in 0..50 {
            server.poll().expect("poll");
            let _ = client.recv_from(&mut buffer);
            std::thread::sleep(Duration::from_millis(2));
        }
        // Client ChannelData → peer.
        let mut frame = Vec::new();
        frame.extend_from_slice(&channel.to_be_bytes());
        frame.extend_from_slice(&4u16.to_be_bytes());
        frame.extend_from_slice(b"ping");
        client.send_to(&frame, server_addr).expect("channel data");
        let started = Instant::now();
        let mut got = None;
        while started.elapsed() < Duration::from_secs(1) {
            server.poll().expect("poll");
            if let Ok((len, _)) = peer.recv_from(&mut buffer) {
                got = Some(buffer[..len].to_vec());
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(got.as_deref(), Some(&b"ping"[..]));
        // Peer → relay → client ChannelData.
        peer.send_to(b"pong", relayed).expect("peer send");
        let started = Instant::now();
        let mut got = None;
        while started.elapsed() < Duration::from_secs(1) {
            server.poll().expect("poll");
            if let Ok((len, _)) = client.recv_from(&mut buffer) {
                if len >= 4 && buffer[0] & 0xC0 == 0x40 {
                    got = Some(buffer[4..len].to_vec());
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(got.as_deref(), Some(&b"pong"[..]));
    }

    fn read_tcp_frame(stream: &mut std::net::TcpStream) -> Vec<u8> {
        let mut header = [0u8; 4];
        stream.read_exact(&mut header).expect("header");
        let length = if header[0] & 0xC0 == 0x40 {
            u16::from_be_bytes([header[2], header[3]]) as usize
        } else {
            let mut rest = [0u8; 16];
            stream.read_exact(&mut rest).expect("stun header");
            let mut frame = vec![0u8; 20];
            frame[..4].copy_from_slice(&header);
            frame[4..].copy_from_slice(&rest);
            let length = u16::from_be_bytes([header[2], header[3]]) as usize;
            let mut payload = vec![0u8; length];
            stream.read_exact(&mut payload).expect("stun payload");
            frame.extend_from_slice(&payload);
            return frame;
        };
        let mut payload = vec![0u8; length];
        stream.read_exact(&mut payload).expect("payload");
        let mut frame = header.to_vec();
        frame.extend_from_slice(&payload);
        frame
    }

    #[test]
    fn tcp_turn_allocates_and_forwards_channel_data() {
        let mut server = TurnTcpServer::bind(
            "127.0.0.1:0".parse().unwrap(),
            "manato",
            [("alice".to_string(), "secret".to_string())],
        )
        .expect("server");
        let server_addr = server.local_addr().expect("addr");
        let mut client = std::net::TcpStream::connect(server_addr).expect("client");
        client
            .set_read_timeout(Some(Duration::from_millis(500)))
            .expect("timeout");
        let peer = UdpSocket::bind("127.0.0.1:0").expect("peer");
        peer.set_nonblocking(true).expect("nonblocking");
        let peer_addr = peer.local_addr().expect("peer addr");
        let key = long_term_key("alice", "manato", "secret");
        let auth = |message: Message| {
            message
                .with(ATTR_USERNAME, b"alice".to_vec())
                .with(ATTR_REALM, b"manato".to_vec())
                .with(ATTR_NONCE, b"manato-nonce".to_vec())
                .encode_authenticated(&key)
        };
        client
            .write_all(&auth(Message::new(ALLOCATE_REQUEST, TX)))
            .expect("allocate");
        let mut relayed = None;
        for _ in 0..100 {
            server.poll().expect("poll");
            if let Ok(frame) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                read_tcp_frame(&mut client)
            })) {
                let message = Message::parse(&frame).expect("reply");
                if message.message_type == ALLOCATE_SUCCESS {
                    relayed = message
                        .attribute(ATTR_XOR_RELAYED_ADDRESS)
                        .and_then(|value| decode_xor_address(value, &TX));
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let relayed = relayed.expect("relayed");
        client
            .write_all(&auth(
                Message::new(CREATE_PERMISSION_REQUEST, [2; 12]).with(
                    ATTR_XOR_PEER_ADDRESS,
                    encode_xor_address(peer_addr, &[2; 12]),
                ),
            ))
            .expect("permission");
        let channel = 0x4002u16;
        client
            .write_all(&auth(
                Message::new(CHANNEL_BIND_REQUEST, [3; 12])
                    .with(ATTR_CHANNEL_NUMBER, channel.to_be_bytes().to_vec())
                    .with(
                        ATTR_XOR_PEER_ADDRESS,
                        encode_xor_address(peer_addr, &[3; 12]),
                    ),
            ))
            .expect("bind");
        for _ in 0..20 {
            server.poll().expect("poll");
            std::thread::sleep(Duration::from_millis(2));
        }
        let mut frame = Vec::new();
        frame.extend_from_slice(&channel.to_be_bytes());
        frame.extend_from_slice(&3u16.to_be_bytes());
        frame.extend_from_slice(b"tcp");
        client.write_all(&frame).expect("channel data");
        let started = Instant::now();
        let mut buffer = [0u8; 64];
        let mut got = None;
        while started.elapsed() < Duration::from_secs(1) {
            server.poll().expect("poll");
            if let Ok((len, _)) = peer.recv_from(&mut buffer) {
                got = Some(buffer[..len].to_vec());
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(got.as_deref(), Some(&b"tcp"[..]));

        peer.send_to(b"back", relayed).expect("peer");
        let started = Instant::now();
        let mut got = None;
        while started.elapsed() < Duration::from_secs(1) {
            server.poll().expect("poll");
            if let Ok(frame) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                read_tcp_frame(&mut client)
            })) {
                if frame.len() >= 4 && frame[0] & 0xC0 == 0x40 {
                    got = Some(frame[4..].to_vec());
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(got.as_deref(), Some(&b"back"[..]));
    }

    #[test]
    fn turn_rejects_bad_credentials() {
        let mut server = TurnServer::bind(
            "127.0.0.1:0".parse().unwrap(),
            "manato",
            [("alice".to_string(), "secret".to_string())],
        )
        .expect("server");
        let server_addr = server.local_addr().expect("addr");
        let client = UdpSocket::bind("127.0.0.1:0").expect("client");
        client.set_nonblocking(true).expect("nonblocking");
        let wrong = long_term_key("alice", "manato", "wrong");
        let request = Message::new(ALLOCATE_REQUEST, TX)
            .with(ATTR_USERNAME, b"alice".to_vec())
            .with(ATTR_REALM, b"manato".to_vec())
            .with(ATTR_NONCE, b"manato-nonce".to_vec())
            .encode_authenticated(&wrong);
        client.send_to(&request, server_addr).expect("send");
        let started = Instant::now();
        let mut buffer = [0u8; 4096];
        let mut reply = None;
        while started.elapsed() < Duration::from_secs(1) {
            server.poll().expect("poll");
            if let Ok((len, _)) = client.recv_from(&mut buffer) {
                reply = Message::parse(&buffer[..len]);
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let reply = reply.expect("error reply");
        assert_eq!(reply.message_type, ALLOCATE_ERROR);
        let error = reply.attribute(ATTR_ERROR_CODE).expect("error");
        assert_eq!(u16::from_be_bytes([error[2], error[3]]), 401);
    }
}
