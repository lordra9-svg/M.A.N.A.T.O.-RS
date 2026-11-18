//! Two-peer UDP forwarding relay used when NAT hole punching fails.

use std::net::{SocketAddr, UdpSocket};

/// Forwards datagrams between the first two peers that send to it.
///
/// Both peers configure the relay address as their netplay remote. The relay
/// learns each peer's source address from its first datagram and forwards every
/// later datagram to the other peer. This is a plaintext trusted relay, not a
/// full TURN implementation.
pub struct UdpForwarder {
    socket: UdpSocket,
    peers: Vec<SocketAddr>,
    pub forwarded: u64,
    pub dropped: u64,
}

impl UdpForwarder {
    pub fn bind(addr: SocketAddr) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            peers: Vec::new(),
            forwarded: 0,
            dropped: 0,
        })
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    pub fn peers(&self) -> &[SocketAddr] {
        &self.peers
    }

    /// Drains pending datagrams and forwards them between the two peers.
    pub fn poll(&mut self) -> std::io::Result<usize> {
        let mut buffer = vec![0u8; 65536];
        let mut count = 0;
        loop {
            match self.socket.recv_from(&mut buffer) {
                Ok((len, source)) => {
                    if !self.peers.contains(&source) {
                        if self.peers.len() < 2 {
                            self.peers.push(source);
                        } else {
                            self.dropped += 1;
                            continue;
                        }
                    }
                    let Some(target) = self.peers.iter().find(|peer| **peer != source).copied()
                    else {
                        self.dropped += 1;
                        continue;
                    };
                    if self.socket.send_to(&buffer[..len], target).is_ok() {
                        self.forwarded += 1;
                        count += 1;
                    } else {
                        self.dropped += 1;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
        Ok(count)
    }
}

/// Framed multi-peer relay: clients register, receive an id, and send
/// `[destination id: u16][payload]`; the relay forwards
/// `[source id: u16][payload]` to the destination.
pub const RELAY_JOIN: &[u8] = b"MANATO_RELAY_JOIN_V1";
pub const RELAY_WELCOME: &[u8] = b"MANATO_RELAY_WELCOME_V1:";
pub const MAX_RELAY_PEERS: usize = 8;

pub struct MultiRelay {
    socket: UdpSocket,
    peers: std::collections::HashMap<u16, SocketAddr>,
    next_id: u16,
    auth: Option<crate::relay_auth::RelayAuthenticator>,
    pub forwarded: u64,
    pub dropped: u64,
}

impl MultiRelay {
    pub fn bind(addr: SocketAddr) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            peers: std::collections::HashMap::new(),
            next_id: 0,
            auth: None,
            forwarded: 0,
            dropped: 0,
        })
    }

    /// Enables shared-secret authentication for registering peers.
    pub fn bind_with_auth(addr: SocketAddr, secret: impl Into<String>) -> std::io::Result<Self> {
        let mut relay = Self::bind(addr)?;
        relay.auth = Some(crate::relay_auth::RelayAuthenticator::new(secret));
        Ok(relay)
    }

    /// Enables realm/user-password long-term credential authentication.
    pub fn bind_with_long_term_auth(
        addr: SocketAddr,
        realm: impl Into<String>,
        users: impl IntoIterator<Item = (String, String)>,
    ) -> std::io::Result<Self> {
        let mut relay = Self::bind(addr)?;
        relay.auth = Some(crate::relay_auth::RelayAuthenticator::new_long_term(
            realm, users,
        ));
        Ok(relay)
    }

    fn register(&mut self, source: SocketAddr) -> Option<u16> {
        if self.peers.len() >= MAX_RELAY_PEERS {
            self.dropped += 1;
            return None;
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.peers.retain(|_, peer| *peer != source);
        self.peers.insert(id, source);
        let mut welcome = RELAY_WELCOME.to_vec();
        welcome.extend_from_slice(&id.to_be_bytes());
        let _ = self.socket.send_to(&welcome, source);
        Some(id)
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    pub fn peers(&self) -> &std::collections::HashMap<u16, SocketAddr> {
        &self.peers
    }

    pub fn poll(&mut self) -> std::io::Result<usize> {
        let mut buffer = vec![0u8; 65536];
        let mut count = 0;
        loop {
            match self.socket.recv_from(&mut buffer) {
                Ok((len, source)) => {
                    if &buffer[..len] == RELAY_JOIN {
                        if self.auth.is_some() {
                            self.dropped += 1;
                            continue;
                        }
                        self.register(source);
                        continue;
                    }
                    if buffer[..len].starts_with(crate::relay_auth::AUTH_JOIN) {
                        let Some(request) = crate::relay_auth::AuthRequest::decode(&buffer[..len])
                        else {
                            self.dropped += 1;
                            continue;
                        };
                        let allowed = self
                            .auth
                            .as_mut()
                            .is_some_and(|auth| auth.validate(&request));
                        if !allowed {
                            self.dropped += 1;
                        } else {
                            self.register(source);
                        }
                        continue;
                    }
                    if len < 2 {
                        self.dropped += 1;
                        continue;
                    }
                    let destination = u16::from_be_bytes([buffer[0], buffer[1]]);
                    let Some(target) = self.peers.get(&destination).copied() else {
                        self.dropped += 1;
                        continue;
                    };
                    let Some(source_id) = self
                        .peers
                        .iter()
                        .find(|(_, peer)| **peer == source)
                        .map(|(id, _)| *id)
                    else {
                        self.dropped += 1;
                        continue;
                    };
                    let mut frame = Vec::with_capacity(len + 1);
                    frame.extend_from_slice(&source_id.to_be_bytes());
                    frame.extend_from_slice(&buffer[2..len]);
                    if self.socket.send_to(&frame, target).is_ok() {
                        self.forwarded += 1;
                        count += 1;
                    } else {
                        self.dropped += 1;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
        Ok(count)
    }
}

/// Client for [`MultiRelay`]. Registration is asynchronous; call [`Self::poll`]
/// until [`Self::id`] is assigned.
pub struct RelayClient {
    socket: UdpSocket,
    relay: SocketAddr,
    id: Option<u16>,
}

impl RelayClient {
    pub fn connect(relay: SocketAddr) -> std::io::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_nonblocking(true)?;
        socket.send_to(RELAY_JOIN, relay)?;
        Ok(Self {
            socket,
            relay,
            id: None,
        })
    }

    /// Registers with long-term realm/user/password credentials.
    pub fn connect_long_term(
        relay: SocketAddr,
        realm: &str,
        username: &str,
        password: &str,
        nonce: u64,
    ) -> std::io::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_nonblocking(true)?;
        let request =
            crate::relay_auth::AuthRequest::new_long_term(realm, username, password, nonce);
        socket.send_to(&request.encode(), relay)?;
        Ok(Self {
            socket,
            relay,
            id: None,
        })
    }

    /// Registers with shared-secret authentication.
    pub fn connect_auth(
        relay: SocketAddr,
        username: &str,
        secret: &str,
        nonce: u64,
    ) -> std::io::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_nonblocking(true)?;
        let request = crate::relay_auth::AuthRequest::new(secret, username, nonce);
        socket.send_to(&request.encode(), relay)?;
        Ok(Self {
            socket,
            relay,
            id: None,
        })
    }

    pub fn id(&self) -> Option<u16> {
        self.id
    }

    pub fn send(&self, destination: u16, payload: &[u8]) -> std::io::Result<()> {
        let mut frame = Vec::with_capacity(payload.len() + 2);
        frame.extend_from_slice(&destination.to_be_bytes());
        frame.extend_from_slice(payload);
        self.socket.send_to(&frame, self.relay)?;
        Ok(())
    }

    /// Drains welcome/forwarded frames; returns `(source id, payload)`.
    pub fn poll(&mut self) -> std::io::Result<Vec<(u16, Vec<u8>)>> {
        let mut buffer = vec![0u8; 65536];
        let mut received = Vec::new();
        loop {
            match self.socket.recv_from(&mut buffer) {
                Ok((len, _)) => {
                    if let Some(id) = buffer[..len]
                        .strip_prefix(RELAY_WELCOME)
                        .and_then(|bytes| bytes.get(..2))
                    {
                        self.id = Some(u16::from_be_bytes([id[0], id[1]]));
                        continue;
                    }
                    if len < 2 {
                        continue;
                    }
                    let source = u16::from_be_bytes([buffer[0], buffer[1]]);
                    received.push((source, buffer[2..len].to_vec()));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
        Ok(received)
    }
}

/// GGRS socket that tunnels through [`MultiRelay`] framing.
pub struct RelayGgrsSocket {
    socket: UdpSocket,
    relay: SocketAddr,
    peer_id: u16,
    remote: SocketAddr,
}

impl RelayGgrsSocket {
    /// Registers at the relay, waits for the assigned id, and targets
    /// `peer_id`. `remote` is the synthetic address reported to GGRS.
    pub fn connect(
        relay: SocketAddr,
        peer_id: u16,
        username: Option<(&str, &str, u64)>,
    ) -> std::io::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_nonblocking(true)?;
        if let Some((username, secret, nonce)) = username {
            let request = crate::relay_auth::AuthRequest::new(secret, username, nonce);
            socket.send_to(&request.encode(), relay)?;
        } else {
            socket.send_to(RELAY_JOIN, relay)?;
        }
        let mut buffer = [0u8; 256];
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(2) {
            if let Ok((len, _)) = socket.recv_from(&mut buffer) {
                if buffer[..len].starts_with(RELAY_WELCOME) {
                    return Ok(Self {
                        socket,
                        relay,
                        peer_id,
                        remote: SocketAddr::from(([127, 0, 0, 1], 1)),
                    });
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "relay registration timed out",
        ))
    }

    pub fn remote(&self) -> SocketAddr {
        self.remote
    }

    fn send_raw(&self, payload: &[u8]) {
        let mut frame = Vec::with_capacity(payload.len() + 2);
        frame.extend_from_slice(&self.peer_id.to_be_bytes());
        frame.extend_from_slice(payload);
        let _ = self.socket.send_to(&frame, self.relay_addr());
    }

    fn relay_addr(&self) -> SocketAddr {
        // The socket is connected to the relay during construction; recover the
        // address from the last send target by remembering it separately.
        self.relay
    }
}

impl ggrs::NonBlockingSocket<SocketAddr> for RelayGgrsSocket {
    fn send_to(&mut self, msg: &ggrs::Message, _addr: &SocketAddr) {
        if let Ok(payload) = bincode::serialize(msg) {
            self.send_raw(&payload);
        }
    }

    fn receive_all_messages(&mut self) -> Vec<(SocketAddr, ggrs::Message)> {
        let mut out = Vec::new();
        let mut buffer = vec![0u8; 65536];
        loop {
            match self.socket.recv_from(&mut buffer) {
                Ok((len, _)) => {
                    if len < 2 || buffer[..len].starts_with(RELAY_WELCOME) {
                        continue;
                    }
                    let source = u16::from_be_bytes([buffer[0], buffer[1]]);
                    if source != self.peer_id {
                        continue;
                    }
                    if let Ok(message) = bincode::deserialize::<ggrs::Message>(&buffer[2..len]) {
                        out.push((self.remote, message));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn forwards_between_two_peers() {
        let mut relay = UdpForwarder::bind("127.0.0.1:0".parse().unwrap()).expect("relay");
        let relay_addr = relay.local_addr().expect("addr");
        let a = UdpSocket::bind("127.0.0.1:0").expect("a");
        let b = UdpSocket::bind("127.0.0.1:0").expect("b");
        a.set_nonblocking(true).expect("a nonblocking");
        b.set_nonblocking(true).expect("b nonblocking");
        let a_addr = a.local_addr().expect("a addr");
        let b_addr = b.local_addr().expect("b addr");

        let started = Instant::now();
        let mut a_got = None;
        let mut b_got = None;
        while started.elapsed() < Duration::from_secs(1) && (a_got.is_none() || b_got.is_none()) {
            a.send_to(b"from-a", relay_addr).expect("a send");
            b.send_to(b"from-b", relay_addr).expect("b send");
            relay.poll().expect("poll");
            let mut buffer = [0u8; 64];
            if let Ok((len, _)) = a.recv_from(&mut buffer) {
                a_got = Some(buffer[..len].to_vec());
            }
            if let Ok((len, _)) = b.recv_from(&mut buffer) {
                b_got = Some(buffer[..len].to_vec());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(a_got.as_deref(), Some(&b"from-b"[..]));
        assert_eq!(b_got.as_deref(), Some(&b"from-a"[..]));
        assert_eq!(relay.peers(), &[a_addr, b_addr]);
        assert!(relay.forwarded >= 2);
    }

    #[test]
    fn multi_relay_forwards_by_destination_id() {
        let mut relay = MultiRelay::bind("127.0.0.1:0".parse().unwrap()).expect("relay");
        let relay_addr = relay.local_addr().expect("addr");
        let mut clients: Vec<RelayClient> = (0..3)
            .map(|_| RelayClient::connect(relay_addr).expect("client"))
            .collect();
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(1)
            && clients.iter().any(|client| client.id().is_none())
        {
            relay.poll().expect("relay");
            for client in &mut clients {
                client.poll().expect("poll");
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(clients.len(), 3);
        assert!(clients.iter().all(|client| client.id().is_some()));
        clients[0].send(2, b"hello").expect("send");
        let mut got = Vec::new();
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(1) {
            relay.poll().expect("relay");
            for client in &mut clients {
                got.extend(client.poll().expect("poll"));
            }
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, 0);
        assert_eq!(got[0].1, b"hello");
    }

    #[test]
    fn multi_relay_requires_authenticated_join() {
        let mut relay =
            MultiRelay::bind_with_auth("127.0.0.1:0".parse().unwrap(), "token").expect("relay");
        let relay_addr = relay.local_addr().expect("addr");
        let mut good = RelayClient::connect_auth(relay_addr, "player", "token", 1).expect("good");
        let mut bad = RelayClient::connect_auth(relay_addr, "player", "wrong", 2).expect("bad");
        let unauth = RelayClient::connect(relay_addr).expect("unauth");
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(1) && good.id().is_none() {
            relay.poll().expect("relay");
            good.poll().expect("good poll");
            bad.poll().expect("bad poll");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(good.id().is_some());
        assert!(bad.id().is_none());
        assert!(
            relay.peers().len() == 1,
            "only the authenticated peer registers"
        );
        drop(unauth);
        assert!(relay.dropped >= 2, "bad and unauthenticated joins dropped");
    }

    #[test]
    fn multi_relay_long_term_credentials() {
        let mut relay = MultiRelay::bind_with_long_term_auth(
            "127.0.0.1:0".parse().unwrap(),
            "manato",
            [("alice".to_string(), "secret".to_string())],
        )
        .expect("relay");
        let relay_addr = relay.local_addr().expect("addr");
        let mut good = RelayClient::connect_long_term(relay_addr, "manato", "alice", "secret", 1)
            .expect("good");
        let mut bad =
            RelayClient::connect_long_term(relay_addr, "manato", "alice", "wrong", 2).expect("bad");
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(1) && good.id().is_none() {
            relay.poll().expect("relay");
            good.poll().expect("good poll");
            bad.poll().expect("bad poll");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(good.id().is_some());
        assert!(bad.id().is_none());
        assert_eq!(relay.peers().len(), 1);
        assert!(relay.dropped >= 1);
    }

    #[test]
    fn ignores_third_peer() {
        let mut relay = UdpForwarder::bind("127.0.0.1:0".parse().unwrap()).expect("relay");
        let relay_addr = relay.local_addr().expect("addr");
        for _ in 0..3 {
            let socket = UdpSocket::bind("127.0.0.1:0").expect("peer");
            socket.send_to(b"x", relay_addr).expect("send");
            std::thread::sleep(Duration::from_millis(2));
            relay.poll().expect("poll");
        }
        assert_eq!(relay.peers().len(), 2);
        assert!(relay.dropped > 0);
    }
}
