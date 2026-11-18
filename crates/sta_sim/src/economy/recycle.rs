//! Decommissioning and recycling (SIM-45, first slice). Evidence: **S** read from the
//! executable, static; **D** read from data; **ours** STA64's own choice; nothing measured.
//!
//! - *Order (S).* `Order::Decommission { units, yard }` for live, finished, own crafts and
//!   stations that a script has not made undecommissionable (`SetCanBeDecommissioned`).
//! - *Where (S, D).* A station, or a craft with `recyclesInPlace = 1`, is recycled where it
//!   stands. Any other craft goes to the named yard when it is one of its owner's live,
//!   finished yards with `decommissionFacility = 1`, else to the best such yard: the highest
//!   `dilithium fraction x dilithium cost + metal fraction x metal cost - 0.1 x distance`. No yard,
//!   no order for that craft.
//! - *At the yard (S; queue ours).* The craft flies until it is [`DOCK_GAP`] off the yard's
//!   surface. The original puts it in the yard's repair queue; here each yard has its own
//!   decommission queue, the head is taken, the others hold [`WAIT_GAP`] off (ours).
//! - *Time (S, D).* The recycle takes the unit's build time times `DECONSTRUCT_PERCENT_TIME`
//!   (`RTS_CFG.h`; absent: stock Armada II's 0.67, ours).
//! - *Refund (S, D).* Dilithium and metal cost times the yard's `dilithiumRecyclingFraction` /
//!   `metalRecyclingFraction`, else the yard's race's `recycleDilithiumFraction` /
//!   `recycleMetalFraction`; in place, the unit's race's (default 0.8, S). Biomatter cost times
//!   `cfgBIOMATTER_RECYCLE_FRACTION` (absent: 0). The unit's crew goes back to the team pool
//!   (A2 `AddCrew`). Latinum is not returned. Then the unit leaves the world without dying.
//! - *End (S, ours).* Any other order, the yard's loss or the unit's end stops it; nothing is
//!   refunded then.
//!
//! Not done: the FO/Roots `recycle*` / `recycleTime` / `destroyedBiomatterCost` keys, `recycler`
//! / `recycleExchangeRate`, `eventDecommission` and the deconstruction effect, Roots' rule
//! against decommissioning a ship being boarded, the shared yard queue.

use serde::{Deserialize, Serialize};
use sta_content::ContentDb;
use sta_core::{EntityId, Fixed, PlayerId, Tick, Vec3};

use crate::Sim;
use crate::command::TeamOp;
use crate::view::UnitFlags;
use crate::world::{Entity, EntityKind, Fnv};

/// Surface gap at which the head of a yard's decommission queue is taken (S: radius + 20).
pub const DOCK_GAP: Fixed = Fixed::from_bits(20 << 32);
/// Surface gap at which a waiting craft holds (ours).
pub const WAIT_GAP: Fixed = Fixed::from_bits(150 << 32);
/// The engine's race recycle fraction when the race ODF sets none (S).
const RACE_DEFAULT_FRACTION: Fixed = Fixed::from_bits(3_435_973_837);
/// `DECONSTRUCT_PERCENT_TIME` when the header has none (stock A2's value, ours).
const DEFAULT_DECONSTRUCT: Fixed = Fixed::from_bits(2_877_628_088);

/// One unit being decommissioned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    /// `None`: in place.
    pub yard: Option<EntityId>,
    /// Ticks of recycling left once started; `None` while it travels or waits.
    pub left: Option<Tick>,
}

/// Every unit being decommissioned, in the order the orders came.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recycles {
    jobs: Vec<(EntityId, Job)>,
}

/// The snapshot form of [`Recycles`] (format 37).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecyclesRec {
    pub version: u8,
    pub jobs: Vec<(EntityId, Job)>,
}

impl Recycles {
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    pub fn job_of(&self, id: EntityId) -> Option<Job> {
        self.jobs.iter().find(|(u, _)| *u == id).map(|(_, j)| *j)
    }

    pub(crate) fn end(&mut self, id: EntityId) {
        self.jobs.retain(|(u, _)| *u != id);
    }

    /// An entity left the world: as the unit or as its yard.
    pub(crate) fn remove_entity(&mut self, id: EntityId) {
        self.jobs.retain(|(u, j)| *u != id && j.yard != Some(id));
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        if self.jobs.is_empty() {
            return;
        }
        h.str("recycles-v1");
        h.u64(self.jobs.len() as u64);
        for (u, j) in &self.jobs {
            h.u64(u64::from(u.0));
            h.u64(j.yard.map_or(0, |y| u64::from(y.0) + 1));
            h.u64(j.left.map_or(0, |t| t + 1));
        }
    }

    pub(crate) fn to_rec(&self) -> RecyclesRec {
        RecyclesRec { version: 1, jobs: self.jobs.clone() }
    }

    /// No unit twice, none its own yard, every id in the world.
    pub(crate) fn from_rec(rec: RecyclesRec, exists: impl Fn(EntityId) -> bool) -> Option<Recycles> {
        if rec.version != 1 {
            return None;
        }
        let mut seen: Vec<EntityId> = rec.jobs.iter().map(|(u, _)| *u).collect();
        let n = seen.len();
        seen.sort();
        seen.dedup();
        let ok = seen.len() == n && rec.jobs.iter().all(|(u, j)| exists(*u) && j.yard.is_none_or(|y| y != *u && exists(y)));
        ok.then_some(Recycles { jobs: rec.jobs })
    }
}

fn alive(e: &Entity) -> bool {
    e.hull[0] > Fixed::ZERO && !e.flags.contains(UnitFlags::DYING) && e.progress.is_none_or(|p| p >= Fixed::ONE)
}

/// Saturating `a x b`.
fn mul(a: Fixed, b: Fixed) -> Fixed {
    Fixed::from_bits(((i128::from(a.to_bits()) * i128::from(b.to_bits())) >> 32).clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64)
}

fn dist(a: Vec3, b: Vec3) -> Fixed {
    let d = |p: Fixed, q: Fixed| i128::from(p.to_bits()) - i128::from(q.to_bits());
    let (dx, dz) = (d(a.x, b.x), d(a.z, b.z));
    Fixed::from_bits(((dx * dx + dz * dz) >> 32).min(i128::from(i64::MAX)) as i64).sqrt()
}

impl Sim {
    /// A race's recycle fractions (dilithium, metal), the engine's 0.8 where it sets none.
    fn race_recycle(db: &ContentDb, race: &str) -> (Fixed, Fixed) {
        let r = db.rules().race_repair_of(race).unwrap_or_default();
        (r.recycle_dilithium.unwrap_or(RACE_DEFAULT_FRACTION), r.recycle_metal.unwrap_or(RACE_DEFAULT_FRACTION))
    }

    /// A yard's fractions: its own keys, else its race's.
    fn yard_fractions(&self, db: &ContentDb, yard: EntityId) -> (Fixed, Fixed) {
        let race = self.unit_race(yard).unwrap_or("").to_owned();
        let (rd, rm) = Self::race_recycle(db, &race);
        let def = self.units.get(yard).and_then(|y| db.unit(y.def));
        (def.and_then(|d| d.repair.dilithium_recycling).unwrap_or(rd), def.and_then(|d| d.repair.metal_recycling).unwrap_or(rm))
    }

    /// A live, finished yard of `owner` that takes decommissioned ships.
    fn decommission_yard_ok(&self, db: &ContentDb, yard: EntityId, owner: PlayerId) -> bool {
        self.units.get(yard).is_some_and(|y| {
            y.owner == owner && alive(y) && y.kind == EntityKind::Station && db.unit(y.def).is_some_and(|d| d.repair.decommission_facility == Some(true))
        })
    }

    /// `Order::Decommission`.
    pub(crate) fn apply_decommission(&mut self, player: PlayerId, units: &[EntityId], yard: Option<EntityId>) -> bool {
        let Some(db) = self.content.clone() else { return false };
        let mut ids = units.to_vec();
        ids.sort();
        ids.dedup();
        let mut taken = Vec::new();
        for id in ids {
            let Some(u) = self.units.get(id) else { continue };
            let Some(def) = db.unit(u.def) else { continue };
            if !matches!(u.kind, EntityKind::Craft | EntityKind::Station)
                || !alive(u)
                || (player.is_player() && u.owner != player)
                || self.switches.get(id).no_decommission
                || self.recycles.job_of(id).is_some()
            {
                continue;
            }
            let owner = u.owner;
            let in_place = u.kind == EntityKind::Station || def.repair.recycles_in_place == Some(true);
            let at = if in_place {
                None
            } else {
                let chosen = match yard {
                    Some(y) if self.decommission_yard_ok(&db, y, owner) => Some(y),
                    _ => {
                        let (dc, mc) = (def.build.cost.first().copied().unwrap_or(Fixed::ZERO), def.build.cost.get(2).copied().unwrap_or(Fixed::ZERO));
                        let pos = u.pos;
                        let mut best: Option<(Fixed, EntityId)> = None;
                        for y in self.units.iter() {
                            if !self.decommission_yard_ok(&db, y.id, owner) {
                                continue;
                            }
                            let (fd, fm) = self.yard_fractions(&db, y.id);
                            let score = mul(fd, dc) + mul(fm, mc) - mul(dist(pos, y.pos), Fixed::from_bits(429_496_730));
                            if best.is_none_or(|(b, _)| score > b) {
                                best = Some((score, y.id));
                            }
                        }
                        best.map(|(_, y)| y)
                    }
                };
                let Some(y) = chosen else { continue };
                Some(y)
            };
            self.recycles.jobs.push((id, Job { yard: at, left: None }));
            taken.push(id);
        }
        if taken.is_empty() {
            return false;
        }
        // Like every order that replaces what a unit is doing.
        self.weapons.stop_attack(&self.units, player, &taken);
        self.economy.cancel(&self.units, player, &taken);
        self.production.cancel(&db, &self.units, &mut self.economy, player, &taken);
        self.clear_scripted(player, &taken);
        for &id in &taken {
            self.repairs.end(id);
            self.set_target(id, None);
        }
        true
    }

    /// What other orders do to a unit being decommissioned: they stop it.
    pub(crate) fn recycles_after_order(&mut self, player: PlayerId, order: &crate::command::Order) {
        use crate::command::Order;
        if self.recycles.is_empty() {
            return;
        }
        let units: &[EntityId] = match order {
            Order::Decommission { .. } => return,
            Order::Stop { units }
            | Order::Move { units, .. }
            | Order::MoveWarp { units, .. }
            | Order::Attack { units, .. }
            | Order::Mine { units, .. }
            | Order::Patrol { units, .. }
            | Order::SearchAndDestroy { units }
            | Order::Scout { units }
            | Order::AttackAny { units }
            | Order::AttackPoint { units, .. }
            | Order::Guard { units, .. }
            | Order::Colonize { units, .. }
            | Order::Transport { units, .. }
            | Order::Repair { units, .. } => units,
            Order::Build { builder, .. } => std::slice::from_ref(builder),
            _ => return,
        };
        for &id in units {
            if self.units.get(id).is_some_and(|u| !player.is_player() || u.owner == player) {
                self.recycles.end(id);
            }
        }
    }

    /// One tick of every decommission.
    pub(crate) fn step_recycles(&mut self) {
        if self.recycles.is_empty() {
            return;
        }
        let Some(db) = self.content.clone() else { return };
        let percent = db.rules().cfg.deconstruct_percent_time.unwrap_or(DEFAULT_DECONSTRUCT).max(Fixed::ZERO);
        let mut heads: Vec<EntityId> = Vec::new();
        let mut done: Vec<EntityId> = Vec::new();
        for (id, job) in self.recycles.jobs.clone() {
            let Some(u) = self.units.get(id) else { continue };
            let Some(def) = db.unit(u.def) else { continue };
            let valid = alive(u) && job.yard.is_none_or(|y| self.decommission_yard_ok(&db, y, u.owner));
            if !valid {
                self.recycles.end(id);
                if u.kind == EntityKind::Craft {
                    self.set_target(id, None);
                }
                continue;
            }
            let start = |left: Option<Tick>| left.unwrap_or_else(|| (mul(Fixed::from_int(def.build.ticks.min(i32::MAX as u64) as i32), percent).to_bits() >> 32).max(0) as Tick);
            let left = match job.yard {
                None => Some(start(job.left)),
                Some(y) => {
                    let Some(yd) = self.units.get(y) else { continue };
                    let gap = dist(u.pos, yd.pos) - def.radius - db.unit(yd.def).map_or(Fixed::ZERO, |d| d.radius);
                    let ypos = yd.pos;
                    let head = !heads.contains(&y);
                    if head {
                        heads.push(y);
                    }
                    if job.left.is_some() {
                        job.left
                    } else if head && gap <= DOCK_GAP {
                        self.set_target(id, None);
                        Some(start(None))
                    } else {
                        let hold = !head && gap <= WAIT_GAP;
                        self.set_target(id, if hold { None } else { Some(ypos) });
                        None
                    }
                }
            };
            let next = left.map(|t| t.saturating_sub(1));
            if next == Some(0) {
                done.push(id);
            }
            if let Some((_, j)) = self.recycles.jobs.iter_mut().find(|(u, _)| *u == id) {
                j.left = next;
            }
        }
        for id in done {
            self.finish_recycle(&db, id);
        }
    }

    /// The refund and the unit's exit.
    fn finish_recycle(&mut self, db: &ContentDb, id: EntityId) {
        let Some(job) = self.recycles.job_of(id) else { return };
        self.recycles.end(id);
        let Some(u) = self.units.get(id) else { return };
        let Some(def) = db.unit(u.def) else { return };
        let owner = u.owner;
        let crew = u.crew.map_or(Fixed::ZERO, |c| c[0]);
        let (fd, fm) = match job.yard {
            Some(y) => self.yard_fractions(db, y),
            None => {
                let race = self.unit_race(id).unwrap_or("").to_owned();
                Self::race_recycle(db, &race)
            }
        };
        let bio = db.rules().cfg.biomatter_recycle_fraction.unwrap_or(Fixed::ZERO).max(Fixed::ZERO);
        let cost = |i: usize| def.build.cost.get(i).copied().unwrap_or(Fixed::ZERO).max(Fixed::ZERO);
        for (slot, amount) in [(0u8, mul(cost(0), fd)), (2, mul(cost(2), fm)), (3, mul(cost(3), bio))] {
            if amount > Fixed::ZERO {
                self.edit_team_pool(owner, TeamOp::ResourceAdd(slot), amount);
            }
        }
        if crew > Fixed::ZERO {
            self.edit_team_pool(owner, TeamOp::CrewAddA2, crew);
        }
        self.retire(id);
    }
}
