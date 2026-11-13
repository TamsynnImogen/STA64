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
}
