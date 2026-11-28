//! LUA-41: the mission-wide switches `AllowWarp`, `CollisionPrevention`,
//! `PathPlanning` and `ObstacleAvoidance`.
//!
//! Rules (read from the executable, static; nothing measured in a running game):
//!
//! - **AllowWarp** is one global switch for every team, not per craft or per
//!   team; it starts on. Its only reader is the craft's warp decision: a craft
//!   wants warp while its impulse speed is below its warp speed and the switch
//!   is on. The attack approach and the warp move both go through it, and the
//!   decision is taken every tick, so a craft already in warp warps out the
//!   next tick when it goes off ([`Sim::warp_allowed`]).
//! - **CollisionPrevention** is one global switch, on at the start of every sim
//!   (the original never resets it between missions; STA64 does). Off, the
//!   craft-craft separation shifts are skipped; the push out of solids stays.
//! - **PathPlanning** overrides the `RTS_CFG.h` key `USE_PATH_PLANNING`
//!   (`Some(true)` plans even when the cfg says 0).
//! - **ObstacleAvoidance** in Armada II writes a flag that nothing reads; it is
//!   stored and has no effect.
//!
//! The Armada I movement model does not read `AllowWarp`: no Armada I script calls it, so
//! its automatic warp on long plain moves is unchanged by the switch's default.
//!
//! State: one global record, saved always and hashed only when it differs from
//! the default (all on, no path override), so a run that never sends a switch
//! edit keeps every digest.

use serde::{Deserialize, Serialize};

use crate::Sim;
use crate::command::MissionSwitch;
use crate::world::Fnv;

/// The four switches. [`Default`] is the start of every sim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionSwitches {
    pub allow_warp: bool,
    pub collision_prevention: bool,
    pub obstacle_avoidance: bool,
    /// `None`: the `RTS_CFG.h` value; `Some(b)`: a script's override.
    pub path_planning: Option<bool>,
}

impl Default for MissionSwitches {
    fn default() -> Self {
        MissionSwitches { allow_warp: true, collision_prevention: true, obstacle_avoidance: true, path_planning: None }
    }
}

impl MissionSwitches {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub(crate) fn set(&mut self, which: MissionSwitch, on: bool) {
        match which {
            MissionSwitch::AllowWarp => self.allow_warp = on,
            MissionSwitch::CollisionPrevention => self.collision_prevention = on,
            MissionSwitch::ObstacleAvoidance => self.obstacle_avoidance = on,
            MissionSwitch::PathPlanning => self.path_planning = Some(on),
        }
    }

    /// Nothing is hashed at the default.
    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        if self.is_default() {
            return;
        }
        h.str("mission-switches-v1");
        h.u64(u64::from(self.allow_warp) | u64::from(self.collision_prevention) << 1 | u64::from(self.obstacle_avoidance) << 2);
        h.u64(match self.path_planning {
            None => 0,
            Some(false) => 1,
            Some(true) => 2,
        });
    }

    pub(crate) fn to_rec(self) -> SwitchesRec {
        SwitchesRec { version: 1, switches: self }
    }

    pub(crate) fn from_rec(rec: SwitchesRec) -> Result<Self, &'static str> {
        if rec.version != 1 {
            return Err("unknown mission switches record version");
        }
        Ok(rec.switches)
    }
}

/// The snapshot record (format 34).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchesRec {
    version: u8,
    switches: MissionSwitches,
}

#[cfg(test)]
impl SwitchesRec {
    pub(crate) fn for_test(version: u8, switches: MissionSwitches) -> Self {
        SwitchesRec { version, switches }
    }
}

impl Sim {
    /// `Edit::MissionSwitch`: always accepted.
    pub(crate) fn edit_mission_switch(&mut self, which: MissionSwitch, on: bool) -> bool {
        self.mission_switches.set(which, on);
        true
    }

    /// May a craft warp at all: the mission switch (`AllowWarp`), the content's warp switch
    /// (`Rules::warp_enabled`, #1102) and the match option (`GameOptions::allow_warp`) must
    /// all allow it. A script cannot turn warp back on for content that has none or in a match
    /// that has it off (the three are separate in the original: read from the executable,
    /// static). ANDed into every decision that asks for warp: the warp move (also the one a
    /// plain move of a keyed craft becomes), the attack approach, and the gravity wells that
    /// only matter to warp. A sim without content has warp.
    pub(crate) fn warp_allowed(&self) -> bool {
        self.mission_switches.allow_warp && !self.match_warp_off && self.content.as_deref().is_none_or(|db| db.rules().warp_enabled())
    }

    /// Whether the match option "warp allowed" is off for this match (always false outside the
    /// profile it counts for).
    pub fn match_warp_off(&self) -> bool {
        self.match_warp_off
    }

    /// The match option in the digest: a marker only when it is off, so a match with warp
    /// allowed keeps every digest it had before the option existed.
    pub(crate) fn hash_match_warp(&self, hash: &mut crate::world::Fnv) {
        if self.match_warp_off {
            hash.str("match-warp-off");
        }
    }

    /// The switches as the view shows them.
    pub fn mission_switches(&self) -> MissionSwitches {
        self.mission_switches
    }
}
