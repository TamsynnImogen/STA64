//! Station Recrew (SIM-70). The command, ten-second post-hit hold, and
//! `recrewRate` (default 10 crew per second) come from static Armada II
//! evidence; native in-game timing has not been measured. The `facility` and
//! `has_crew` exclusions are the resolved ODF keys used by the HUD.

use serde::{Deserialize, Serialize};
use sta_content::ContentDb;
use sta_core::{EntityId, Fixed, PlayerId, TICK_HZ};

use crate::Sim;
use crate::event::SimEvent;
use crate::view::UnitFlags;
use crate::world::{Entities, Entity, EntityKind, Fnv, Table};

const HOLD_TICKS: u64 = 10 * TICK_HZ as u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Row {
    pub(crate) owner: PlayerId,
    pub(crate) on: bool,
    pub(crate) resume_at: u64,
}

/// Active station orders and unexpired hit holds, ascending by entity id.
#[derive(Debug, Clone, Default)]
pub(crate) struct Recrews {
    rows: Table<Row>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RecrewRec {
    pub(crate) version: u8,
    pub(crate) rows: Vec<(EntityId, Row)>,
}

fn capable(u: &Entity, db: &ContentDb) -> bool {
    u.kind == EntityKind::Station
        && u.hull[0] > Fixed::ZERO
        && !u.flags.contains(UnitFlags::DYING)
        && u.crew.is_some_and(|c| c[1] > Fixed::ZERO)
        && db.unit(u.def).is_some_and(|d| {
            d.facility != Some(false) && d.has_crew != Some(false)
        })
}

/// Exact tick slice of a per-second fixed-point rate; no accumulated state.
fn tick_gain(rate: Fixed, tick: u64) -> Fixed {
    let bits = i128::from(rate.to_bits());
    let t = i128::from(tick);
    let hz = i128::from(TICK_HZ);
    Fixed::from_bits(((bits * (t + 1)).div_euclid(hz) - (bits * t).div_euclid(hz)) as i64)
}

impl Recrews {
    pub(crate) fn remove(&mut self, id: EntityId) {
        self.rows.remove(id);
    }

    /// Switch off while retaining any unexpired hit hold.
    pub(crate) fn cancel(&mut self, id: EntityId, tick: u64) {
        let Some(mut row) = self.rows.remove(id) else {
            return;
        };
        row.on = false;
        if tick < row.resume_at {
            self.rows.insert(id, row);
        }
    }

    pub(crate) fn active(&self, id: EntityId) -> bool {
        self.rows.get(id).is_some_and(|r| r.on)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.str("recrew-v1");
        h.u64(self.rows.len() as u64);
        for (id, row) in self.rows.iter() {
            h.u64(id.0 as u64);
            h.u64(u64::from(row.owner.0));
            h.u64(u64::from(row.on));
            h.u64(row.resume_at);
        }
    }

    pub(crate) fn to_rec(&self) -> RecrewRec {
        RecrewRec {
            version: 1,
            rows: self.rows.iter().map(|(id, row)| (id, *row)).collect(),
        }
    }

    /// Invalid schema, order, entity, owner, or future hold rejects the save.
    /// A station that is currently full or under construction may still have
    /// a transient row; the next step settles it.
    pub(crate) fn from_rec(rec: RecrewRec, units: &Entities, tick: u64) -> Option<Recrews> {
        if rec.version != 1
            || rec.rows.iter().any(|(id, row)| {
                units
                    .get(*id)
                    .is_none_or(|u| u.kind != EntityKind::Station || u.owner != row.owner)
                    || row.resume_at > tick.saturating_add(HOLD_TICKS)
            })
        {
            return None;
        }
        Some(Recrews {
            rows: Table::from_rows(rec.rows)?,
        })
    }

    /// Raw positive hit damage starts or extends a hold even before activation.
    pub(crate) fn note_hits(
        &mut self,
        units: &Entities,
        db: &ContentDb,
        events: &[SimEvent],
        tick: u64,
    ) {
        for event in events {
            if let SimEvent::Hit {
                target,
                shield,
                hull,
                ..
            } = event
            {
                self.note_damage(units, db, *target, *shield, *hull, tick);
            }
        }
    }

    /// Same hold for a scripted hit that has no `SimEvent::Hit` yet.
    pub(crate) fn note_damage(
        &mut self,
        units: &Entities,
        db: &ContentDb,
        id: EntityId,
        shield: Fixed,
        hull: Fixed,
        tick: u64,
    ) {
        if shield <= Fixed::ZERO && hull <= Fixed::ZERO {
            return;
        }
        let Some(u) = units.get(id).filter(|u| capable(u, db)) else {
            return;
        };
        let mut row = self
            .rows
            .get(id)
            .copied()
            .filter(|r| r.owner == u.owner)
            .unwrap_or(Row {
                owner: u.owner,
                on: false,
                resume_at: 0,
            });
        row.resume_at = tick.saturating_add(HOLD_TICKS);
        self.rows.insert(id, row);
    }
}

impl Sim {
    /// Whether the station's Recrew command is currently switched on.
    pub fn is_recrewing(&self, id: EntityId) -> bool {
        self.recrew.active(id)
    }

    /// Set or toggle Recrew for eligible stations, processing each id once.
    pub(crate) fn order_recrew(
        &mut self,
        player: PlayerId,
        ids: &[EntityId],
        on: Option<bool>,
    ) -> bool {
        let Some(db) = self.content.as_deref() else {
            return false;
        };
        let mut ids = ids.to_vec();
        ids.sort();
        ids.dedup();
        let mut any = false;
        for id in ids {
            let Some(u) = self.units.get(id).filter(|u| {
                (!player.is_player() || u.owner == player)
                    && u.owner.is_player()
                    && capable(u, db)
                    && u.progress.is_none_or(|p| p >= Fixed::ONE)
            }) else {
                continue;
            };
            let owner = u.owner;
            let crew = u.crew.expect("capable station has crew");
            let current = self
                .recrew
                .rows
                .get(id)
                .copied()
                .filter(|r| r.owner == owner);
            let desired = on.unwrap_or(!current.is_some_and(|r| r.on));
            if desired {
                if self.switches.get(id).recrew == Some(false)
                    || crew[0] >= crew[1]
                    || self.economy.crew_of(owner).is_none()
                {
                    continue;
                }
                self.recrew.rows.insert(
                    id,
                    Row {
                        owner,
                        on: true,
                        resume_at: current.map_or(0, |r| r.resume_at),
                    },
                );
            } else {
                self.recrew.cancel(id, self.tick);
            }
            any = true;
        }
        any
    }

    /// Transfer crew from each owner's initialized pool in ascending station id.
    pub(crate) fn step_recrew(&mut self) {
        let Some(db) = self.content.as_deref() else {
            return;
        };
        let rows: Vec<(EntityId, Row)> = self
            .recrew
            .rows
            .iter()
            .map(|(id, row)| (id, *row))
            .collect();
        for (id, row) in rows {
            let Some(u) = self
                .units
                .get(id)
                .filter(|u| u.owner == row.owner && capable(u, db))
            else {
                self.recrew.remove(id);
                continue;
            };
            if self.switches.get(id).recrew == Some(false) {
                self.recrew.cancel(id, self.tick);
                continue;
            }
            if !row.on {
                if self.tick >= row.resume_at {
                    self.recrew.remove(id);
                }
                continue;
            }
            if u.progress.is_some_and(|p| p < Fixed::ONE) {
                continue;
            }
            let [now, max] = u.crew.expect("capable station has crew");
            if now >= max {
                self.recrew.cancel(id, self.tick);
                continue;
            }
            if self.tick < row.resume_at
                || self
                    .economy
                    .crew_of(row.owner)
                    .is_none_or(|c| c < Fixed::ONE)
            {
                continue;
            }
            let Some(def) = db.unit(u.def) else { continue };
            let rate = def.casualty.recrew_rate.unwrap_or(Fixed::from_int(10));
            if rate <= Fixed::ZERO {
                continue;
            }
            let gain = tick_gain(rate, self.tick);
            let gap = i128::from(max.to_bits()) - i128::from(now.to_bits());
            let wanted = Fixed::from_bits(
                i128::from(gain.to_bits())
                    .min(gap)
                    .clamp(0, i128::from(i64::MAX)) as i64,
            );
            if wanted <= Fixed::ZERO {
                continue;
            }
            let taken = self.economy.withdraw_crew(row.owner, wanted);
            if taken <= Fixed::ZERO {
                continue;
            }
            if let Some(unit) = self.units.get_mut(id) {
                unit.crew = Some([
                    Fixed::from_bits(
                        (i128::from(now.to_bits()) + i128::from(taken.to_bits()))
                            .min(i128::from(max.to_bits())) as i64,
                    ),
                    max,
                ]);
                if unit.crew.is_some_and(|c| c[0] >= c[1]) {
                    self.recrew.cancel(id, self.tick);
                }
            }
        }
    }
}
