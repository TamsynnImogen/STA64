//! Yard repair at runtime (SIM-40): `Order::Repair`, the yard queues and the
//! service of the ship at the head of each queue.
//!
//! Rules, with their grades (**S** read from the Armada II executable,
//! static; **D** read from data; **ours** STA64's own choice; nothing here is
//! measured on a running game):
//!
//! - *Yards (D, S).* A finished, live station whose ODF says
//!   `repairFacility = 1`, owned by the ship's owner or a team mate (ours: the
//!   original's ally rule is not read).
//! - *Ships (D).* A live craft that can move, owned by the ordering player
//!   (any for scripts), whose ODF does not say `canShipyardRepair = 0`, and
//!   that needs repair (below).
//! - *Which yard (S).* The order's yard when it is eligible, else the lowest
//!   `surface distance / speed + 5 x queue length` (`super::select_a2_yard`).
//! - *Needs repair (S).* Hull below maximum, shields below maximum, a damaged
//!   system, or at least 0.1 crew missing that the team pool can give.
//! - *Roots* (Fo4 profile): both reaches are [`ROOTS_EXTRA_REACH`] larger (changelog, not
//!   measured).
//! - *Queue (S, ours).* First come, first served per yard. The head of the
//!   queue flies to the yard and is serviced once its surface distance is at
//!   most [`DOCK_REACH`] (ours; the original's dock point and entry distance
//!   are geometry we do not read, `super::a2_dock_offset` is not wired). Ships
//!   behind it fly to the yard and hold [`WAIT_REACH`] off its surface (ours).
//! - *Service per second (S).* Crew: `BASE_CREW_PER_SEC` (`yard_crew_per_sec`)
//!   from the team pool, up to the missing crew. Shields: 50 x the unit's
//!   `shieldRate`. Each damaged system: 100 / its modified repair time
//!   (`super::a2_modified_system_time` with the race's `repairStrength`,
//!   `repairYellow`, `repairRed`, `crewYellowStatus`, `crewRedStatus`; the
//!   engine's defaults 1, 2, 4, 0.5, 0.25 when the race leaves them out, S).
//!   Hull: 100 x `healthRate`, which defaults to 0.2 x `shieldRate` (S).
//! - *A system with no `<system>RepairTime`* (ours): restored at once, since
//!   the engine takes the value from a class template we do not read.
//! - *End.* When nothing needs repair the ship leaves the queue and stays
//!   where it is. Any other order, Stop, the yard's loss or a change of owner
//!   of either ends it; nothing is refunded (there is no cost).
//!
//! Not done: the dock point on the yard model (`repairHardpoint`,
//! `repairSide`), the Roots deltas (queue entry 50 further, priority repair
//! that a second order does not cancel), special energy refilled on
//! completion, AI use.

use serde::{Deserialize, Serialize};
use sta_content::ContentDb;
use sta_core::{EntityId, Fixed, PlayerId, TICK_HZ};

use super::{RepairTimeFactors, YardCandidate, a2_modified_system_time, select_a2_yard};
use crate::Sim;
use crate::view::UnitFlags;
use crate::world::{Entity, EntityKind, Fnv};

/// Surface distance at which the head of a queue is serviced (ours).
pub const DOCK_REACH: Fixed = Fixed::from_bits(30 << 32);
/// Surface distance at which a waiting ship holds (ours).
pub const WAIT_REACH: Fixed = Fixed::from_bits(150 << 32);
/// Roots (the Fo4 profile) lets a ship enter the queue 50 further out than stock
/// (WORKPLAN SIM-40, from the Roots changelog; not measured): both reaches grow by it.
pub const ROOTS_EXTRA_REACH: Fixed = Fixed::from_bits(50 << 32);

/// The dock and wait reaches of `db`'s profile.
pub fn reaches(db: &ContentDb) -> (Fixed, Fixed) {
    let extra = if db.profile() == sta_content::Profile::Fo4 { ROOTS_EXTRA_REACH } else { Fixed::ZERO };
    (DOCK_REACH + extra, WAIT_REACH + extra)
}

/// 0.1 crew: less missing than this does not count (S).
const CREW_SLACK: Fixed = Fixed::from_bits(429_496_730);

/// The yard queues: `(yard, ships in arrival order)`, ascending yard; and the repair ships'
/// beams (SIM-41): `(repair ship, target, hold)`, ascending ship; `hold`: taken on its own
/// within range (Roots' medium autonomy), so the ship does not move for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Repairs {
    queues: Vec<(EntityId, Vec<EntityId>)>,
    pub(super) beams: Vec<(EntityId, EntityId, bool)>,
}

/// The snapshot form of [`Repairs`] (format 37; version 2 adds the beams).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairsRec {
    pub version: u8,
    pub queues: Vec<(EntityId, Vec<EntityId>)>,
    pub beams: Vec<(EntityId, EntityId, bool)>,
}

impl Repairs {
    pub fn is_empty(&self) -> bool {
        self.queues.is_empty() && self.beams.is_empty()
    }

    /// The ship repair ship `ship` is repairing (SIM-41).
    pub fn beam_of(&self, ship: EntityId) -> Option<EntityId> {
        self.beams.iter().find(|(s, _, _)| *s == ship).map(|(_, t, _)| *t)
    }

    pub(super) fn set_beam(&mut self, ship: EntityId, target: EntityId, hold: bool) {
        self.end(ship);
        match self.beams.binary_search_by_key(&ship, |(s, _, _)| *s) {
            Ok(i) => self.beams[i] = (ship, target, hold),
            Err(i) => self.beams.insert(i, (ship, target, hold)),
        }
    }

    /// The yard `ship` is queued at.
    pub fn yard_of(&self, ship: EntityId) -> Option<EntityId> {
        self.queues.iter().find(|(_, q)| q.contains(&ship)).map(|(y, _)| *y)
    }

    /// The ships queued at `yard`, head first.
    pub fn queue(&self, yard: EntityId) -> &[EntityId] {
        self.queues.iter().find(|(y, _)| *y == yard).map_or(&[], |(_, q)| q.as_slice())
    }

    fn len_at(&self, yard: EntityId) -> u32 {
        self.queue(yard).len() as u32
    }

    /// Take `ship` out of any queue.
    pub(crate) fn end(&mut self, ship: EntityId) {
        for (_, q) in &mut self.queues {
            q.retain(|s| *s != ship);
        }
        self.queues.retain(|(_, q)| !q.is_empty());
        self.beams.retain(|(s, _, _)| *s != ship);
    }

    /// An entity left the world: as a ship or as a yard.
    pub(crate) fn remove_entity(&mut self, id: EntityId) {
        self.queues.retain(|(y, _)| *y != id);
        self.end(id);
    }

    fn push(&mut self, yard: EntityId, ship: EntityId) {
        self.end(ship);
        match self.queues.binary_search_by_key(&yard, |(y, _)| *y) {
            Ok(i) => self.queues[i].1.push(ship),
            Err(i) => self.queues.insert(i, (yard, vec![ship])),
        }
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.str("repairs-v1");
        h.u64(self.queues.len() as u64);
        for (y, q) in &self.queues {
            h.u64(u64::from(y.0));
            h.u64(q.len() as u64);
            for s in q {
                h.u64(u64::from(s.0));
            }
        }
        // Only when there are any, so worlds without repair ships hash as before.
        if !self.beams.is_empty() {
            h.str("repair-beams-v1");
            h.u64(self.beams.len() as u64);
            for (s, t, hold) in &self.beams {
                h.u64(u64::from(s.0));
                h.u64(u64::from(t.0));
                h.u64(u64::from(*hold));
            }
        }
    }

    pub(crate) fn to_rec(&self) -> RepairsRec {
        RepairsRec { version: 2, queues: self.queues.clone(), beams: self.beams.clone() }
    }

    /// Ascending yards, no empty queue, no ship twice, every id in the world; beams by
    /// ascending repair ship, none on itself or also queued at a yard.
    pub(crate) fn from_rec(rec: RepairsRec, exists: impl Fn(EntityId) -> bool) -> Option<Repairs> {
        if rec.version != 2 || !rec.queues.windows(2).all(|w| w[0].0 < w[1].0) || !rec.beams.windows(2).all(|w| w[0].0 < w[1].0) {
            return None;
        }
        if rec.beams.iter().any(|(s, t, _)| s == t || !exists(*s) || !exists(*t) || rec.queues.iter().any(|(_, q)| q.contains(s))) {
            return None;
        }
        let mut seen: Vec<EntityId> = Vec::new();
        for (y, q) in &rec.queues {
            if q.is_empty() || !exists(*y) || q.iter().any(|s| !exists(*s) || s == y) {
                return None;
            }
            seen.extend(q.iter().copied());
        }
        let n = seen.len();
        seen.sort();
        seen.dedup();
        (seen.len() == n).then_some(Repairs { queues: rec.queues, beams: rec.beams })
    }
}

pub(super) fn alive(e: &Entity) -> bool {
    e.hull[0] > Fixed::ZERO && !e.flags.contains(UnitFlags::DYING)
}

/// Saturating `a x b` for non-negative quantities.
fn mul(a: Fixed, b: Fixed) -> Fixed {
    Fixed::from_bits(((i128::from(a.to_bits()) * i128::from(b.to_bits())) >> 32).clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64)
}

/// Centre distance minus both radii, in 128-bit arithmetic.
pub(super) fn surface(a: &Entity, ra: Fixed, b: &Entity, rb: Fixed) -> Fixed {
    let d = |p: Fixed, q: Fixed| i128::from(p.to_bits()) - i128::from(q.to_bits());
    let (dx, dz) = (d(a.pos.x, b.pos.x), d(a.pos.z, b.pos.z));
    let sq = ((dx * dx + dz * dz) >> 32).min(i128::from(i64::MAX)) as i64;
    Fixed::from_bits(sq).sqrt() - ra - rb
}

/// `per_second / TICK_HZ`: one tick's share.
fn per_tick(per_second: Fixed) -> Fixed {
    Fixed::from_bits(per_second.to_bits() / i64::from(TICK_HZ))
}

impl Sim {
    /// The yard queues (SIM-40).
    pub fn repairs(&self) -> &Repairs {
        &self.repairs
    }

    fn repair_team(&self, a: PlayerId, b: PlayerId) -> bool {
        let team_of = |p: PlayerId| self.players.iter().find(|(q, _)| *q == p).map(|(_, t)| *t);
        a == b || team_of(a).is_some() && team_of(a) == team_of(b)
    }

    /// A yard `ship` (owned by `owner`) may repair at.
    pub(super) fn repair_yard_ok(&self, db: &ContentDb, yard: EntityId, owner: PlayerId) -> bool {
        self.units.get(yard).is_some_and(|y| {
            y.kind == EntityKind::Station
                && alive(y)
                && y.progress.is_none_or(|p| p >= Fixed::ONE)
                && self.repair_team(y.owner, owner)
                && db.unit(y.def).is_some_and(|d| d.repair.repair_facility == Some(true))
        })
    }

    /// Whether the unit needs anything a yard gives (S).
    fn needs_repair(&self, db: &ContentDb, id: EntityId) -> bool {
        let Some(u) = self.units.get(id) else { return false };
        let Some(def) = db.unit(u.def) else { return false };
        if u.hull[0] < u.hull[1] {
            return true;
        }
        let pools = if u.shields.directional { 4 } else { 1 };
        if u.shields.max > Fixed::ZERO && u.shields.now[..pools].iter().any(|s| *s < u.shields.max) {
            return true;
        }
        let row = self.subsystems.get(id).unwrap_or_default();
        if (0..5).any(|n| def.system_hit_points[n] > Fixed::ZERO && row.damage[n] > 0) {
            return true;
        }
        u.crew.is_some_and(|[now, max]| {
            let missing = max - now;
            missing >= CREW_SLACK && self.economy.crew_of(u.owner).is_some_and(|pool| pool.min(missing) >= CREW_SLACK)
        })
    }

    /// `Order::Repair`: queue each eligible ship at the given yard, else the
    /// best one. False when no ship was queued.
    pub(crate) fn apply_repair(&mut self, player: PlayerId, units: &[EntityId], yard: Option<EntityId>) -> bool {
        let Some(db) = self.content.clone() else { return false };
        let mut ids = units.to_vec();
        ids.sort();
        ids.dedup();
        // SIM-41: a repair ship told to repair a ship that is not a yard for it beams it.
        let beamed = yard.map_or_else(Vec::new, |t| self.assign_beams(&db, player, &ids, t));
        ids.retain(|id| !beamed.contains(id));
        if ids.is_empty() && !beamed.is_empty() {
            return true;
        }
        let mut taken = Vec::new();
        for id in ids {
            let Some(u) = self.units.get(id) else { continue };
            let Some(def) = db.unit(u.def) else { continue };
            if u.kind != EntityKind::Craft
                || !alive(u)
                || !u.mobile()
                || (player.is_player() && u.owner != player)
                || def.repair.can_shipyard_repair == Some(false)
                || !self.needs_repair(&db, id)
            {
                continue;
            }
            let owner = u.owner;
            let chosen = match yard {
                Some(y) => self.repair_yard_ok(&db, y, owner).then_some(y),
                None => {
                    let speed = u.stats.max_speed;
                    let radius = def.radius;
                    let candidates: Vec<YardCandidate> = self
                        .units
                        .iter()
                        .filter(|y| self.repair_yard_ok(&db, y.id, owner))
                        .map(|y| YardCandidate {
                            id: y.id,
                            surface_distance: surface(u, radius, y, db.unit(y.def).map_or(Fixed::ZERO, |d| d.radius)),
                            queued: self.repairs.len_at(y.id),
                        })
                        .collect();
                    select_a2_yard(&candidates, speed)
                }
            };
            let Some(y) = chosen else { continue };
            self.repairs.push(y, id);
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
            self.watchdog.remove(id);
            self.movement.clear_waypoints(id);
            let to = self.repairs.yard_of(id).and_then(|y| self.units.get(y)).map(|y| y.pos);
            self.set_target(id, to);
        }
        true
    }

    /// What other orders do to a ship in a repair queue: they take it out.
    pub(crate) fn repairs_after_order(&mut self, player: PlayerId, order: &crate::command::Order) {
        use crate::command::Order;
        if self.repairs.is_empty() {
            return;
        }
        let units: &[EntityId] = match order {
            Order::Repair { .. } => return,
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
            | Order::Decommission { units, .. } => units,
            Order::Build { builder, .. } => std::slice::from_ref(builder),
            _ => return,
        };
        for &id in units {
            if self.units.get(id).is_some_and(|u| !player.is_player() || u.owner == player) {
                self.repairs.end(id);
            }
        }
    }

    /// One tick of every yard queue (after combat, so a hit this tick counts).
    pub(crate) fn step_repairs(&mut self) {
        if self.repairs.is_empty() {
            return;
        }
        let Some(db) = self.content.clone() else { return };
        let (dock_reach, wait_reach) = reaches(&db);
        self.step_beams(&db);
        let queues = self.repairs.queues.clone();
        for (yard, ships) in queues {
            // Drop what is no longer valid: a dead yard, a ship gone, moved to another owner,
            // or (after its own repair) no longer in need.
            let mut head = true;
            for ship in ships {
                let owner = self.units.get(ship).map(|u| u.owner);
                let valid = owner.is_some_and(|o| self.repair_yard_ok(&db, yard, o))
                    && self.units.get(ship).is_some_and(|u| alive(u) && u.kind == EntityKind::Craft);
                if !valid || !self.needs_repair(&db, ship) {
                    self.repairs.end(ship);
                    if valid {
                        self.set_target(ship, None);
                    }
                    continue;
                }
                let (Some(u), Some(y)) = (self.units.get(ship), self.units.get(yard)) else { continue };
                let (ur, yr) = (db.unit(u.def).map_or(Fixed::ZERO, |d| d.radius), db.unit(y.def).map_or(Fixed::ZERO, |d| d.radius));
                let gap = surface(u, ur, y, yr);
                let ypos = y.pos;
                if head {
                    head = false;
                    if gap <= dock_reach {
                        self.set_target(ship, None);
                        self.service(&db, ship);
                    } else {
                        self.set_target(ship, Some(ypos));
                    }
                } else if gap <= wait_reach {
                    self.set_target(ship, None);
                } else {
                    self.set_target(ship, Some(ypos));
                }
            }
        }
    }

    /// One tick of the yard's work on a docked ship (S).
    fn service(&mut self, db: &ContentDb, id: EntityId) {
        let Some(u) = self.units.get(id) else { return };
        if db.unit(u.def).is_none() {
            return;
        }
        let owner = u.owner;
        // Crew from the team pool.
        if let Some([now, max]) = u.crew
            && max - now >= CREW_SLACK
        {
            let want = per_tick(db.rules().cfg.yard_crew_per_sec()).min(max - now);
            if want > Fixed::ZERO {
                let got = self.economy.withdraw_crew(owner, want);
                if let Some(u) = self.units.get_mut(id) {
                    u.crew = Some([(now + got).min(max), max]);
                }
            }
        }
        self.repair_systems_and_hull(db, id, Fixed::ONE);
        self.restore_shields(db, id, Fixed::ONE);
    }

    /// Shields at 50 times the unit's own recharge rate, times `scale` (S).
    pub(super) fn restore_shields(&mut self, db: &ContentDb, id: EntityId, scale: Fixed) {
        let Some(u) = self.units.get(id) else { return };
        let Some(def) = db.unit(u.def) else { return };
        let shield_rate = def.shields.recharge * Fixed::from_int(TICK_HZ as i32);
        let Some(u) = self.units.get_mut(id) else { return };
        if u.shields.max > Fixed::ZERO {
            let gain = mul(mul(per_tick(shield_rate), Fixed::from_int(50)), scale);
            let pools = if u.shields.directional { 4 } else { 1 };
            for s in &mut u.shields.now[..pools] {
                *s = (*s + gain).min(u.shields.max);
            }
        }
    }

    /// The part of a yard's work a repair ship's beam also does (SIM-41): systems at 100 points
    /// per second by their modified repair time, hull at 100 times `healthRate`, both times
    /// `scale` (1 at a yard, the repair ship's `repairRate` for a beam; read from the
    /// executable, static).
    pub(super) fn repair_systems_and_hull(&mut self, db: &ContentDb, id: EntityId, scale: Fixed) {
        let Some(u) = self.units.get(id) else { return };
        let Some(def) = db.unit(u.def) else { return };
        let shield_rate = def.shields.recharge * Fixed::from_int(TICK_HZ as i32);
        let health_rate = def.stats.health_rate.unwrap_or_else(|| mul(shield_rate, Fixed::from_bits(858_993_459)));
        // Systems, by their modified repair time.
        let race = self.unit_race(id).unwrap_or("").to_owned();
        let r = db.rules().race_repair_of(&race).unwrap_or_default();
        let factors = RepairTimeFactors {
            strength: r.strength.unwrap_or(Fixed::ONE),
            yellow: r.yellow.unwrap_or(Fixed::from_int(2)),
            red: r.red.unwrap_or(Fixed::from_int(4)),
            crew_yellow_status: r.crew_yellow_status.unwrap_or(Fixed::from_bits(1 << 31)),
            crew_red_status: r.crew_red_status.unwrap_or(Fixed::from_bits(1 << 30)),
        };
        let crew_fraction = self.units.get(id).and_then(|u| u.crew).filter(|c| c[1] > Fixed::ZERO).map_or(Fixed::ONE, |[n, m]| Fixed::from_bits(((i128::from(n.to_bits()) << 32) / i128::from(m.to_bits())).clamp(0, i128::from(i64::MAX)) as i64));
        let budget = mul(per_tick(Fixed::from_int(100)), scale);
        let row = self.subsystems.get(id).unwrap_or_default();
        for n in 0..5 {
            if def.system_hit_points[n] <= Fixed::ZERO || row.damage[n] <= 0 {
                continue;
            }
            let healed = match def.repair.system_times[n] {
                Some(t) => match a2_modified_system_time(t, crew_fraction, factors, None) {
                    Some(m) if m > Fixed::ZERO => Fixed::from_bits(((i128::from(budget.to_bits()) << 32) / i128::from(m.to_bits())).min(i128::from(i64::MAX)) as i64),
                    _ => Fixed::from_bits(i64::MAX),
                },
                None => Fixed::from_bits(i64::MAX),
            };
            self.yard_repair_system(id, n, healed);
        }
        let Some(u) = self.units.get_mut(id) else { return };
        // Hull, at 100 times `healthRate`.
        if health_rate > Fixed::ZERO {
            let gain = mul(mul(per_tick(health_rate), Fixed::from_int(100)), scale);
            u.hull[0] = (u.hull[0] + gain).min(u.hull[1]);
        }
    }
}
