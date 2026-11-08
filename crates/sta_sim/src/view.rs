//! The world as everything outside the sim sees it (S3).
//!
//! - An immutable snapshot built at the end of each tick; readers never
//!   borrow `Sim`. Render, UI, audio, AI, Lua and the bench read only views
//!   and events.
//! - Everything in it is `Fixed`; readers convert with `to_f32` and never
//!   feed the result back.
//! - Omniscient until per-player visibility (`visible_to`, SIM-12).
//! - Evolution: fields are added, never removed, within a milestone.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, TeamId, Tick, Vec3};

use crate::event::SimEvent;

/// The world after a tick.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WorldView {
    pub tick: Tick,
    /// `Sim::digest` of the state this view shows.
    pub digest: u64,
    /// Ascending id.
    pub units: Vec<UnitView>,
    /// Ascending id.
    pub players: Vec<PlayerView>,
}

impl WorldView {
    pub fn unit(&self, id: EntityId) -> Option<&UnitView> {
        self.units.binary_search_by_key(&id, |u| u.id).ok().map(|i| &self.units[i])
    }

    pub fn player(&self, id: PlayerId) -> Option<&PlayerView> {
        self.players.binary_search_by_key(&id, |p| p.id).ok().map(|i| &self.players[i])
    }
}

/// One entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitView {
    pub id: EntityId,
    /// [`ContentKey::NONE`] for units not made from content (today's sim).
    pub def: ContentKey,
    pub owner: PlayerId,
    pub pos: Vec3,
    /// Unit vector in XZ, `(x, z)`.
    pub heading: (Fixed, Fixed),
    /// Units per second.
    pub speed: Fixed,
    /// `[now, max]`.
    pub hull: [Fixed; 2],
    /// `[now, max]` per facing: fore, aft, port, starboard. Without
    /// directional shields all four show the one pool.
    pub shields: [[Fixed; 2]; 4],
    pub flags: UnitFlags,
    /// Building or under construction, 0..1.
    pub progress: Option<Fixed>,
}

/// Unit state bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct UnitFlags(pub u8);

impl UnitFlags {
    pub const CLOAKED: UnitFlags = UnitFlags(1);
    pub const DYING: UnitFlags = UnitFlags(1 << 1);
    pub const BUILDING: UnitFlags = UnitFlags(1 << 2);
    pub const DOCKED: UnitFlags = UnitFlags(1 << 3);
    pub const SELECTABLE: UnitFlags = UnitFlags(1 << 4);

    pub fn contains(self, other: UnitFlags) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for UnitFlags {
    type Output = UnitFlags;
    fn bitor(self, o: UnitFlags) -> UnitFlags {
        UnitFlags(self.0 | o.0)
    }
}

/// How one player stands toward another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Relation {
    Ally,
    Neutral,
    Enemy,
}

/// One player.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerView {
    pub id: PlayerId,
    pub team: TeamId,
    /// Stock per resource id (0 dilithium, 1 latinum, 2 metal, 3 biomatter).
    pub resources: Vec<Fixed>,
    /// `[in use, capacity]`.
    pub officers: [Fixed; 2],
    pub eliminated: bool,
    /// Toward every other player, ascending id.
    pub relations: Vec<(PlayerId, Relation)>,
}

/// What one step of a [`WorldSource`](crate::WorldSource) produces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TickOutput {
    pub view: Arc<WorldView>,
    pub events: Vec<SimEvent>,
}
