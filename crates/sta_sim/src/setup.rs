//! What a game starts from, and who plays it (S7).
//!
//! - The UI (Instant Action, lobby) produces a [`GameSetup`]; the sim, net
//!   and bench consume it. A replay is a `GameSetup`, the command log and
//!   digests.
//! - [`Controller`]s (AI, missions) run on one host and send their commands
//!   through the `CommandRouter` like a player, so they need not be
//!   bit-deterministic and replays never rerun them.

use std::sync::Arc;

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
    /// A computer player, by registry name (`sta_ai`).
    Ai(String),
    /// Nobody: the seat exists but never sends a command.
    Idle,
}

/// Game rules chosen in setup. Empty until the packages that need options
/// (victory conditions, starting resources, ...) add them, with defaults.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct GameOptions {}

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
}
