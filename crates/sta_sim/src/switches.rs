//! Per-craft switches that missions set (SIM-58): `AllowAsTarget(object, team,
//! bool)`, `CraftCanRepair`, `ButItCanBeDerelicted`, `CraftCanRecrew`,
//! `HideObject`, `SetName` and `SetSpecialForcesFlag`. Every switch writes one
//! flag, mask bit or name on the unit; a unit with all defaults has no row.
//!
//! Rules (read from the Armada II executable, static; nothing measured):
//!
//! - **AllowAsTarget** writes one bit per team in the unit's own mask (set when
//!   disallowed). The mask is read only by the engine's target validity test and
//!   its candidate table, so it blocks *automatic* acquisition (idle, moving,
//!   guard, hunt, attack-without-target choices and assist's own best pick)
//!   and nothing else: an explicit attack order on the unit, firing at its
//!   commanded target and an assist order against an attacker are not
//!   blocked. [`Sim::auto_target_blocked`] is the one test; the weapons'
//!   target pick and the targeting candidate table call it. The bit is per
//!   seat here (the engine's team number is the seat).
//! - **CraftCanRepair** false: no shield regeneration (`combat::damage::recharge`
//!   skips the unit). The engine also stops hull regeneration and the repair
//!   system; STA64 has no hull regeneration or repair system to stop.
//! - **ButItCanBeDerelicted**, **CraftCanRecrew**: stored and in snapshots and
//!   the digest. STA64 has no derelict state and no per-craft recrew, so
//!   neither changes behaviour yet (the engine's `canRecrew` default at spawn is
//!   not established; the flag here starts unset).
//! - **HideObject** and **SetSpecialForcesFlag** are two bits of the unit
//!   (`hidden`, `special_forces`); the engine reads hidden only when drawing and
//!   special forces in the script order gate, assist and the AI planner.
//!   `Craft::SetTeam` clears special forces (an ownership change does so here).
//!   They are read with [`Sim::switches`]; the view does not carry them yet
//!   (a `UnitView` field touches every view literal of `sta64`) and the
//!   `SetSpecialForcesFlag` binding keeps its flag as mission state, so it sends
//!   no `Edit::SpecialForces` until the AI consumer (AI-02) reads it.
//! - **SetName**: a display string (limited to [`MAX_NAME_BYTES`]).

use serde::{Deserialize, Serialize};
use sta_core::{EntityId, PlayerId};

use crate::world::{Fnv, Table};
use crate::Sim;

/// Longest name a script may give a unit, in UTF-8 bytes.
pub const MAX_NAME_BYTES: usize = 64;

/// One unit's switches; the default row is never stored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Switches {
    /// Bit `p` set: seat `p` may not pick this unit automatically.
    pub not_target_of: u32,
    /// `CraftCanRepair(false)`.
    pub no_repair: bool,
    /// `ButItCanBeDerelicted(true)`.
    pub derelict_ok: bool,
    /// `CraftCanRecrew`, `None` until set.
    pub recrew: Option<bool>,
    pub hidden: bool,
    pub special_forces: bool,
    pub name: Option<String>,
}

impl Switches {
    fn is_default(&self) -> bool {
        *self == Switches::default()
    }
}

/// Every unit's switches, ascending id.
#[derive(Debug, Clone, Default)]
pub struct SwitchTable {
    rows: Table<Switches>,
}

impl SwitchTable {
    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(crate) fn remove(&mut self, id: EntityId) {
        self.rows.remove(id);
    }

    pub fn get(&self, id: EntityId) -> Switches {
        self.rows.get(id).cloned().unwrap_or_default()
    }

    pub(crate) fn edit(&mut self, id: EntityId, f: impl FnOnce(&mut Switches)) {
        let mut s = self.rows.remove(id).unwrap_or_default();
        f(&mut s);
        if !s.is_default() {
            self.rows.insert(id, s);
        }
    }

    /// Seat `by` may not pick `id` automatically.
    pub(crate) fn blocked_for(&self, id: EntityId, by: PlayerId) -> bool {
        by.0 < 32 && self.rows.get(id).is_some_and(|s| s.not_target_of & (1 << by.0) != 0)
    }

    pub(crate) fn no_repair(&self, id: EntityId) -> bool {
        self.rows.get(id).is_some_and(|s| s.no_repair)
    }

    /// The display names scripts gave (`Edit::Name`), ascending id.
    pub(crate) fn names(&self) -> Vec<(EntityId, String)> {
        self.rows.iter().filter_map(|(id, s)| s.name.clone().map(|n| (id, n))).collect()
    }

    pub(crate) fn to_rows(&self) -> Vec<(EntityId, Switches)> {
        self.rows.iter().map(|(id, s)| (id, s.clone())).collect()
    }

    pub(crate) fn from_rows(rows: Vec<(EntityId, Switches)>) -> Option<SwitchTable> {
        if rows.iter().any(|(_, s)| s.is_default() || s.name.as_ref().is_some_and(|n| n.len() > MAX_NAME_BYTES)) {
            return None;
        }
        Some(SwitchTable { rows: Table::from_rows(rows)? })
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.str("switches-v1");
        h.u64(self.rows.len() as u64);
        for (id, s) in self.rows.iter() {
            h.u64(id.0 as u64);
            h.u64(u64::from(s.not_target_of));
            h.u64(u64::from(s.no_repair) | u64::from(s.derelict_ok) << 1 | u64::from(s.hidden) << 2 | u64::from(s.special_forces) << 3);
            h.u64(match s.recrew { None => 0, Some(false) => 1, Some(true) => 2 });
            h.str(s.name.as_deref().unwrap_or(""));
        }
    }
}

impl Sim {
    /// The switches of a live unit.
    pub fn switches(&self, id: EntityId) -> Option<Switches> {
        self.units.get(id).map(|_| self.switches.get(id))
    }

    /// Apply a switch edit; `None` when `e` is not one.
    pub(crate) fn apply_switch_edit(&mut self, e: &crate::command::Edit) -> Option<bool> {
        use crate::command::Edit;
        let live = |s: &Sim, id: EntityId| {
            s.units.get(id).is_some_and(|u| u.hull[0] > sta_core::Fixed::ZERO && !u.flags.contains(crate::view::UnitFlags::DYING))
        };
        Some(match e {
            Edit::AllowAsTarget { id, player, allowed } => {
                if !live(self, *id) || !player.is_player() || player.0 >= 32 {
                    return Some(false);
                }
                self.switches.edit(*id, |s| {
                    if *allowed { s.not_target_of &= !(1 << player.0) } else { s.not_target_of |= 1 << player.0 }
                });
                true
            }
            Edit::CraftCanRepair(id, on) => self.craft_switch(*id, |s| s.no_repair = !on),
            Edit::DerelictAllowed(id, on) => self.craft_switch(*id, |s| s.derelict_ok = *on),
            Edit::CanRecrew(id, on) => self.craft_switch(*id, |s| s.recrew = Some(*on)),
            Edit::Hidden(id, on) => live(self, *id) && {
                self.switches.edit(*id, |s| s.hidden = *on);
                true
            },
            Edit::SpecialForces(id, on) => self.craft_switch(*id, |s| s.special_forces = *on),
            Edit::Name(id, name) => {
                if name.len() > MAX_NAME_BYTES || !self.units.get(*id).is_some_and(|u| u.kind == crate::world::EntityKind::Craft) || !live(self, *id) {
                    return Some(false);
                }
                self.switches.edit(*id, |s| s.name = (!name.is_empty()).then(|| name.clone()));
                true
            }
            _ => return None,
        })
    }

    /// A switch only crafts have (the engine's calls act on a `Craft`).
    fn craft_switch(&mut self, id: EntityId, f: impl FnOnce(&mut Switches)) -> bool {
        let ok = self.units.get(id).is_some_and(|u| {
            u.kind == crate::world::EntityKind::Craft && u.hull[0] > sta_core::Fixed::ZERO && !u.flags.contains(crate::view::UnitFlags::DYING)
        });
        if ok {
            self.switches.edit(id, f);
        }
        ok
    }
}
