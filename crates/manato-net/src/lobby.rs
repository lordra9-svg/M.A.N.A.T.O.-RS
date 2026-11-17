//! Transport-agnostic lobby and spectator session state (N-03).
//!
//! The lobby protocol carries join/leave/ready/start/frame messages over any
//! transport; the UDP relay wiring is host-provided.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use bincode::Options as _;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LobbyRole {
    Host,
    Guest,
    Spectator,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbyMember {
    pub id: u32,
    pub name: String,
    pub role: LobbyRole,
    pub ready: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LobbyPhase {
    Open,
    Starting,
    InMatch,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LobbyState {
    pub host: u32,
    /// Player slots (spectators do not occupy these).
    pub max_players: usize,
    pub members: Vec<LobbyMember>,
    pub phase: LobbyPhase,
    next_id: u32,
}

impl LobbyState {
    pub fn new(host_name: &str, max_players: usize) -> (Self, u32) {
        let host = LobbyMember {
            id: 0,
            name: host_name.to_string(),
            role: LobbyRole::Host,
            ready: true,
        };
        (
            Self {
                host: 0,
                max_players: max_players.max(1),
                members: vec![host],
                phase: LobbyPhase::Open,
                next_id: 1,
            },
            0,
        )
    }

    pub fn player_count(&self) -> usize {
        self.members
            .iter()
            .filter(|member| member.role != LobbyRole::Spectator)
            .count()
    }

    pub fn spectator_count(&self) -> usize {
        self.members
            .iter()
            .filter(|member| member.role == LobbyRole::Spectator)
            .count()
    }

    /// Adds a member. Players are rejected once the player slots are full;
    /// spectators are capped at 32 total connections.
    pub fn join(&mut self, name: &str, spectator: bool) -> Option<u32> {
        if self.phase != LobbyPhase::Open {
            return None;
        }
        if spectator {
            if self.members.len() >= 32 {
                return None;
            }
        } else if self.player_count() >= self.max_players {
            return None;
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        // Cap names so snapshot messages stay within the 4 KiB datagram limit.
        let name: String = name.chars().take(32).collect();
        self.members.push(LobbyMember {
            id,
            name,
            role: if spectator {
                LobbyRole::Spectator
            } else {
                LobbyRole::Guest
            },
            ready: false,
        });
        Some(id)
    }

    pub fn leave(&mut self, id: u32) -> bool {
        let Some(index) = self.members.iter().position(|member| member.id == id) else {
            return false;
        };
        self.members.remove(index);
        if self.members.is_empty() {
            self.phase = LobbyPhase::Closed;
            return true;
        }
        if self.host == id {
            if let Some(next) = self
                .members
                .iter_mut()
                .find(|member| member.role != LobbyRole::Spectator)
            {
                next.role = LobbyRole::Host;
                next.ready = true;
                self.host = next.id;
            } else {
                // Only spectators remain: nobody can start a match.
                self.phase = LobbyPhase::Closed;
            }
        }
        true
    }

    pub fn set_ready(&mut self, id: u32, ready: bool) -> bool {
        if self.phase != LobbyPhase::Open {
            return false;
        }
        let Some(member) = self.members.iter_mut().find(|member| member.id == id) else {
            return false;
        };
        if member.role == LobbyRole::Spectator {
            return false;
        }
        member.ready = ready;
        true
    }

    pub fn all_players_ready(&self) -> bool {
        self.members
            .iter()
            .filter(|member| member.role != LobbyRole::Spectator)
            .all(|member| member.ready)
    }

    /// The host starts the match once every player is ready.
    pub fn start(&mut self, id: u32) -> bool {
        if id != self.host
            || self.phase != LobbyPhase::Open
            || self.player_count() < 2
            || !self.all_players_ready()
        {
            return false;
        }
        self.phase = LobbyPhase::Starting;
        true
    }

    pub fn enter_match(&mut self) -> bool {
        if self.phase != LobbyPhase::Starting {
            return false;
        }
        self.phase = LobbyPhase::InMatch;
        true
    }
}

/// Lobby/spectator wire messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LobbyMessage {
    Join {
        name: String,
        spectator: bool,
    },
    Leave {
        id: u32,
    },
    Ready {
        id: u32,
        ready: bool,
    },
    Start {
        id: u32,
    },
    Frame {
        frame: i32,
        checksum: u64,
    },
    /// Server → client: assigned member id and current host.
    Welcome {
        id: u32,
        host: u32,
    },
    /// Host → spectators: one chunk of a rollback state snapshot.
    Snapshot {
        frame: i32,
        checksum: u64,
        index: u16,
        total: u16,
        data: Vec<u8>,
    },
    /// Server → clients: full lobby snapshot.
    State {
        host: u32,
        phase: LobbyPhase,
        members: Vec<LobbyMember>,
    },
}

impl LobbyMessage {
    pub fn encode(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap_or_default()
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        // Bound allocations from hostile length prefixes; datagrams are ≤4 KiB.
        bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .with_limit(4096)
            .deserialize(bytes)
            .ok()
    }
}

/// Spectator rollback-frame tracking for relayed host frames.
#[derive(Debug, Clone, Default)]
pub struct SpectatorSync {
    pub last_frame: i32,
    pub checksum: u64,
    pub missed_frames: u32,
    pub frames_received: u32,
}

impl SpectatorSync {
    /// Accepts the next newer frame; stale/duplicate frames are rejected.
    pub fn on_frame(&mut self, frame: i32, checksum: u64) -> bool {
        if frame <= self.last_frame {
            return false;
        }
        if self.last_frame > 0 && frame > self.last_frame + 1 {
            self.missed_frames += (frame - self.last_frame - 1) as u32;
        }
        self.last_frame = frame;
        self.checksum = checksum;
        self.frames_received += 1;
        true
    }

    /// True when more than a second of frames has gone missing.
    pub fn lagging(&self) -> bool {
        self.missed_frames > 60
    }
}

/// Lobby protocol server: maps peers to member ids and produces replies to
/// send back over any transport.
#[derive(Debug, Clone)]
pub struct LobbyServer {
    pub state: LobbyState,
    pub peers: HashMap<u32, SocketAddr>,
    join_windows: HashMap<IpAddr, (Instant, u32)>,
    join_limit: u32,
    join_window: Duration,
}

impl LobbyServer {
    pub fn new(host_name: &str, max_players: usize, host_addr: SocketAddr) -> (Self, u32) {
        let (state, host) = LobbyState::new(host_name, max_players);
        let mut peers = HashMap::new();
        peers.insert(host, host_addr);
        (
            Self {
                state,
                peers,
                join_windows: HashMap::new(),
                join_limit: 5,
                join_window: Duration::from_secs(10),
            },
            host,
        )
    }

    fn snapshot(&self) -> LobbyMessage {
        LobbyMessage::State {
            host: self.state.host,
            phase: self.state.phase,
            members: self.state.members.clone(),
        }
    }

    fn broadcast(&self) -> Vec<(SocketAddr, LobbyMessage)> {
        let message = self.snapshot();
        self.peers
            .values()
            .map(|addr| (*addr, message.clone()))
            .collect()
    }

    /// Processes one message from `addr` and returns messages to send.
    fn allow_join(&mut self, ip: IpAddr, now: Instant) -> bool {
        let entry = self.join_windows.entry(ip).or_insert((now, 0));
        if now.duration_since(entry.0) >= self.join_window {
            *entry = (now, 0);
        }
        if entry.1 >= self.join_limit {
            return false;
        }
        entry.1 += 1;
        true
    }

    pub fn handle(
        &mut self,
        addr: SocketAddr,
        message: LobbyMessage,
    ) -> Vec<(SocketAddr, LobbyMessage)> {
        self.handle_at(addr, message, Instant::now())
    }

    /// Time-injected variant used by tests and deterministic hosts.
    pub fn handle_at(
        &mut self,
        addr: SocketAddr,
        message: LobbyMessage,
        now: Instant,
    ) -> Vec<(SocketAddr, LobbyMessage)> {
        let sender = self
            .peers
            .iter()
            .find(|(_, peer)| **peer == addr)
            .map(|(id, _)| *id);
        let host_addr = self.peers.get(&self.state.host).copied();
        match message {
            LobbyMessage::Join { name, spectator } => {
                if !self.allow_join(addr.ip(), now) {
                    return Vec::new();
                }
                let Some(id) = self.state.join(&name, spectator) else {
                    return Vec::new();
                };
                self.peers.insert(id, addr);
                let mut replies = vec![(
                    addr,
                    LobbyMessage::Welcome {
                        id,
                        host: self.state.host,
                    },
                )];
                replies.extend(self.broadcast());
                replies
            }
            LobbyMessage::Leave { id } => {
                if sender != Some(id) || !self.state.leave(id) {
                    return Vec::new();
                }
                self.peers.remove(&id);
                self.broadcast()
            }
            LobbyMessage::Ready { id, ready } => {
                if sender != Some(id) || !self.state.set_ready(id, ready) {
                    return Vec::new();
                }
                self.broadcast()
            }
            LobbyMessage::Start { id } => {
                if host_addr != Some(addr) || !self.state.start(id) {
                    return Vec::new();
                }
                self.broadcast()
            }
            LobbyMessage::Frame { frame, checksum } => {
                if host_addr != Some(addr) {
                    return Vec::new();
                }
                let message = LobbyMessage::Frame { frame, checksum };
                self.state
                    .members
                    .iter()
                    .filter(|member| member.role == LobbyRole::Spectator)
                    .filter_map(|member| {
                        self.peers
                            .get(&member.id)
                            .map(|addr| (*addr, message.clone()))
                    })
                    .collect()
            }
            LobbyMessage::Snapshot {
                frame,
                checksum,
                index,
                total,
                data,
            } => {
                if host_addr != Some(addr) {
                    return Vec::new();
                }
                let message = LobbyMessage::Snapshot {
                    frame,
                    checksum,
                    index,
                    total,
                    data,
                };
                self.state
                    .members
                    .iter()
                    .filter(|member| member.role == LobbyRole::Spectator)
                    .filter_map(|member| {
                        self.peers
                            .get(&member.id)
                            .map(|addr| (*addr, message.clone()))
                    })
                    .collect()
            }
            LobbyMessage::Welcome { .. } | LobbyMessage::State { .. } => Vec::new(),
        }
    }
}

/// Non-blocking UDP relay for the lobby protocol.
pub struct UdpRelay {
    socket: UdpSocket,
}

impl UdpRelay {
    pub fn bind(addr: SocketAddr) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(Self { socket })
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// Drains pending datagrams through `server`, sending every reply.
    pub fn poll(
        &self,
        server: &mut LobbyServer,
    ) -> std::io::Result<Vec<(SocketAddr, LobbyMessage)>> {
        let mut buffer = [0u8; 4096];
        let mut sent = Vec::new();
        loop {
            match self.socket.recv_from(&mut buffer) {
                Ok((len, addr)) => {
                    let Some(message) = LobbyMessage::decode(&buffer[..len]) else {
                        continue;
                    };
                    for (target, reply) in server.handle(addr, message) {
                        let bytes = reply.encode();
                        if self.socket.send_to(&bytes, target).is_ok() {
                            sent.push((target, reply));
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
        Ok(sent)
    }

    /// Sends a message directly (host-side helpers, tests).
    pub fn send_to(&self, message: &LobbyMessage, addr: SocketAddr) -> std::io::Result<usize> {
        self.socket.send_to(&message.encode(), addr)
    }
}

/// Payload size per snapshot chunk (keeps datagrams well under 4 KiB).
pub const SNAPSHOT_CHUNK_SIZE: usize = 1024;
/// Maximum reassembled spectator snapshot (chunks × size).
pub const MAX_SNAPSHOT_SIZE: usize = 256 * 1024;

/// Splits a serialized state snapshot into relayable chunks.
pub fn chunk_snapshot(frame: i32, checksum: u64, bytes: &[u8]) -> Vec<LobbyMessage> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let total = bytes
        .len()
        .div_ceil(SNAPSHOT_CHUNK_SIZE)
        .min(u16::MAX as usize);
    bytes
        .chunks(SNAPSHOT_CHUNK_SIZE)
        .enumerate()
        .map(|(index, chunk)| LobbyMessage::Snapshot {
            frame,
            checksum,
            index: index as u16,
            total: total as u16,
            data: chunk.to_vec(),
        })
        .collect()
}

/// Splits the current simulation snapshot into spectator relay messages.
pub fn snapshot_messages(
    sim: &manato_sim::SimState,
    frame: i32,
) -> anyhow::Result<Vec<LobbyMessage>> {
    let bytes = sim.encode_snapshot()?;
    anyhow::ensure!(
        bytes.len() <= MAX_SNAPSHOT_SIZE,
        "snapshot is too large for spectator streaming ({} bytes)",
        bytes.len()
    );
    Ok(chunk_snapshot(frame, sim.checksum(), &bytes))
}

/// Reassembles host snapshot chunks and restores them into a simulation.
#[derive(Debug, Clone, Default)]
pub struct SpectatorSnapshotReceiver {
    assembler: SnapshotAssembler,
}

impl SpectatorSnapshotReceiver {
    /// Feeds one message; returns true when a complete snapshot was restored.
    pub fn accept(
        &mut self,
        sim: &mut manato_sim::SimState,
        message: &LobbyMessage,
    ) -> anyhow::Result<bool> {
        let LobbyMessage::Snapshot {
            frame,
            checksum,
            index,
            total,
            data,
        } = message
        else {
            return Ok(false);
        };
        let Some(bytes) = self
            .assembler
            .accept(*frame, *checksum, *index, *total, data)
        else {
            return Ok(false);
        };
        sim.decode_snapshot(&bytes)?;
        anyhow::ensure!(
            sim.checksum() == *checksum,
            "restored snapshot checksum mismatch"
        );
        Ok(true)
    }
}

/// Continuous host-side snapshot broadcaster over a direct UDP socket.
///
/// Spectators join by sending [`LobbyMessage::Join`]; the host sends chunked
/// [`LobbyMessage::Snapshot`] messages every `interval_ticks` simulation
/// frames. This is the transport used by the in-app `--broadcast` mode.
pub struct SnapshotBroadcaster {
    socket: UdpSocket,
    spectators: Vec<SocketAddr>,
    pub interval_ticks: i32,
    last_frame: i32,
}

impl SnapshotBroadcaster {
    pub fn bind(addr: SocketAddr) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            spectators: Vec::new(),
            interval_ticks: 6,
            last_frame: -1,
        })
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    pub fn spectator_count(&self) -> usize {
        self.spectators.len()
    }

    /// Registers joining spectators and removes leaving ones.
    pub fn poll(&mut self) -> std::io::Result<()> {
        let mut buffer = [0u8; 4096];
        loop {
            match self.socket.recv_from(&mut buffer) {
                Ok((len, addr)) => {
                    let Some(message) = LobbyMessage::decode(&buffer[..len]) else {
                        continue;
                    };
                    match message {
                        LobbyMessage::Join {
                            spectator: true, ..
                        } => {
                            if !self.spectators.contains(&addr) {
                                self.spectators.push(addr);
                            }
                        }
                        LobbyMessage::Leave { .. } => {
                            self.spectators.retain(|peer| *peer != addr);
                        }
                        _ => {}
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// Sends the current snapshot when the interval has elapsed. Returns the
    /// number of datagrams sent.
    pub fn broadcast(&mut self, sim: &manato_sim::SimState, frame: i32) -> anyhow::Result<usize> {
        if self.spectators.is_empty() {
            return Ok(0);
        }
        if self.last_frame >= 0 && frame - self.last_frame < self.interval_ticks {
            return Ok(0);
        }
        let messages = snapshot_messages(sim, frame)?;
        let mut sent = 0;
        for address in &self.spectators {
            for message in &messages {
                let bytes = message.encode();
                if self.socket.send_to(&bytes, address).is_ok() {
                    sent += 1;
                }
            }
        }
        self.last_frame = frame;
        Ok(sent)
    }
}

/// Spectator client for the direct broadcaster socket.
pub struct SpectatorClient {
    socket: UdpSocket,
    host: SocketAddr,
    receiver: SpectatorSnapshotReceiver,
    last_join: Instant,
    pub frame: i32,
    pub checksum: u64,
    pub connected: bool,
}

impl SpectatorClient {
    pub fn connect(host: SocketAddr) -> std::io::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_nonblocking(true)?;
        let client = Self {
            socket,
            host,
            receiver: SpectatorSnapshotReceiver::default(),
            last_join: Instant::now(),
            frame: -1,
            checksum: 0,
            connected: false,
        };
        client.send_join();
        Ok(client)
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    fn send_join(&self) {
        let message = LobbyMessage::Join {
            name: "spectator".into(),
            spectator: true,
        };
        let _ = self.socket.send_to(&message.encode(), self.host);
    }

    /// Polls snapshots and restores complete states. Returns true when a
    /// snapshot was applied this call.
    pub fn poll(&mut self, sim: &mut manato_sim::SimState) -> anyhow::Result<bool> {
        let mut restored = false;
        let mut buffer = [0u8; 4096];
        loop {
            match self.socket.recv_from(&mut buffer) {
                Ok((len, _)) => {
                    let Some(message) = LobbyMessage::decode(&buffer[..len]) else {
                        continue;
                    };
                    if let LobbyMessage::Snapshot {
                        frame, checksum, ..
                    } = &message
                    {
                        self.frame = *frame;
                        self.checksum = *checksum;
                    }
                    if self.receiver.accept(sim, &message)? {
                        restored = true;
                        self.connected = true;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error.into()),
            }
        }
        // Retry the join until the first snapshot arrives, then once a second
        // to survive a restarted host.
        let retry = if self.connected {
            Duration::from_secs(1)
        } else {
            Duration::from_millis(250)
        };
        if self.last_join.elapsed() >= retry {
            self.send_join();
            self.last_join = Instant::now();
        }
        Ok(restored)
    }

    pub fn status_text(&self) -> String {
        if self.connected {
            format!(
                "SPECTATING frame {} checksum {:016x}",
                self.frame, self.checksum
            )
        } else {
            format!("SPECTATING - waiting for host {}", self.host)
        }
    }
}

/// Reassembles host snapshot chunks for a spectator, rejecting stale frames,
/// duplicates, oversized payloads, and checksum mismatches.
#[derive(Debug, Clone, Default)]
pub struct SnapshotAssembler {
    frame: i32,
    checksum: u64,
    total: u16,
    parts: Vec<Option<Vec<u8>>>,
    received: u16,
}

impl SnapshotAssembler {
    pub fn accept(
        &mut self,
        frame: i32,
        checksum: u64,
        index: u16,
        total: u16,
        data: &[u8],
    ) -> Option<Vec<u8>> {
        if total == 0
            || index >= total
            || data.len() > SNAPSHOT_CHUNK_SIZE
            || total as usize * SNAPSHOT_CHUNK_SIZE > MAX_SNAPSHOT_SIZE
        {
            return None;
        }
        if frame < self.frame || (frame == self.frame && checksum != self.checksum) {
            return None;
        }
        if frame > self.frame {
            self.frame = frame;
            self.checksum = checksum;
            self.total = total;
            self.parts = vec![None; total as usize];
            self.received = 0;
        } else if total != self.total {
            return None;
        }
        let slot = &mut self.parts[index as usize];
        if slot.is_some() {
            return None;
        }
        *slot = Some(data.to_vec());
        self.received += 1;
        if self.received != self.total {
            return None;
        }
        let mut out = Vec::new();
        for part in &self.parts {
            out.extend_from_slice(part.as_deref()?);
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lobby_capacity_ready_and_start() {
        let (mut lobby, host) = LobbyState::new("host", 2);
        let guest = lobby.join("guest", false).expect("guest");
        let spectator = lobby.join("watcher", true).expect("spectator");
        assert_eq!(lobby.player_count(), 2);
        assert_eq!(lobby.spectator_count(), 1);
        assert!(lobby.join("third", false).is_none(), "player slots full");
        assert!(!lobby.set_ready(spectator, true));
        assert!(!lobby.start(host));
        assert!(lobby.set_ready(guest, true));
        assert!(!lobby.start(guest), "only the host starts");
        assert!(lobby.start(host));
        assert!(lobby.enter_match());
        assert!(lobby.join("late", false).is_none());
    }

    #[test]
    fn host_transfer_and_close() {
        let (mut lobby, host) = LobbyState::new("host", 2);
        let guest = lobby.join("guest", false).expect("guest");
        let spectator = lobby.join("watcher", true).expect("spectator");
        assert!(lobby.leave(host));
        assert_eq!(lobby.host, guest);
        assert_eq!(
            lobby.members.iter().find(|m| m.id == guest).unwrap().role,
            LobbyRole::Host
        );
        let _ = spectator;
        assert!(lobby.leave(guest));
        assert_eq!(lobby.phase, LobbyPhase::Closed);
    }

    #[test]
    fn join_rate_limit_and_name_cap() {
        let addr: SocketAddr = "127.0.0.1:9200".parse().unwrap();
        let (mut server, _host) = LobbyServer::new("host", 16, addr);
        let now = std::time::Instant::now();
        let mut accepted = 0;
        for index in 0..6 {
            let replies = server.handle_at(
                addr,
                LobbyMessage::Join {
                    name: "x".repeat(200),
                    spectator: true,
                },
                now,
            );
            if !replies.is_empty() {
                accepted += 1;
            }
            assert!(index < 5 || replies.is_empty());
        }
        assert_eq!(accepted, 5, "per-IP join rate limit");
        assert!(server
            .state
            .members
            .iter()
            .all(|member| member.name.chars().count() <= 32));
        // A later window resets the budget.
        let later = now + std::time::Duration::from_secs(11);
        assert!(!server
            .handle_at(
                addr,
                LobbyMessage::Join {
                    name: "late".into(),
                    spectator: true,
                },
                later,
            )
            .is_empty());
    }

    #[test]
    fn snapshot_chunks_reassemble_and_reject_bad_input() {
        let payload: Vec<u8> = (0..3000u32).map(|v| v as u8).collect();
        let chunks = chunk_snapshot(5, 99, &payload);
        assert!(chunks.len() >= 3);
        let mut assembler = SnapshotAssembler::default();
        let mut complete = None;
        for (position, chunk) in chunks.iter().enumerate().rev() {
            let LobbyMessage::Snapshot {
                frame,
                checksum,
                index,
                total,
                data,
            } = chunk
            else {
                panic!("snapshot variant");
            };
            let result = assembler.accept(*frame, *checksum, *index, *total, data);
            if position == 0 {
                complete = result;
            } else {
                assert!(result.is_none());
            }
        }
        assert_eq!(complete.as_deref(), Some(payload.as_slice()));

        // Stale frame, duplicate, oversized chunk, and bad totals are rejected.
        assert!(assembler.accept(4, 99, 0, 1, &[1, 2, 3]).is_none());
        assert!(assembler.accept(5, 99, 0, 3, &[1]).is_none());
        assert!(assembler
            .accept(6, 1, 0, 1, &vec![0; SNAPSHOT_CHUNK_SIZE + 1])
            .is_none());
        assert!(assembler
            .accept(
                6,
                1,
                0,
                (MAX_SNAPSHOT_SIZE / SNAPSHOT_CHUNK_SIZE + 1) as u16,
                &[1]
            )
            .is_none());
    }

    #[test]
    fn host_snapshot_round_trips_through_spectator_receiver() {
        use std::path::PathBuf;
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = root.join("content/chars/kaen/kaen.def");
        if !source.exists() {
            eprintln!("skipping: bundled content missing");
            return;
        }
        let config = manato_sim::MatchConfig {
            mugen_root: root.clone(),
            p1: source,
            p2: PathBuf::from("content/chars/nival/nival.def"),
            stage: PathBuf::from("content/stages/training/stage1.def"),
            screenpack: None,
            rounds_to_win: 1,
            round_time: 99,
        };
        let mut host = manato_sim::SimState::load(&config).expect("host state");
        for _ in 0..5 {
            host.tick([manato_core::PlayerInput::default(); 2]);
        }
        let checksum = host.checksum();
        let messages = snapshot_messages(&host, 5).expect("snapshot");
        assert!(!messages.is_empty());

        let mut spectator = manato_sim::SimState::load(&config).expect("spectator state");
        let mut receiver = SpectatorSnapshotReceiver::default();
        let mut restored = false;
        for message in messages.iter().rev() {
            restored |= receiver.accept(&mut spectator, message).expect("accept");
        }
        assert!(restored);
        assert_eq!(spectator.checksum(), checksum);
    }

    #[test]
    fn direct_broadcaster_streams_snapshots_at_interval() {
        use std::path::PathBuf;
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = root.join("content/chars/kaen/kaen.def");
        if !source.exists() {
            eprintln!("skipping: bundled content missing");
            return;
        }
        let config = manato_sim::MatchConfig {
            mugen_root: root.clone(),
            p1: source,
            p2: PathBuf::from("content/chars/nival/nival.def"),
            stage: PathBuf::from("content/stages/training/stage1.def"),
            screenpack: None,
            rounds_to_win: 1,
            round_time: 99,
        };
        let mut host = manato_sim::SimState::load(&config).expect("host state");
        for _ in 0..4 {
            host.tick([manato_core::PlayerInput::default(); 2]);
        }
        let checksum = host.checksum();

        let mut broadcaster =
            SnapshotBroadcaster::bind("127.0.0.1:0".parse().unwrap()).expect("broadcaster");
        let address = broadcaster.local_addr().expect("addr");
        let mut client = SpectatorClient::connect(address).expect("client");
        for _ in 0..50 {
            broadcaster.poll().expect("poll joins");
            if broadcaster.spectator_count() == 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(broadcaster.spectator_count(), 1);

        let mut spectator = manato_sim::SimState::load(&config).expect("spectator state");
        let mut restored = false;
        // UDP can drop chunks; the periodic re-broadcast fills the gaps.
        for round in 0..12 {
            assert!(broadcaster.broadcast(&host, round * 6).expect("broadcast") > 0);
            for _ in 0..200 {
                if client.poll(&mut spectator).expect("client poll") {
                    restored = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            if restored {
                break;
            }
        }
        assert!(restored, "spectator receives a complete snapshot");
        assert!(client.connected);
        assert!(client.frame >= 0);
        assert_eq!(spectator.checksum(), checksum);
        assert!(client.status_text().contains("SPECTATING frame "));

        // The interval suppresses duplicate frames and resumes later.
        let current = client.frame;
        assert_eq!(
            broadcaster.broadcast(&host, current).expect("suppressed"),
            0
        );
        host.tick([manato_core::PlayerInput::default(); 2]);
        assert!(broadcaster.broadcast(&host, current + 6).expect("interval") > 0);
    }

    #[test]
    fn rejects_spoofed_messages_and_oversized_lengths() {
        let host_addr: SocketAddr = "127.0.0.1:9100".parse().unwrap();
        let guest_addr: SocketAddr = "127.0.0.1:9101".parse().unwrap();
        let (mut server, host) = LobbyServer::new("host", 2, host_addr);
        let guest = server.state.join("guest", false).expect("guest");
        server.peers.insert(guest, guest_addr);

        // Another peer cannot ready someone else's id or start the match.
        assert!(server
            .handle(
                guest_addr,
                LobbyMessage::Ready {
                    id: host,
                    ready: true,
                },
            )
            .is_empty());
        let stranger: SocketAddr = "127.0.0.1:9102".parse().unwrap();
        assert!(server
            .handle(stranger, LobbyMessage::Start { id: host })
            .is_empty());
        // Non-host frames are not relayed to spectators.
        assert!(server
            .handle(
                guest_addr,
                LobbyMessage::Frame {
                    frame: 1,
                    checksum: 1,
                },
            )
            .is_empty());

        // Hostile length prefixes are rejected by the decode limit.
        let mut bytes = 0u32.to_le_bytes().to_vec(); // Join variant
        bytes.extend_from_slice(&u64::MAX.to_le_bytes());
        assert!(LobbyMessage::decode(&bytes).is_none());
    }

    #[test]
    fn messages_round_trip() {
        let messages = [
            LobbyMessage::Join {
                name: "a".into(),
                spectator: true,
            },
            LobbyMessage::Ready { id: 3, ready: true },
            LobbyMessage::Frame {
                frame: 120,
                checksum: 0xdead_beef,
            },
        ];
        for message in messages {
            let bytes = message.encode();
            assert_eq!(LobbyMessage::decode(&bytes), Some(message));
        }
        assert!(LobbyMessage::decode(&[0xff, 0x00]).is_none());
    }

    #[test]
    fn server_routes_join_ready_start_and_frames() {
        let host_addr: SocketAddr = "127.0.0.1:9000".parse().unwrap();
        let guest_addr: SocketAddr = "127.0.0.1:9001".parse().unwrap();
        let spec_addr: SocketAddr = "127.0.0.1:9002".parse().unwrap();
        let (mut server, host) = LobbyServer::new("host", 2, host_addr);

        let replies = server.handle(
            guest_addr,
            LobbyMessage::Join {
                name: "guest".into(),
                spectator: false,
            },
        );
        assert!(replies
            .iter()
            .any(|(_, m)| matches!(m, LobbyMessage::Welcome { .. })));
        let guest = server.state.members[1].id;
        let replies = server.handle(
            spec_addr,
            LobbyMessage::Join {
                name: "spec".into(),
                spectator: true,
            },
        );
        let _spec = server.state.members[2].id;
        assert!(replies.iter().any(|(addr, _)| *addr == spec_addr));

        server.handle(
            guest_addr,
            LobbyMessage::Ready {
                id: guest,
                ready: true,
            },
        );
        let replies = server.handle(host_addr, LobbyMessage::Start { id: host });
        assert!(replies.iter().any(|(_, m)| matches!(
            m,
            LobbyMessage::State {
                phase: LobbyPhase::Starting,
                ..
            }
        )));

        let replies = server.handle(
            host_addr,
            LobbyMessage::Frame {
                frame: 10,
                checksum: 7,
            },
        );
        assert_eq!(replies.len(), 1, "frames go to spectators only");
        assert_eq!(replies[0].0, spec_addr);

        // Snapshot chunks relay from the host to spectators only.
        let replies = server.handle(
            host_addr,
            LobbyMessage::Snapshot {
                frame: 10,
                checksum: 7,
                index: 0,
                total: 1,
                data: vec![1, 2, 3],
            },
        );
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].0, spec_addr);
        assert!(server
            .handle(
                guest_addr,
                LobbyMessage::Snapshot {
                    frame: 11,
                    checksum: 8,
                    index: 0,
                    total: 1,
                    data: vec![9],
                },
            )
            .is_empty());
    }

    #[test]
    fn udp_relay_joins_and_relays_state() {
        let relay = UdpRelay::bind("127.0.0.1:0".parse().unwrap()).expect("relay");
        let addr = relay.local_addr().expect("addr");
        let (mut server, _host) = LobbyServer::new("host", 2, addr);
        let client = UdpSocket::bind("127.0.0.1:0").expect("client");
        client
            .set_read_timeout(Some(std::time::Duration::from_millis(200)))
            .expect("timeout");
        client
            .send_to(
                &LobbyMessage::Join {
                    name: "guest".into(),
                    spectator: false,
                }
                .encode(),
                addr,
            )
            .expect("send join");
        let mut buffer = [0u8; 4096];
        let mut decoded = None;
        for _ in 0..100 {
            relay.poll(&mut server).expect("poll");
            if let Ok((len, _)) = client.recv_from(&mut buffer) {
                decoded = LobbyMessage::decode(&buffer[..len]);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(server.state.player_count(), 2);
        assert!(matches!(
            decoded,
            Some(LobbyMessage::Welcome { .. }) | Some(LobbyMessage::State { .. })
        ));
    }

    #[test]
    fn spectator_frames_ignore_stale_and_track_lag() {
        let mut sync = SpectatorSync::default();
        assert!(sync.on_frame(10, 1));
        assert!(!sync.on_frame(10, 1));
        assert!(!sync.on_frame(9, 1));
        assert!(sync.on_frame(12, 2));
        assert_eq!(sync.missed_frames, 1);
        assert_eq!(sync.frames_received, 2);
        assert!(!sync.lagging());
        sync.on_frame(100, 3);
        assert!(sync.lagging());
    }
}
