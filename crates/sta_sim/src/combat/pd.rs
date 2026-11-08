//! Point-defence cycles (SIM-06): PD hardpoints shoot down homing shots.
//!
//! Active only when `Features::point_defense_cycles` is on; otherwise PD
//! weapons stay idle, as before.
//!
//! - Each PD hardpoint runs one cycle when its weapon is ready: it removes
//!   the nearest in-flight shot that is homing on the hardpoint's own unit,
//!   comes from a hostile team (or an unknown shooter) and is inside the
//!   weapon's range. One shot per cycle; the mount is then busy for the
//!   weapon's `refire` ticks (at least one). A cycle with nothing to shoot
//!   costs nothing: the mount stays ready.
//! - A salvo bigger than the cycles that fit in the shots' time inside the
//!   range gets through: the rest hit as usual.
//! - Runs at the start of the weapons step, before shots move, so a shot is
//!   judged at the position it had at the end of the last tick. Units go in
//!   ascending id, hardpoints in definition order.
//! - Each cycle emits `Fired` (weapon = the PD weapon, target = the point
//!   the shot was at). There is no intercept event in S4 (see PR).
//! - The PD weapon's `damage` is unused: an intercept always kills.

use sta_content::{ContentDb, WeaponKind};
use sta_core::{EntityId, Fixed, PlayerId, TeamId};

use super::weapons::{Shot, flat};
use crate::event::{SimEvent, Target};
use crate::world::{Entities, Fnv, Table};

/// Per-unit, per-hardpoint first tick the PD weapon may cycle again.
#[derive(Debug, Clone, Default)]
pub(crate) struct Pd {
    ready: Table<Vec<u64>>,
}

/// [`Pd`] as the snapshot stores it (rows ascending by unit id).
pub(crate) type PdRec = Vec<(EntityId, Vec<u64>)>;

impl Pd {
    pub(crate) fn to_rec(&self) -> PdRec {
        self.ready.iter().map(|(id, r)| (id, r.clone())).collect()
    }

    /// `None` if the rows are not ascending by id.
    pub(crate) fn from_rec(r: PdRec) -> Option<Pd> {
        Some(Pd {
            ready: Table::from_rows(r)?,
        })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.ready.is_empty()
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.u64(self.ready.len() as u64);
        for (id, rs) in self.ready.iter() {
            h.u64(id.0 as u64);
            for &r in rs {
                h.u64(r);
            }
        }
    }

    /// One tick of point defence: remove the shots the ready PD mounts take.
    pub(crate) fn step(
        &mut self,
        tick: u64,
        units: &Entities,
        db: &ContentDb,
        teams: &[(PlayerId, TeamId)],
        shots: &mut Vec<Shot>,
        events: &mut Vec<SimEvent>,
    ) {
        if !db.features().point_defense_cycles {
            return;
        }
        let gone: Vec<EntityId> = self
            .ready
            .iter()
            .filter(|(id, _)| units.get(*id).is_none_or(|u| u.hull[1] <= Fixed::ZERO))
            .map(|(id, _)| id)
            .collect();
        for id in gone {
            self.ready.remove(id);
        }
        let team = |p: PlayerId| teams.iter().find(|x| x.0 == p).map(|x| x.1);
        for u in units.iter() {
            if u.hull[1] <= Fixed::ZERO || shots.is_empty() {
                continue;
            }
            let Some(def) = db.unit(u.def) else { continue };
            for (i, hp) in def.hardpoints.iter().enumerate() {
                let Some(w) = db.weapon(hp.weapon.key) else {
                    continue;
                };
                if w.kind != WeaponKind::PointDefence {
                    continue;
                }
                if self.ready.get(u.id).is_some_and(|r| r[i] > tick) {
                    continue;
                }
                // Nearest hostile shot homing on this unit, inside range;
                // the list is ascending by shot id, so ties go to the lowest.
                let mut best: Option<(Fixed, usize)> = None;
                for (k, s) in shots.iter().enumerate() {
                    if s.target != u.id {
                        continue;
                    }
                    let friendly = units
                        .get(s.shooter)
                        .and_then(|sh| team(sh.owner))
                        .is_some_and(|t| Some(t) == team(u.owner));
                    let d = flat(u.pos, s.pos).1;
                    if friendly || d > w.range || best.is_some_and(|(bd, _)| d >= bd) {
                        continue;
                    }
                    best = Some((d, k));
                }
                let Some((_, k)) = best else { continue };
                let s = shots.remove(k);
                if self.ready.get(u.id).is_none() {
                    self.ready.insert(u.id, vec![0; def.hardpoints.len()]);
                }
                if let Some(r) = self.ready.get_mut(u.id) {
                    r[i] = tick + w.refire.max(1);
                }
                events.push(SimEvent::Fired {
                    shooter: u.id,
                    weapon: w.key,
                    hardpoint: i as u8,
                    target: Target::Point(s.pos),
                });
            }
        }
    }
}
