//! What happened during a tick (S4).
//!
//! - Emitted in a deterministic order, identical on every peer, so marks and
//!   mission triggers can key on them. Not hashed separately: they follow
//!   from state.
//! - Client-side events the sim never sees (`OrderIssued`,
//!   `SelectionChanged`) are Bevy messages in `sta64::sim_link`.
//! - Evolution: new variants freely; consumers must ignore unknown ones.

use serde::{Deserialize, Serialize};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, Vec3};

pub use crate::command::Target;

/// Something that happened in a tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SimEvent {
    Spawned { id: EntityId },
    Destroyed { id: EntityId, by: Option<EntityId> },
    Fired { shooter: EntityId, weapon: ContentKey, hardpoint: u8, target: Target },
    /// `facing`: 0 fore, 1 aft, 2 port, 3 starboard. `shield`/`hull`: damage
    /// taken by each.
    Hit { target: EntityId, by: EntityId, weapon: ContentKey, facing: u8, shield: Fixed, hull: Fixed, at: Vec3 },
    ShieldsDown { id: EntityId, facing: u8 },
    BuildStarted { builder: EntityId, what: ContentKey },
    Built { builder: EntityId, id: EntityId },
    Delivered { player: PlayerId, resource: u8, amount: Fixed },
    CloakChanged { id: EntityId, on: bool },
    /// A command that was ignored, by its `(player, seq)`.
    Rejected { player: PlayerId, seq: u32 },
    PlayerEliminated { player: PlayerId },
    /// A mission's presentation, passed through from
    /// [`Edit::Present`](crate::command::Edit::Present) the tick it applied.
    Present(crate::present::Presentation),
    /// The actual ID allocated by a successful SCRIPT `Edit::SpawnRequest`.
    /// This accompanies ordinary `Spawned`, not a liveness guarantee: a
    /// later command or combat can destroy the entity during the same tick.
    /// Consumers must match an outstanding unique request and check the view.
    /// Appended to preserve every existing postcard event tag.
    ScriptSpawned { request: u64, id: EntityId },
    /// Accepted player mission text, in command application order. The
    /// originating `(player, seq)` identifies it within this tick; it is not
    /// a chat-display instruction or a keyboard event. See `MissionText` in
    /// [`crate::command::CommandKind`]. Appended after spawn receipts.
    MissionText { player: PlayerId, seq: u32, text: String },
    /// A point-defence weapon switched on or off (SIM-37), by an order or by
    /// running out of special energy. Only with `Features::pd_energy`.
    /// Appended to preserve every existing postcard event tag.
    PointDefenceChanged { id: EntityId, on: bool },
    /// A freighter changed what it is doing in its mining cycle (RND-29): the renderer
    /// draws the beam while `Mining` and the berth while `Unloading` from it (the sim itself
    /// circles the node). Not hashed with the state; a miner is announced when its phase
    /// changes and when a stop, move or new order takes it off its work (a phase a loaded save
    /// resumes in is not announced until the next change).
    /// Appended to preserve every existing postcard event tag.
    /// `dock`: the station it unloads at, when `phase` is `Unloading`.
    MinerPhase { id: EntityId, phase: MinerPhase, node: EntityId, dock: Option<Vec3> },
    /// A craft's move order cannot be served: it is held walled in (SIM-22,
    /// order kept for now) or the stuck-order watch ended it after two minutes
    /// without getting nearer. Raised once per order, the tick the hold
    /// starts or, failing that, when the watch ends it. Not hashed.
    /// Appended to preserve every existing postcard event tag.
    OrderBlocked { id: EntityId },
    /// The sim ended a craft's move order itself (the stuck-order watch): its
    /// target and queued legs are cleared. A controller that remembers the
    /// order should forget it; `OrderBlocked` alone does not mean that (the
    /// SIM-22 hold keeps the order). Not hashed. Appended to preserve every
    /// existing postcard event tag.
    OrderAbandoned { id: EntityId },
}

/// Where a freighter is in `ToNode -> Mining -> ToDock -> Unloading` ([`SimEvent::MinerPhase`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MinerPhase {
    /// On its way to the resource.
    ToNode,
    /// At the resource, filling its hold.
    Mining,
    /// Carrying the load to the station.
    ToDock,
    /// At the station, unloading.
    Unloading,
    /// Mining ended (a new order, or nothing left to mine).
    Idle,
}
