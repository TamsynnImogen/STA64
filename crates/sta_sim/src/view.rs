//! The world as everything outside the sim sees it (S3).
//!
//! - An immutable snapshot built at the end of each tick; readers never
//!   borrow `Sim`. Render, UI, audio, AI, Lua and the bench read only views
//!   and events.
//! - Everything in it is `Fixed`; readers convert with `to_f32` and never
//!   feed the result back.
//! - Per-player visibility is `visible_to` (SIM-12); a view without
//!   visibility data is omniscient.
//! - Evolution: fields are added, never removed, within a milestone.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, TeamId, Tick, Vec3};

use crate::event::SimEvent;

/// The world after a tick.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WorldView {
    /// Native presentation metadata, never a substitute for gameplay state.
    #[serde(default)]
    pub appearance: crate::appearance::AppearanceView,
    pub tick: Tick,
    /// `Sim::digest` of the state this view shows.
    pub digest: u64,
    /// Ascending id.
    pub units: Vec<UnitView>,
    /// Ascending id.
    pub players: Vec<PlayerView>,
    /// Per player, the ids it sees (ascending), ascending player. Empty when
    /// the source computes no visibility, in which case the view is omniscient.
    #[serde(default)]
    pub visibility: Vec<(PlayerId, Vec<EntityId>)>,
    /// Global perceived-owner overrides, ascending entity id. Absence means
    /// real owner; this never grants command authority or sensor sharing.
    #[serde(default)]
    pub perceived_owners: Vec<(EntityId, PlayerId)>,
    /// Mission-set names and races (SIM-54); empty when nothing was set.
    #[serde(default)]
    pub attrs: crate::attrs::AttrsView,
    /// Planets with a population row (SIM-30), ascending id; the owner is
    /// the planet's [`UnitView::owner`].
    #[serde(default)]
    pub colonies: Vec<crate::colony::ColonyView>,
}

impl WorldView {
    pub fn perceived_owner(&self, id: EntityId) -> Option<PlayerId> {
        let unit = self.unit(id)?;
        Some(match self.perceived_owners.binary_search_by_key(&id, |row| row.0) {
            Ok(i) => self.perceived_owners[i].1,
            Err(_) => unit.owner,
        })
    }

    /// The units `player` sees, ascending id: its team's and whatever its
    /// team's sensors reach (`crate::sensors`). Everything when the view
    /// carries no visibility data; nothing for a player it does not know.
    pub fn visible_to(&self, player: PlayerId) -> Vec<&UnitView> {
        if self.visibility.is_empty() {
            return self.units.iter().collect();
        }
        match self.visibility.binary_search_by_key(&player, |v| v.0) {
            Ok(i) => self.visibility[i].1.iter().filter_map(|&id| self.unit(id)).collect(),
            Err(_) => Vec::new(),
        }
    }

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
    /// `[now, max]`; absent when unit crew has not been initialized.
    #[serde(default)]
    pub crew: Option<[Fixed; 2]>,
    /// `[now, max]`; absent until special energy has a live row.
    #[serde(default)]
    pub energy: Option<[Fixed; 2]>,
    /// `[movement, special weapon]` autonomy, 0 low, 1 medium, 2 high
    /// (SIM-38); only craft carry it, `None` for everything else and for
    /// sources that do not track it.
    #[serde(default)]
    pub autonomy: Option<[u8; 2]>,
}

/// Unit state bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct UnitFlags(pub u8);

impl UnitFlags {
    pub const CLOAKED: UnitFlags = UnitFlags(1);
    pub const DYING: UnitFlags = UnitFlags(1 << 1);
    pub const BUILDING: UnitFlags = UnitFlags(1 << 2);
    /// A craft is docked at this unit right now (RND-21): a freighter unloading at
    /// this station. Set on the facility, not on the craft; derived each tick like
    /// [`UnitFlags::BUILDING`], so a client that joins or loads mid-dock sees it.
    /// Repair docks at yards are not modelled yet and do not set it.
    pub const DOCKED: UnitFlags = UnitFlags(1 << 3);
    pub const SELECTABLE: UnitFlags = UnitFlags(1 << 4);
    /// Hull never falls below one (bench `Immortal`).
    pub const IMMORTAL: UnitFlags = UnitFlags(1 << 5);
    /// Fires nothing (bench `Weapons` off).
    pub const WEAPONS_OFF: UnitFlags = UnitFlags(1 << 6);
    /// Does not move (bench `Engines` off).
    pub const ENGINES_OFF: UnitFlags = UnitFlags(1 << 7);

    pub fn contains(self, other: UnitFlags) -> bool {
        self.0 & other.0 == other.0
    }

    /// These bits set or cleared.
    pub fn with(self, other: UnitFlags, on: bool) -> UnitFlags {
        UnitFlags(if on { self.0 | other.0 } else { self.0 & !other.0 })
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
    /// Current initialized crew stock; None means the source has no crew state.
    #[serde(default)]
    pub crew: Option<Fixed>,
}

/// What one step of a [`WorldSource`](crate::WorldSource) produces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TickOutput {
    pub view: Arc<WorldView>,
    pub events: Vec<SimEvent>,
}
