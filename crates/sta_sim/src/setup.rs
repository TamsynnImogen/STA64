//! What a game starts from, and who plays it (S7).
//!
//! - The UI (Instant Action, lobby) produces a [`GameSetup`]; the sim, net
//!   and bench consume it. A replay is a `GameSetup`, the command log and
//!   digests.
//! - [`Controller`]s (AI, missions) run on one host and send their commands
//!   through the `CommandRouter` like a player, so they need not be
//!   bit-deterministic and replays never rerun them.

use std::sync::Arc;
use serde::{Serialize, Deserialize};

use sta_content::{ContentDb, MapDef};
use sta_core::{PlayerId, TeamId};

use crate::command::Command;
use crate::event::SimEvent;
use crate::view::WorldView;
use crate::Sim;

/// Everything needed to start the same game on every peer.
#[derive(Debug, Clone)]
pub struct GameSetup {
    pub seed: u64,
    pub map: Arc<MapDef>,
    pub content: Arc<ContentDb>,
    /// Ascending id.
    pub players: Vec<PlayerSetup>,
    pub options: GameOptions,
}

/// One seat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerSetup {
    pub id: PlayerId,
    pub team: TeamId,
    pub race: String,
    pub control: Control,
}

/// Who sends a player's commands.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Control {
    /// Input on this machine.
    Human,
    /// Input on another peer.
    Remote,
    /// A computer player, by registry name (`sta_ai`), run on this machine.
    Ai(String),
    /// A computer player run by the host: its commands arrive like remote
    /// input, and the name is the one the host runs. Every peer of a session
    /// holds the same set of AI seats (the tech gate and passive economy read
    /// it inside `step`), so a non-host peer builds its sim with this, not
    /// with [`Control::Remote`], which would leave the seat out of
    /// `Sim::ai_seats` and desync the peers.
    RemoteAi(String),
    /// Nobody: the seat exists but never sends a command.
    Idle,
}

/// Native starting choices, shared by all peers and persisted in replays.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct GameOptions {
    pub starting_resources: sta_content::StartingResources,
    /// Authored MP_OFFICER_LIMIT row; zero selects its first/default entry.
    pub officer_limit: u32,
}

/// A computer player or a mission: reads views and events, writes commands.
pub trait Controller: Send {
    fn think(&mut self, view: &WorldView, events: &[SimEvent], out: &mut Vec<Command>);
}

impl Sim {
    /// A sim at tick 0 for `s`: seeded, with the players and the map's
    /// objects (`crate::spawn`). Objects it cannot resolve are skipped;
    /// [`Sim::from_setup_with_warnings`] lists them.
    pub fn from_setup(s: &GameSetup) -> Sim {
        Sim::from_setup_with_warnings(s).0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorldSource;
    use crate::view::Relation;

    #[test]
    fn from_setup_records_players() {
        let setup = GameSetup {
            seed: 7,
            map: Arc::new(sta_content::fake::two_bases()),
            content: Arc::new(sta_content::fake::db()),
            players: vec![
                PlayerSetup { id: PlayerId(1), team: TeamId(1), race: "fx".into(), control: Control::Idle },
                PlayerSetup { id: PlayerId(0), team: TeamId(0), race: "fx".into(), control: Control::Human },
            ],
            options: GameOptions::default(),
        };
        let sim = Sim::from_setup(&setup);
        let view = sim.view();
        assert_eq!(view.tick, 0);
        let ids: Vec<_> = view.players.iter().map(|p| p.id).collect();
        assert_eq!(ids, [PlayerId(0), PlayerId(1)]);
        assert_eq!(view.players[0].relations, [(PlayerId(1), Relation::Enemy)]);
        assert_eq!(Sim::from_setup(&setup).digest(), sim.digest());
    }
    fn setup_with(control: Control) -> GameSetup {
        GameSetup {
            seed: 7,
            map: Arc::new(sta_content::fake::two_bases()),
            content: Arc::new(sta_content::fake::db()),
            players: vec![
                PlayerSetup { id: PlayerId(0), team: TeamId(0), race: "fx".into(), control: Control::Human },
                PlayerSetup { id: PlayerId(1), team: TeamId(1), race: "fx".into(), control },
            ],
            options: GameOptions::default(),
        }
    }

    /// Which machine runs the AI is not state: the host's `Ai` seat and a
    /// joiner's `RemoteAi` seat give identical sims, digests and snapshots,
    /// and a replay keeps the difference away.
    #[test]
    fn ai_seats_are_session_state_whoever_runs_the_ai() {
        let host = Sim::from_setup(&setup_with(Control::Ai("skirmish".into())));
        let joiner = Sim::from_setup(&setup_with(Control::RemoteAi("skirmish".into())));
        assert_eq!(host.digest(), joiner.digest());
        assert_eq!(host.section_digests(), joiner.section_digests());
        assert!(host.snapshot() == joiner.snapshot());
        let roundtrip = crate::replay::Replay::new(&setup_with(Control::RemoteAi("skirmish".into())))
            .setup(Arc::new(sta_content::fake::db()))
            .unwrap();
        assert_eq!(Sim::from_setup(&roundtrip).digest(), host.digest());
        // A seat that is not an AI seat is a different session.
        let remote = Sim::from_setup(&setup_with(Control::Remote));
        assert_ne!(remote.digest(), host.digest());
        assert_eq!(remote.section_digests().differing(&host.section_digests()), ["roster"]);
        assert!(remote.snapshot() != host.snapshot());
    }
}
