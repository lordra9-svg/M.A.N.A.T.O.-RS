//! GGRS rollback netcode integration.
//!
//! The simulation is already deterministic and cheap to clone, so a snapshot is
//! just a [`SimState`]. Assets live behind an `Arc` and are never cloned.

use std::net::SocketAddr;

use ggrs::{
    Config, GgrsEvent, GgrsRequest, InputStatus, P2PSession, PlayerType, SessionBuilder,
    SyncTestSession,
};
use manato_core::PlayerInput;
use manato_sim::{ai, SimState};

/// GGRS configuration for a two-player M.A.N.A.T.O.-RS match.
pub struct ManatoConfig;

impl Config for ManatoConfig {
    type Input = PlayerInput;
    type InputPredictor = ggrs::PredictRepeatLast;
    type State = SimState;
    type Address = SocketAddr;
}

fn apply_requests(sim: &mut SimState, requests: Vec<GgrsRequest<ManatoConfig>>) {
    for request in requests {
        match request {
            GgrsRequest::SaveGameState { cell, frame } => {
                let checksum = sim.checksum() as u128;
                cell.save(frame, Some(sim.clone()), Some(checksum));
            }
            GgrsRequest::LoadGameState { cell, frame } => match cell.load() {
                Some(state) => {
                    log::debug!("rollback to frame {frame}");
                    *sim = state;
                }
                None => log::error!("missing savestate for frame {frame}"),
            },
            GgrsRequest::AdvanceFrame { inputs } => {
                let input = inputs_for(&inputs);
                sim.tick(input);
            }
        }
    }
}

fn inputs_for(inputs: &[(PlayerInput, InputStatus)]) -> [PlayerInput; 2] {
    let mut out = [PlayerInput::default(); 2];
    for (i, (input, _status)) in inputs.iter().take(2).enumerate() {
        out[i] = *input;
    }
    out
}

/// Local determinism harness: runs the real game with GGRS's save/load/rollback
/// cycle and compares checksums.
pub struct SyncTest {
    session: SyncTestSession<ManatoConfig>,
    sim: SimState,
    ai_level: i32,
}

impl SyncTest {
    pub fn new(sim: SimState, ai_level: i32, check_distance: usize) -> anyhow::Result<Self> {
        let session = SessionBuilder::<ManatoConfig>::new()
            .with_num_players(2)?
            .with_check_distance(check_distance)
            .with_input_delay(0)
            .start_synctest_session()?;
        Ok(Self {
            session,
            sim,
            ai_level,
        })
    }

    pub fn sim(&self) -> &SimState {
        &self.sim
    }

    pub fn sim_mut(&mut self) -> &mut SimState {
        &mut self.sim
    }

    pub fn frame(&self) -> i32 {
        self.session.current_frame()
    }

    /// Adds inputs for both slots and runs exactly one rollback-tested frame.
    pub fn step(&mut self, local: PlayerInput) -> anyhow::Result<()> {
        let remote = ai::input_for(&self.sim, 1, self.ai_level);
        self.session.add_local_input(0, local)?;
        self.session.add_local_input(1, remote)?;
        let requests = self.session.advance_frame()?;
        apply_requests(&mut self.sim, requests);
        Ok(())
    }
}

/// Thin wrapper around a two-player peer-to-peer rollback session.
pub struct Netplay {
    session: P2PSession<ManatoConfig>,
    sim: SimState,
    local_handle: usize,
}

impl Netplay {
    pub fn new(
        sim: SimState,
        local_handle: usize,
        socket: ggrs::UdpNonBlockingSocket,
        remote: SocketAddr,
    ) -> anyhow::Result<Self> {
        let mut builder = SessionBuilder::<ManatoConfig>::new()
            .with_num_players(2)?
            .with_input_delay(2);
        for handle in 0..2 {
            let player = if handle == local_handle {
                PlayerType::Local
            } else {
                PlayerType::Remote(remote)
            };
            builder = builder.add_player(player, handle)?;
        }
        let session = builder.start_p2p_session(socket)?;
        Ok(Self {
            session,
            sim,
            local_handle,
        })
    }

    pub fn session(&self) -> &P2PSession<ManatoConfig> {
        &self.session
    }

    pub fn sim(&self) -> &SimState {
        &self.sim
    }

    pub fn sim_mut(&mut self) -> &mut SimState {
        &mut self.sim
    }

    pub fn poll(&mut self) -> Vec<GgrsEvent<ManatoConfig>> {
        self.session.poll_remote_clients();
        self.session.events().collect()
    }

    /// Adds the local input and advances. Returns false when GGRS asked us to
    /// skip this frame (prediction threshold).
    pub fn step(&mut self, local: PlayerInput) -> anyhow::Result<bool> {
        self.session.add_local_input(self.local_handle, local)?;
        match self.session.advance_frame() {
            Ok(requests) => {
                apply_requests(&mut self.sim, requests);
                Ok(true)
            }
            Err(ggrs::GgrsError::PredictionThreshold) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
}
