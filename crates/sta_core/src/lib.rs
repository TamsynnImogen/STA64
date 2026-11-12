//! Core types of the STA64 seams (docs/WORKPLAN.md, S1).
//!
//! Everything here is shared by the content database, the simulation, the
//! bench, net, AI and the client, so it lives below all of them and depends on
//! nothing. Evolution is additive only: moving or renaming a type is a
//! breaking change and bumps [`SEAM_REV`].
//!
//! Space is the maps' own: Direct3D left-handed, +Y up, ODF/BZN units.
//! Headings are unit vectors in the XZ plane, `(x, z)`; the sim uses no
//! trigonometry. Durations are ticks; ODF seconds convert once, at content
//! load, rounding half up ([`seconds_to_ticks`]).

pub mod fixed;
pub use fixed::Fixed;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Bumped on any breaking change to a seam type; versions the `postcard`
/// encoding of commands, views, snapshots and replays.
pub const SEAM_REV: u32 = 3;

/// Simulation ticks per second. Armada ran its sim at a fixed rate independent
/// of frame rate; we do the same and let the renderer interpolate.
pub const TICK_HZ: u32 = 30;

/// A simulation tick count, from 0 at game start.
pub type Tick = u64;

/// Ticks for a duration in seconds, rounding half up. For content loading
/// only: the float comes from a parsed ODF value and never from sim state.
pub fn seconds_to_ticks(seconds: f64) -> Tick {
    // `floor(x + 0.5)` is exact on every IEEE target, unlike platform maths.
    (seconds * TICK_HZ as f64 + 0.5).floor().max(0.0) as Tick
}

/// A position or direction in map space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Vec3 {
    pub x: Fixed,
    pub y: Fixed,
    pub z: Fixed,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: Fixed::ZERO };

    /// For loading data only (see [`Fixed::from_f64`]).
    pub fn from_f32(v: [f32; 3]) -> Vec3 {
        Vec3 {
            x: Fixed::from_f64(v[0] as f64),
            y: Fixed::from_f64(v[1] as f64),
            z: Fixed::from_f64(v[2] as f64),
        }
    }
    /// For rendering only; never feed the result back.
    pub fn to_f32(self) -> [f32; 3] {
        [self.x.to_f32(), self.y.to_f32(), self.z.to_f32()]
    }
}

/// A simulation entity. Ids are handed out in ascending order and never
/// reused, so `#3` names the same object on every run of a scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct EntityId(pub u32);

impl EntityId {
    /// No entity; never handed out.
    pub const NONE: EntityId = EntityId(0);
}

/// A player slot: 0..=7 for players, plus the two command sources that are
/// not players.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PlayerId(pub u8);

impl PlayerId {
    /// Mission scripts: may send `Edit` commands.
    pub const SCRIPT: PlayerId = PlayerId(254);
    /// The test bench: may send `Edit` commands in bench builds only.
    pub const BENCH: PlayerId = PlayerId(255);
    /// Highest player count a game can have.
    pub const MAX_PLAYERS: u8 = 8;

    /// A real player slot rather than a script or the bench.
    pub fn is_player(self) -> bool {
        self.0 < Self::MAX_PLAYERS
    }
}

/// A team (alliance) of players.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TeamId(pub u8);

/// A content definition: an index into `ContentDb`'s table of names sorted
/// bytewise, so equal on every peer that loaded the same content. Never
/// persist a key without the `ContentDb::hash()` it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ContentKey(pub u32);

impl ContentKey {
    /// No definition: things not (yet) bound to content, such as the sim's
    /// units before the entity model (SIM-01) gives them one.
    pub const NONE: ContentKey = ContentKey(u32::MAX);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_round_half_up() {
        assert_eq!(seconds_to_ticks(1.0), 30);
        assert_eq!(seconds_to_ticks(0.05), 2); // 1.5 ticks
        assert_eq!(seconds_to_ticks(0.04), 1); // 1.2 ticks
        assert_eq!(seconds_to_ticks(-1.0), 0);
    }

    #[test]
    fn script_and_bench_are_not_players() {
        assert!(PlayerId(7).is_player());
        assert!(!PlayerId::SCRIPT.is_player());
        assert!(!PlayerId::BENCH.is_player());
    }
}
