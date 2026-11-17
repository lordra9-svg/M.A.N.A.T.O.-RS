//! Best-effort UDP NAT hole punching for two players that have exchanged
//! public addresses (for example through the lobby `Peer` handoff).

use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

pub const PUNCH: &[u8] = b"MANATO_PUNCH_V1";
pub const PUNCH_ACK: &[u8] = b"MANATO_PUNCH_ACK_V1";

/// Sends/receives punch datagrams on the same local port a netplay socket will
/// later bind. Both peers must run this at the same time for best results.
pub struct NatPuncher {
    socket: UdpSocket,
    remote: SocketAddr,
    pub punched: bool,
    pub packets_sent: u32,
    pub packets_received: u32,
}

impl NatPuncher {
    pub fn bind(local: SocketAddr, remote: SocketAddr) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(local)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            remote,
            punched: false,
            packets_sent: 0,
            packets_received: 0,
        })
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// Sends one punch datagram.
    pub fn send(&mut self) -> std::io::Result<()> {
        if self.socket.send_to(PUNCH, self.remote).is_ok() {
            self.packets_sent += 1;
        }
        Ok(())
    }

    /// Drains punch traffic. Receiving either a punch or an ack marks the path
    /// open, since both prove the peer's mapping reached this socket.
    pub fn poll(&mut self) -> std::io::Result<bool> {
        let mut buffer = [0u8; 64];
        loop {
            match self.socket.recv_from(&mut buffer) {
                Ok((len, _)) => {
                    self.packets_received += 1;
                    if &buffer[..len] == PUNCH {
                        let _ = self.socket.send_to(PUNCH_ACK, self.remote);
                        self.punched = true;
                    } else if &buffer[..len] == PUNCH_ACK {
                        self.punched = true;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
        Ok(self.punched)
    }

    /// Pumps punches until both sides open or `timeout` elapses.
    pub fn punch(&mut self, timeout: Duration) -> std::io::Result<bool> {
        let started = Instant::now();
        while started.elapsed() < timeout {
            self.send()?;
            self.poll()?;
            if self.punched {
                return Ok(true);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(self.punched)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_peers_punch_through_to_each_other() {
        let mut a = NatPuncher::bind(
            "127.0.0.1:0".parse().unwrap(),
            "127.0.0.1:1".parse().unwrap(),
        )
        .expect("a");
        let mut b = NatPuncher::bind(
            "127.0.0.1:0".parse().unwrap(),
            "127.0.0.1:1".parse().unwrap(),
        )
        .expect("b");
        let a_addr = a.local_addr().expect("a addr");
        let b_addr = b.local_addr().expect("b addr");
        a.remote = b_addr;
        b.remote = a_addr;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(2) {
            a.send().expect("a send");
            b.send().expect("b send");
            a.poll().expect("a poll");
            b.poll().expect("b poll");
            if a.punched && b.punched {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(a.punched && b.punched);
        assert!(a.packets_received > 0 && b.packets_received > 0);
    }

    #[test]
    fn punch_timeout_reports_failure() {
        let mut puncher = NatPuncher::bind(
            "127.0.0.1:0".parse().unwrap(),
            "127.0.0.1:9".parse().unwrap(),
        )
        .expect("puncher");
        assert!(!puncher.punch(Duration::from_millis(80)).expect("punch"));
        assert!(!puncher.punched);
        assert!(puncher.packets_sent > 0);
    }
}
