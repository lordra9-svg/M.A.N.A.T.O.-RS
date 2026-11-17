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
