//! Transport-agnostic lobby and spectator session state (N-03).
//!
//! The lobby protocol carries join/leave/ready/start/frame messages over any
//! transport; the UDP relay wiring is host-provided.

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
        self.members.push(LobbyMember {
            id,
            name: name.to_string(),
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
    Join { name: String, spectator: bool },
    Leave { id: u32 },
    Ready { id: u32, ready: bool },
    Start { id: u32 },
    Frame { frame: i32, checksum: u64 },
}

impl LobbyMessage {
    pub fn encode(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap_or_default()
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        bincode::deserialize(bytes).ok()
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
