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
//!
//! Energy model (SIM-34, `Features::pd_energy`; implements D5, D9). The PD
//! weapon is a toggled special (`Order::Special` on its slot). It starts off.
//! While on it drains `specialEnergyCost` per second from the unit's special
//! energy pool, with nothing to shoot as well (measured on stock A2 and
//! Roots), and switches itself off when the pool reaches 0. Recharge keeps
//! running alongside (`specials.rs`).
//! - Stock rate model: every eligible shot in range is taken, `shotDelay` is
//!   ignored (measured).
//! - `Features::pd_debt_timer` (Roots; measured, creation of the rule also read
//!   from the engine's code): one timer per weapon instance, in ticks. It falls
//!   by one per tick while the weapon is on, is never clamped at 0, is not
//!   reset by toggling, and the weapon fires while it is <= 0; each shot adds
//!   `shotDelay` x (1 + U[0, 0.05]) from the sim's RNG. Switching on needs
//!   energy >= `specialEnergyCost` (read from the engine's code only). Stock
//!   switch-on needs energy above 0 (not measured; a weapon that cannot pay a
//!   tick would switch itself off at once).
//! - FO class (`WeaponDef::ordnance_defence`, behind `pd_debt_timer`; read
//!   from the engine's code only, NOT MEASURED): the timer is set to the raw
//!   `shotDelay` after a shot, with no jitter and no debt. FO's list of
//!   already-engaged ordnance has no effect here, since an intercept removes
//!   the shot at once, so it is not modelled.
//! - A unit whose weapons are muted (`WEAPONS_OFF`) neither drains nor fires,
//!   and a PD weapon that is on stays on and its timer stands still. Not
//!   measured; chosen so a mute is not an energy sink.
//! - The toggle is `Order::Special` on the weapon's slot
//!   ([`crate::specials::point_defence_slot`]); every change, including the
//!   auto-off, raises `SimEvent::PointDefenceChanged`. `Fo4` and `Sta2Classic`
//!   keep `pd_energy` off by default until a HUD button can toggle PD (UI-31;
//!   SIM-37 then flips the profile defaults). The original's PD weapon is a
//!   `toggle = 1` special (read from the data of Classic and FO 4.0), the
//!   executable handles a toggle request by flipping the switch (read from the
//!   executable, static), and PD stood off until switched on in the stock A2
//!   and Roots + Classic bench runs (measured).
//! - Snapshot: the rows stay `Vec<u64>`. Rows of units with energy-model
//!   state start with `u64::MAX` and carry `[ready, on, timer bits]` per
//!   hardpoint; other rows are unchanged, so old snapshots load.

use sta_content::{ContentDb, WeaponKind};
use sta_core::{EntityId, Fixed, TICK_HZ};

use super::weapons::{Shot, draw, flat};
use crate::Rng;
use crate::event::{SimEvent, Target};
use crate::specials::Specials;
use crate::world::{Entities, Fnv, Table};

/// One PD hardpoint: the first tick it may cycle again (legacy model), and
/// the energy model's switch and timer.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Mount {
    ready: u64,
    on: bool,
    /// Ticks; negative is debt (Roots model).
    timer: Fixed,
}

impl Mount {
    fn modelled(&self) -> bool {
        self.on || self.timer != Fixed::ZERO
    }
}

/// Per-unit PD hardpoints.
#[derive(Debug, Clone, Default)]
pub(crate) struct Pd {
    ready: Table<Vec<Mount>>,
}

const MARK: u64 = u64::MAX;

/// [`Pd`] as the snapshot stores it (rows ascending by unit id).
pub(crate) type PdRec = Vec<(EntityId, Vec<u64>)>;

impl Pd {
    pub(crate) fn to_rec(&self) -> PdRec {
        self.ready
            .iter()
            .map(|(id, ms)| {
                let row = if ms.iter().any(Mount::modelled) {
                    let mut row = vec![MARK];
                    for m in ms {
                        row.extend([m.ready, m.on as u64, m.timer.to_bits() as u64]);
                    }
                    row
                } else {
                    ms.iter().map(|m| m.ready).collect()
                };
                (id, row)
            })
            .collect()
    }

    /// `None` if the rows are not ascending by id or a row is malformed.
    pub(crate) fn from_rec(r: PdRec) -> Option<Pd> {
        let mut rows = Vec::with_capacity(r.len());
        for (id, row) in r {
            let ms = if row.first() == Some(&MARK) {
                let body = &row[1..];
                if !body.len().is_multiple_of(3) {
                    return None;
                }
                body.chunks(3)
                    .map(|c| {
                        Some(Mount {
                            ready: c[0],
                            on: match c[1] {
                                0 => false,
                                1 => true,
                                _ => return None,
                            },
                            timer: Fixed::from_bits(c[2] as i64),
                        })
                    })
                    .collect::<Option<Vec<_>>>()?
            } else {
                row.into_iter().map(|ready| Mount { ready, ..Mount::default() }).collect()
            };
            rows.push((id, ms));
        }
        Some(Pd {
            ready: Table::from_rows(rows)?,
        })
    }

    pub(crate) fn is_on(&self, id: EntityId, i: usize) -> bool {
        self.ready.get(id).and_then(|r| r.get(i)).is_some_and(|m| m.on)
    }

    /// Switch the PD weapon at hardpoint `i` of `u` on or off (a toggle).
    /// True when the order took.
    pub(crate) fn toggle(
        &mut self,
        db: &ContentDb,
        specials: &Specials,
        id: EntityId,
        def: &sta_content::UnitDef,
        i: usize,
    ) -> bool {
        let Some(w) = db.weapon(def.hardpoints[i].weapon.key) else { return false };
        let feats = db.weapon_features(w);
        if !(feats.pd_energy && feats.point_defense_cycles) {
            return false;
        }
        if self.ready.get(id).is_none() {
            self.ready.insert(id, vec![Mount::default(); def.hardpoints.len()]);
        }
        let on = self.ready.get(id).is_some_and(|r| r[i].on);
        if !on {
            let have = specials.energy_of(id);
            let ok = match have {
                Some(e) if feats.pd_debt_timer => e >= w.energy_cost,
                Some(e) => e > Fixed::ZERO || w.energy_cost == Fixed::ZERO,
                None => w.energy_cost == Fixed::ZERO,
            };
            if !ok {
                return false;
            }
        }
        if let Some(r) = self.ready.get_mut(id) {
            r[i].on = !on;
        }
        true
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.ready.is_empty()
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.u64(self.ready.len() as u64);
        for (id, rs) in self.ready.iter() {
            h.u64(id.0 as u64);
            for m in rs {
                h.u64(m.ready);
                if m.modelled() {
                    h.u64(m.on as u64);
                    h.fixed(m.timer);
                }
            }
        }
    }

    /// One tick of point defence: remove the shots the ready PD mounts take.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn step(
        &mut self,
        tick: u64,
        units: &Entities,
        db: &ContentDb,
        teams: crate::relations::Diplomacy<'_>,
        shots: &mut Vec<Shot>,
        specials: &mut Specials,
        rng: &mut Rng,
        events: &mut Vec<SimEvent>,
    ) {
        let gone: Vec<EntityId> = self
            .ready
            .iter()
            .filter(|(id, _)| units.get(*id).is_none_or(|u| u.hull[1] <= Fixed::ZERO))
            .map(|(id, _)| id)
            .collect();
        for id in gone {
            self.ready.remove(id);
        }
        for u in units.iter() {
            if u.hull[1] <= Fixed::ZERO || u.flags.contains(crate::view::UnitFlags::WEAPONS_OFF) {
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
                let feats = db.weapon_features(w);
                if !feats.point_defense_cycles {
                    continue;
                }
                if feats.pd_energy {
                    self.energy_cycle(tick, u.id, def, i, w, feats.pd_debt_timer, u, units, teams, shots, specials, rng, events);
                    continue;
                }
                if self.ready.get(u.id).is_some_and(|r| r[i].ready > tick) {
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
                        .is_some_and(|sh| teams.is_seat(sh.owner) && teams.confirmed_ally(sh.owner, u.owner));
                    let d = flat(u.pos, s.pos).1;
                    if friendly || d > w.range || best.is_some_and(|(bd, _)| d >= bd) {
                        continue;
                    }
                    best = Some((d, k));
                }
                let Some((_, k)) = best else { continue };
                let s = shots.remove(k);
                if self.ready.get(u.id).is_none() {
                    self.ready.insert(u.id, vec![Mount::default(); def.hardpoints.len()]);
                }
                if let Some(r) = self.ready.get_mut(u.id) {
                    r[i].ready = tick + w.refire.max(1);
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

    /// The nearest eligible shot for hardpoint `w` of `u`, as in the legacy
    /// path: homing on `u`, from a hostile team, inside range.
    fn nearest(
        u: &crate::world::Entity,
        w: &sta_content::WeaponDef,
        units: &Entities,
        teams: crate::relations::Diplomacy<'_>,
        shots: &[Shot],
    ) -> Option<usize> {
        let mut best: Option<(Fixed, usize)> = None;
        for (k, s) in shots.iter().enumerate() {
            if s.target != u.id {
                continue;
            }
            let friendly = units
                .get(s.shooter)
                .is_some_and(|sh| teams.is_seat(sh.owner) && teams.confirmed_ally(sh.owner, u.owner));
            let d = flat(u.pos, s.pos).1;
            if friendly || d > w.range || best.is_some_and(|(bd, _)| d >= bd) {
                continue;
            }
            best = Some((d, k));
        }
        best.map(|b| b.1)
    }

    /// One tick of one PD hardpoint under the energy model.
    #[allow(clippy::too_many_arguments)]
    fn energy_cycle(
        &mut self,
        _tick: u64,
        id: EntityId,
        def: &sta_content::UnitDef,
        i: usize,
        w: &sta_content::WeaponDef,
        debt: bool,
        u: &crate::world::Entity,
        units: &Entities,
        teams: crate::relations::Diplomacy<'_>,
        shots: &mut Vec<Shot>,
        specials: &mut Specials,
        rng: &mut Rng,
        events: &mut Vec<SimEvent>,
    ) {
        let Some(mut m) = self.ready.get(id).map(|r| r[i]) else { return };
        if !m.on {
            return;
        }
        // Energy first: the drain is per second whether or not there is
        // anything to shoot.
        let drain = w.energy_cost / Fixed::from_int(TICK_HZ as i32);
        let mut empty = false;
        match specials.energy_mut(id) {
            Some(e) => {
                *e = *e - drain;
                if *e <= Fixed::ZERO {
                    *e = Fixed::ZERO;
                    empty = true;
                }
            }
            None => empty = w.energy_cost > Fixed::ZERO,
        }
        if debt {
            m.timer = m.timer - Fixed::ONE;
        }
        let fo = debt && w.ordnance_defence;
        let shot_delay = Fixed::from_int(w.refire.min(i32::MAX as u64) as i32);
        while !debt || m.timer <= Fixed::ZERO {
            let Some(k) = Self::nearest(u, w, units, teams, shots) else { break };
            let s = shots.remove(k);
            if debt {
                m.timer = if fo {
                    shot_delay
                } else {
                    // + 0 to 5 % jitter from the sim's RNG.
                    m.timer + shot_delay * (Fixed::ONE + draw(rng) * Fixed::from_int(5) / Fixed::from_int(100))
                };
            }
            events.push(SimEvent::Fired {
                shooter: id,
                weapon: w.key,
                hardpoint: i as u8,
                target: Target::Point(s.pos),
            });
        }
        if empty {
            m.on = false;
            events.push(SimEvent::PointDefenceChanged { id, on: false });
        }
        let _ = def;
        if let Some(r) = self.ready.get_mut(id) {
            r[i] = m;
        }
    }
}
