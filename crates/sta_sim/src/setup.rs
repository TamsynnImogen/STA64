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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct GameOptions {
    pub starting_resources: sta_content::StartingResources,
    /// Authored MP_OFFICER_LIMIT row; zero selects its first/default entry.
    pub officer_limit: u32,
    /// Armada I's dogfighting mode, agreed at match start.
    #[serde(default)]
    pub directors_cut: bool,
    /// Armada II's match option "warp allowed" (#1102), agreed at match start. Off, no craft
    /// warps in this match, whatever a mission script switches: the option, the mission switch
    /// and the content's warp rule must all allow warp (`Sim::warp_allowed`; that the three are
    /// separate in the original and combined so: read from the executable, static). It counts
    /// under the stock Armada II profile only and is ignored under every other one, as
    /// [`GameOptions::directors_cut`] is outside its game.
    ///
    /// On by default. INTERIM, STA64's own: the original's default was not read.
    #[serde(default = "on")]
    pub allow_warp: bool,
}

fn on() -> bool {
    true
}

impl Default for GameOptions {
    fn default() -> Self {
        GameOptions { starting_resources: Default::default(), officer_limit: 0, directors_cut: false, allow_warp: true }
    }
}

impl GameOptions {
    /// Whether a match on `content` has the option "warp allowed" to choose: the stock Armada II
    /// profile, and content that has warp at all (`Rules::warp_enabled`; where the content
    /// switches warp off the option could not bring it back, so it is not offered). The set-up
    /// screens show the control when this is true.
    ///
    /// STA64's own: it is offered for a single-player skirmish as for a network match (whether
    /// the original offers it outside a network match is not established).
    pub fn warp_offered(content: &sta_content::ContentDb) -> bool {
        Self::warp_option_counts(content) && content.rules().warp_enabled()
    }

    /// Whether [`GameOptions::allow_warp`] counts for a match on `content`: the one profile it
    /// belongs to.
    pub(crate) fn warp_option_counts(content: &sta_content::ContentDb) -> bool {
        content.profile() == sta_content::Profile::Sta2Classic
    }
}

/// A computer player or a mission: reads views and events, writes commands.
pub trait Controller: Send {
    fn think(&mut self, view: &WorldView, events: &[SimEvent], out: &mut Vec<Command>);

    /// Hands out the controller's changing state as one versioned, opaque
    /// record for a save file (ADR 0010 V5), or `None`: this controller
    /// cannot be saved, and a save of a match it plays is refused with its
    /// seat named instead of losing its state. The default is `None`.
    ///
    /// A controller that opts in must be deterministic given this record:
    /// restored from it and fed the same views and events, it sends the same
    /// commands as the uninterrupted controller would have (V5: a loaded
    /// match continues as an uninterrupted one would). The record holds
    /// state only, never configuration that is rebuilt from the setup and
    /// content. Restoring is the owning crate's job: it builds the
    /// controller and calls [`Controller::restore_state`].
    fn save_state(&self) -> Option<ControllerState> {
        None
    }

    /// Takes back a record made by [`Controller::save_state`]. Refuses (with
    /// a message naming the reason) a record version it does not read. The
    /// default refuses everything: a controller that does not save does not
    /// restore.
    fn restore_state(&mut self, _state: &ControllerState) -> Result<(), String> {
        Err("this controller does not restore saved state".to_string())
    }
}

/// A controller's state as a save file keeps it: a version of the
/// controller's own record layout and opaque bytes only that controller
/// reads.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ControllerState {
    pub version: u32,
    pub data: Vec<u8>,
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
