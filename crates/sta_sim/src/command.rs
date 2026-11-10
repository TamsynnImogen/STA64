//! Commands: the only input to the simulation (S2).
//!
//! - A tick applies its batch sorted by `(player, seq)`. Invalid commands
//!   (dead unit, wrong owner, not yet implemented) are ignored
//!   deterministically and produce [`SimEvent::Rejected`](crate::SimEvent).
//! - [`Edit`] is accepted only from [`PlayerId::BENCH`] (bench builds) and
//!   [`PlayerId::SCRIPT`] (missions). Edits are in the command log and the
//!   digest, so edited runs replay.
//! - Evolution: new variants and new fields with defaults within a
//!   milestone. The wire encoding (`postcard`) is versioned by `SEAM_REV`.

use serde::{Deserialize, Serialize};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, Tick, Vec3};

/// One player's (or script's, or the bench's) instruction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Command {
    pub player: PlayerId,
    /// Per player, increasing; orders a player's commands within a tick.
    pub seq: u32,
    pub kind: CommandKind,
}

impl Command {
    /// The order commands apply in within a tick.
    pub fn sort_key(&self) -> (PlayerId, u32) {
        (self.player, self.seq)
    }
}

/// Player orders and mission text; edits only scripts and the bench.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CommandKind {
    Order(Order),
    Edit(Edit),
    /// Text submitted by a configured, noneliminated player seat for mission
    /// hooks. Raises `SimEvent::MissionText` without changing physical state.
    /// UTF-8, at most [`MAX_MISSION_TEXT_BYTES`] bytes; empty text is allowed.
    /// Appended to preserve existing postcard command tags.
    MissionText { text: String },
}

/// Maximum UTF-8 byte length of a single mission-text command.
pub const MAX_MISSION_TEXT_BYTES: usize = 4096;

/// What a weapon, special or event aims at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Target {
    None,
    Entity(EntityId),
    Point(Vec3),
}

/// What a player's input makes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Order {
    /// `queue`: append a waypoint instead of replacing the course (Shift).
    Move { units: Vec<EntityId>, to: Vec3, queue: bool },
    Stop { units: Vec<EntityId> },
    Attack { units: Vec<EntityId>, target: EntityId },
    /// `at`: where to place a station; `None` for a ship from a yard.
    Build { builder: EntityId, what: ContentKey, at: Option<Vec3> },
    Formation { units: Vec<EntityId>, kind: u8 },
    Special { units: Vec<EntityId>, slot: u8, target: Target },
    Cloak { units: Vec<EntityId>, on: bool },
    Mine { units: Vec<EntityId>, node: EntityId },
    /// Set a builder's destination for newly produced mobile units. `None`
    /// clears it; entity positions are resolved when a unit is produced.
    /// Appended: existing postcard Order tags must keep their values.
    Rally { builders: Vec<EntityId>, at: Target },
}

/// Direct changes to the world: the bench's edits (docs/TESTBENCH.md,
/// "Commands") and missions' world changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Edit {
    Spawn { odf: ContentKey, player: PlayerId, pos: Vec3, heading: (Fixed, Fixed) },
    Kill(EntityId),
    Damage(EntityId, Fixed),
    Heal(EntityId),
    Immortal(EntityId, bool),
    SetEnergy(EntityId, Fixed),
    /// Grant a technology to a player.
    Tech(PlayerId, ContentKey),
    /// Set a player's stock of a resource (by resource id).
    Cargo(PlayerId, u8, Fixed),
    /// Engines on or off.
    Engines(EntityId, bool),
    /// Weapons on or off.
    Weapons(EntityId, bool),
    /// Change an entity's owner.
    Team(EntityId, PlayerId),
    /// Reseed the sim's random sequence.
    Seed(u64),
    /// Something to show or play that changes nothing in the world: the sim
    /// raises [`SimEvent::Present`](crate::event::SimEvent::Present) with it
    /// that tick (see [`crate::present`]).
    Present(crate::present::Presentation),
    /// A subsystem on or off (mission `Disable*` calls). `Engines` and
    /// `Weapons` act like [`Edit::Engines`] / [`Edit::Weapons`]; the others
    /// are rejected until the package owning that subsystem applies them.
    Subsystem(EntityId, Subsystem, bool),
    /// Alert state (mission `SetRedAlert` and friends). Rejected until a
    /// package models alert states.
    Alert(EntityId, AlertState),
    /// Cap a unit's speed (`SetImpulseSpeed`), `None` to lift the cap.
    /// Rejected until movement applies it.
    MaxSpeed(EntityId, Option<Fixed>),
    /// Whether other units may target it (`AllowAsTarget`). Rejected until
    /// targeting applies it.
    Targetable(EntityId, bool),
    /// Whether its owner may decommission it (`SetCanBeDecommissioned`).
    /// Rejected until decommissioning exists.
    Decommissionable(EntityId, bool),
    /// A mission spawn with an opaque caller-owned correlation token. Only
    /// SCRIPT may issue this edit. Success emits ordinary `Spawned` and
    /// `ScriptSpawned { request, id }`; failure uses `Rejected { player, seq }`.
    /// Consumers allocate unique tokens and correlate their command sequence;
    /// the sim does not reserve IDs or deduplicate request tokens.
    /// Appended to preserve every existing postcard Edit tag.
    SpawnRequest { request: u64, odf: ContentKey, player: PlayerId, pos: Vec3, heading: (Fixed, Fixed) },
}

/// A unit system a mission can switch on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Subsystem {
    Engines,
    Weapons,
    Shields,
    LifeSupport,
    Sensors,
}

/// Armada's alert states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AlertState {
    Green,
    Yellow,
    Red,
}

/// Where commands wait between being issued and being applied. Local play
/// uses [`LocalRouter`]; lockstep (`sta_net`) holds a turn until every peer's
/// commands for it have arrived.
pub trait CommandRouter: Send {
    fn submit(&mut self, cmd: Command);
    /// The commands to apply on `tick`, sorted by `(player, seq)`; `None`
    /// when they are not all here yet and the sim must stall.
    fn take_ready(&mut self, tick: Tick) -> Option<Vec<Command>>;
}

/// Single player: everything submitted lands on the next tick; never stalls.
#[derive(Debug, Clone, Default)]
pub struct LocalRouter {
    pending: Vec<Command>,
}

impl LocalRouter {
    pub fn new() -> LocalRouter {
        LocalRouter::default()
    }
}

impl CommandRouter for LocalRouter {
    fn submit(&mut self, cmd: Command) {
        self.pending.push(cmd);
    }

    fn take_ready(&mut self, _tick: Tick) -> Option<Vec<Command>> {
        let mut batch = std::mem::take(&mut self.pending);
        // Stable: equal keys (a bug in the sender) keep submission order.
        batch.sort_by_key(Command::sort_key);
        Some(batch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stop(player: u8, seq: u32) -> Command {
        Command { player: PlayerId(player), seq, kind: CommandKind::Order(Order::Stop { units: vec![EntityId(1)] }) }
    }

    #[test]
    fn local_router_delivers_next_tick_in_player_seq_order() {
        let mut r = LocalRouter::new();
        assert_eq!(r.take_ready(0), Some(vec![]));
        r.submit(stop(1, 0));
        r.submit(stop(0, 2));
        r.submit(stop(255, 0));
        r.submit(stop(0, 1));
        r.submit(stop(254, 7));
        let got: Vec<_> = r.take_ready(1).unwrap().iter().map(Command::sort_key).collect();
        let want = [(0, 1), (0, 2), (1, 0), (254, 7), (255, 0)].map(|(p, s)| (PlayerId(p), s));
        assert_eq!(got, want);
        // Delivered once: the next tick is empty.
        assert_eq!(r.take_ready(2), Some(vec![]));
    }

    #[test]
    fn commands_round_trip_through_postcard() {
        let c = Command {
            player: PlayerId(3),
            seq: 9,
            kind: CommandKind::Order(Order::Move { units: vec![EntityId(4), EntityId(5)], to: Vec3::from_f32([1.5, 0.0, -2.0]), queue: true }),
        };
        let bytes = postcard::to_allocvec(&c).unwrap();
        assert_eq!(postcard::from_bytes::<Command>(&bytes).unwrap(), c);
    }

    #[test]
    fn rally_appends_a_wire_tag_and_round_trips_all_target_kinds() {
        let mine = Order::Mine { units: vec![], node: EntityId(1) };
        assert_eq!(postcard::to_allocvec(&mine).unwrap(), [7, 0, 1]);
        let clear = Order::Rally { builders: vec![], at: Target::None };
        assert_eq!(postcard::to_allocvec(&clear).unwrap(), [8, 0, 0]);
        for at in [Target::None, Target::Entity(EntityId(42)), Target::Point(Vec3::from_f32([1.5, -2.0, 3.0]))] {
            let c = Command { player: PlayerId(0), seq: 9, kind: CommandKind::Order(Order::Rally { builders: vec![EntityId(3)], at }) };
            let bytes = postcard::to_allocvec(&c).unwrap();
            assert_eq!(postcard::from_bytes::<Command>(&bytes).unwrap(), c);
        }
    }
}
