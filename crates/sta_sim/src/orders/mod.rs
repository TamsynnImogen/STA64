//! Scripted order variants (SIM-56): `Patrol`, `SearchAndDestroy`, `Scout`,
//! `AI_Attack` without a target and at a point, and the warp move
//! (`Goto(.., warp)`). Implements D5, D9.
//!
//! Each order is a row per craft ([`Job`]); a new move, stop, attack, build or
//! mine order, or any scripted order, replaces it. Rules read from
//! Armada2.exe 1.1 (static; nothing is measured):
//!
//! - **Patrol.** A path with type 2 loops: the craft starts at the path's first
//!   point (not the nearest), flies each leg at impulse speed and from the last
//!   point goes back to the first, for ever. There is no ping-pong. It fires at
//!   the best enemy in range on the way and never turns aside for one (the
//!   script's `Patrol` issues GO; only the player's PATROL command halts at red
//!   alert). Being attacked does not interrupt it and an idle-assist order is
//!   not taken (assist is part of the idle state). Each leg is a new move
//!   target, so the stuck-order watch (`watchdog`) restarts its row per leg; a
//!   leg the watch ends ends the patrol (STA64's own safety net).
//! - **Search and destroy.** Red alert; fight the best enemy (`targeting`) the
//!   craft's sensors show within `max(scan range, weapon range)`, closing in
//!   by the attack approach; with none, fly to a random point of the map, a new
//!   one on arrival or when the leg is ended. Never ends. Engine detail not
//!   followed: it re-chooses every two seconds (STA64 keeps a target until it
//!   dies or leaves that radius) and warps only while the target warps.
//! - **Scout.** Never fires, cloaks if it can. Goes to random places near its
//!   start point in a radius of 500 that it has not been to; none left: the
//!   radius grows by 500 up to the map's diagonal, then any point of the map.
//!   Never ends. The engine's explored-tile map is approximated by 500-unit
//!   cells (the tile size was not read), its flee-when-attacked rule needs a
//!   non-red alert (STA64 crafts are always red) and its warp at high
//!   autonomy is not modelled.
//! - **Attack without a target** (`AttackAny`): take the best enemy in sensor
//!   range, approach and fight it, take the next after it dies, and stop when
//!   none is seen. Not an attack-move. Low autonomy's end state (the engine
//!   probably holds) is not measured; every autonomy stops here.
//! - **Attack at a point** (`AttackPoint`): fly to within weapon range minus 50
//!   of the point firing nothing, then stop. Artillery fire at the point is not
//!   modelled.
//! - **Warp move.** `GO_WARP` asks for warp when the goal is more than 800
//!   (`cfgTOO_CLOSE_TO_WARP`) away and then is a plain move; STA64 warps like an
//!   attack approach does (aligned, above 0.9 of impulse speed, out at the
//!   engine's lead distance). Whether the engine's extra team gate lets it
//!   warp is not established.
//!
//! State is in snapshots (format 21) and in the digest once any exists.

use serde::{Deserialize, Serialize};
use sta_core::{EntityId, Fixed, PlayerId, Vec3};

use crate::combat::targeting;
use crate::command::Order;
use crate::kinematics::Plan;
use crate::movement::approach;
use crate::world::{Entity, EntityKind, Fnv, Table};
use crate::Sim;

/// Where a warp move asks for warp: farther than this from the goal.
fn too_close_to_warp() -> Fixed {
    Fixed::from_int(800)
}

/// Scout cells and radius step.
const CELL: i32 = 500;

/// One craft's scripted order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Job {
    /// `leg`: the index of the point it is flying to; the one after it is
    /// queued, so the course never ends at the craft's own position.
    Patrol { points: Vec<Vec3>, leg: usize },
    SearchAndDestroy,
    Scout { start: Vec3, radius: Fixed, dest: Option<Vec3>, explored: Vec<(i32, i32)>, cloaked: bool },
    AttackAny,
    AttackPoint { at: Vec3 },
    Warp,
}

/// Every craft's scripted order, ascending id.
#[derive(Debug, Clone, Default)]
pub struct Scripted {
    rows: Table<Job>,
}

impl Scripted {
    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    pub(crate) fn remove(&mut self, id: EntityId) {
        self.rows.remove(id);
    }
    pub fn job(&self, id: EntityId) -> Option<&Job> {
        self.rows.get(id)
    }
    pub(crate) fn is_warp(&self, id: EntityId) -> bool {
        matches!(self.rows.get(id), Some(Job::Warp))
    }
    /// The crafts that fire nothing this tick (scouts, a craft on its way to
    /// an attack point).
    pub(crate) fn holding_fire(&self) -> Vec<EntityId> {
        self.rows.iter().filter(|(_, j)| matches!(j, Job::Scout { .. } | Job::AttackPoint { .. })).map(|(id, _)| id).collect()
    }
    pub(crate) fn to_rows(&self) -> Vec<(EntityId, Job)> {
        self.rows.iter().map(|(id, j)| (id, j.clone())).collect()
    }
    pub(crate) fn from_rows(rows: Vec<(EntityId, Job)>) -> Option<Scripted> {
        // A scout's explored cells ascend and its radius is a sane size (a
        // forged huge one would stall the destination search).
        let sane = |j: &Job| match j {
            Job::Scout { radius, explored, .. } => {
                *radius > Fixed::ZERO && *radius <= Fixed::from_int(1_000_000) && explored.windows(2).all(|w| w[0] < w[1])
            }
            _ => true,
        };
        if !rows.iter().all(|(_, j)| sane(j)) {
            return None;
        }
        Some(Scripted { rows: Table::from_rows(rows)? })
    }
    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.str("scripted-orders-v1");
        h.u64(self.rows.len() as u64);
        let v3 = |h: &mut Fnv, p: &Vec3| {
            for f in [p.x, p.y, p.z] {
                h.fixed(f);
            }
        };
        for (id, j) in self.rows.iter() {
            h.u64(id.0 as u64);
            match j {
                Job::Patrol { points, leg } => {
                    h.u64(1);
                    h.u64(*leg as u64);
                    h.u64(points.len() as u64);
                    points.iter().for_each(|p| v3(h, p));
                }
                Job::SearchAndDestroy => h.u64(2),
                Job::Scout { start, radius, dest, explored, cloaked } => {
                    h.u64(3);
                    v3(h, start);
                    h.fixed(*radius);
                    match dest {
                        Some(d) => {
                            h.u64(1);
                            v3(h, d);
                        }
                        None => h.u64(0),
                    }
                    h.u64(explored.len() as u64);
                    for (a, b) in explored {
                        h.u64(*a as u32 as u64);
                        h.u64(*b as u32 as u64);
                    }
                    h.u64(u64::from(*cloaked));
                }
                Job::AttackAny => h.u64(4),
                Job::AttackPoint { at } => {
                    h.u64(5);
                    v3(h, at);
                }
                Job::Warp => h.u64(6),
            }
        }
    }
}

fn flat(a: Vec3, b: Vec3) -> Fixed {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    (dx * dx + dz * dz).sqrt()
}

impl Sim {
    /// Drop the scripted orders of `ids` (a new ordinary order replaces them).
    /// Only crafts `player` may order lose theirs (any, for scripts and the bench).
    pub(crate) fn clear_scripted(&mut self, player: PlayerId, ids: &[EntityId]) {
        if self.scripted.is_empty() {
            return;
        }
        for &id in ids {
            if !self.units.get(id).is_some_and(|u| !player.is_player() || u.owner == player) {
                continue;
            }
            if matches!(self.scripted.rows.remove(id), Some(Job::Warp)) {
                self.approach_warp.remove(id);
            }
        }
    }

    /// The crafts `player` may give `units` a scripted order, ascending.
    fn scripted_takers(&self, player: PlayerId, units: &[EntityId]) -> Vec<EntityId> {
        let mut ids: Vec<EntityId> = units
            .iter()
            .copied()
            .filter(|&id| {
                !self.production.is_welding(id)
                    && self.unit(id).is_some_and(|u| u.orderable() && (!player.is_player() || u.owner == player) && u.hull[0] > Fixed::ZERO)
            })
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// Take a scripted order (see the module docs); false when no craft could.
    pub(crate) fn apply_scripted(&mut self, player: PlayerId, order: &Order) -> bool {
        let units = match order {
            Order::Patrol { units, .. }
            | Order::SearchAndDestroy { units }
            | Order::Scout { units }
            | Order::AttackAny { units }
            | Order::AttackPoint { units, .. }
            | Order::MoveWarp { units, .. } => units,
            _ => return false,
        };
        let ids = self.scripted_takers(player, units);
        if ids.is_empty() || matches!(order, Order::Patrol { points, .. } if points.is_empty()) {
            return false;
        }
        // Like every order that replaces what a craft is doing.
        self.weapons.stop_attack(&self.units, player, &ids);
        self.economy.cancel(&self.units, player, &ids);
        if let Some(db) = self.content.clone() {
            self.production.cancel(&db, &self.units, &mut self.economy, player, &ids);
        }
        self.clear_scripted(player, &ids);
        for &id in &ids {
            self.watchdog.remove(id);
            self.movement.clear_waypoints(id);
        }
        match order {
            Order::Patrol { points, .. } => {
                if points.len() < 2 {
                    // A one-point path is a plain move there: the order has
                    // nothing to loop over.
                    let Some(&to) = points.first() else { return false };
                    for &id in &ids {
                        self.set_target(id, Some(to));
                    }
                } else {
                    for &id in &ids {
                        self.set_target(id, Some(points[0]));
                        self.movement.set_waypoints(id, vec![points[1]]);
                        self.scripted.rows.insert(id, Job::Patrol { points: points.clone(), leg: 0 });
                    }
                }
            }
            Order::SearchAndDestroy { .. } => {
                for &id in &ids {
                    self.set_target(id, None);
                    self.scripted.rows.insert(id, Job::SearchAndDestroy);
                }
            }
            Order::Scout { .. } => {
                for &id in &ids {
                    let start = self.unit(id).map_or(Vec3::ZERO, |u| u.pos);
                    self.set_target(id, None);
                    self.scripted.rows.insert(id, Job::Scout { start, radius: Fixed::from_int(CELL), dest: None, explored: Vec::new(), cloaked: false });
                }
            }
            Order::AttackAny { .. } => {
                for &id in &ids {
                    self.set_target(id, None);
                    self.scripted.rows.insert(id, Job::AttackAny);
                }
            }
            Order::AttackPoint { at, .. } => {
                for &id in &ids {
                    self.set_target(id, Some(*at));
                    self.scripted.rows.insert(id, Job::AttackPoint { at: *at });
                }
            }
            Order::MoveWarp { to, queue, .. } => {
                for &id in &ids {
                    self.set_target(id, Some(*to));
                    let _ = queue; // a queued warp move replaces the course, as the plain one does
                    self.scripted.rows.insert(id, Job::Warp);
                }
            }
            _ => return false,
        }
        true
    }

    fn random_unit(&mut self) -> Fixed {
        let r = self.rng.next_u64();
        Fixed::from_int(((r >> 40) & 0xFFFF) as i32) / Fixed::from_int(65536)
    }

    /// A random point of the map (the unit's height), or `None` with no map bounds.
    fn random_map_point(&mut self, y: Fixed) -> Option<Vec3> {
        let (lo, hi) = self.bounds?;
        let (a, b) = (self.random_unit(), self.random_unit());
        Some(Vec3 { x: lo.x + (hi.x - lo.x) * a, y, z: lo.z + (hi.z - lo.z) * b })
    }

    /// The scripted orders' per-tick work: choose targets and destinations,
    /// advance patrols, end the orders that are done. Runs before the attack
    /// approach, so an attack set here steers in the same tick.
    pub(crate) fn step_scripted(&mut self) {
        if self.scripted.is_empty() {
            return;
        }
        let Some(db) = self.content.clone() else { return };
        let (players, rel) = (self.players.clone(), self.relations.clone());
        let dip = crate::relations::Diplomacy::new(&players, &rel);
        let seats = !self.players.is_empty();
        let visibility = if seats { crate::sensors::compute(&self.units, Some(&db), dip) } else { Vec::new() };
        let ids: Vec<EntityId> = self.scripted.rows.iter().map(|(id, _)| id).collect();
        for id in ids {
            let Some(u) = self.units.get(id) else {
                self.scripted.remove(id);
                continue;
            };
            let Some(job) = self.scripted.rows.get(id).cloned() else { continue };
            let (pos, owner, has_target) = (u.pos, u.owner, u.target.is_some());
            let idle_course = self.movement.course(id).is_none_or(|c| c.waypoints.is_empty());
            match job {
                Job::Patrol { points, leg } => {
                    let n = points.len();
                    let attacking = self.weapons.attack_of(id).is_some();
                    if !has_target && idle_course && !attacking {
                        // Starting, or the leg was ended under it: fly the current point again.
                        self.set_target(id, Some(points[leg]));
                        self.movement.set_waypoints(id, vec![points[(leg + 1) % n]]);
                    } else if idle_course && has_target && !attacking {
                        // The queued point became the target: queue the one after it.
                        let leg = (leg + 1) % n;
                        self.movement.set_waypoints(id, vec![points[(leg + 1) % n]]);
                        self.scripted.rows.insert(id, Job::Patrol { points, leg });
                    }
                }
                Job::Warp => {
                    // Over when no leg is left (queued legs keep the warp move going).
                    if !has_target && idle_course {
                        self.scripted.remove(id);
                        self.approach_warp.remove(id);
                    }
                }
                Job::AttackPoint { at } => {
                    let reach = db.unit(u.def).and_then(|d| approach::attack_range(&db, d)).unwrap_or(Fixed::ZERO);
                    if flat(pos, at) <= (reach - Fixed::from_int(50)).max(Fixed::ZERO) || !has_target {
                        self.set_target(id, None);
                        self.scripted.remove(id);
                    }
                }
                Job::SearchAndDestroy | Job::AttackAny => {
                    let Some(def) = db.unit(u.def) else { continue };
                    let reach = def.sensor.max(approach::attack_range(&db, def).unwrap_or(Fixed::ZERO));
                    if !dip.is_seat(owner) {
                        continue;
                    }
                    let seen = seats.then(|| visibility.iter().find(|(p, _)| *p == owner).map_or(&[][..], |(_, ids)| ids.as_slice()));
                    let ok = |t: &Entity| targeting::candidate(&db, u, t, dip, &self.perception, seen, Some(&self.switches));
                    let current = self.weapons.attack_of(id).and_then(|t| self.units.get(t)).filter(|t| ok(t) && flat(pos, t.pos) <= reach).map(|t| t.id);
                    let pick = current.or_else(|| targeting::best(&db, u, reach, self.units.iter().filter(|t| ok(t))).map(|t| t.id));
                    match pick {
                        Some(t) => self.weapons.set_attack(id, t),
                        None => {
                            self.weapons.stop_attack(&self.units, PlayerId::SCRIPT, &[id]);
                            if matches!(job, Job::AttackAny) {
                                self.set_target(id, None);
                                self.scripted.remove(id);
                            } else if !has_target && let Some(p) = self.random_map_point(pos.y) {
                                self.set_target(id, Some(p));
                            }
                        }
                    }
                }
                Job::Scout { start, mut radius, dest, mut explored, cloaked } => {
                    if !cloaked && let Some(db) = self.content.clone() {
                        let _ = self.specials.order_cloak(&db, &self.units, owner, &[id], true);
                    }
                    let size = i64::from(CELL) << 32;
                    let cell = |p: Vec3| (p.x.to_bits().div_euclid(size) as i32, p.z.to_bits().div_euclid(size) as i32);
                    let arrived = dest.is_some_and(|d| flat(pos, d) <= Fixed::from_int(100)) || (dest.is_some() && !has_target);
                    let mut dest = dest;
                    if arrived {
                        let c = cell(pos);
                        if let Err(i) = explored.binary_search(&c) {
                            explored.insert(i, c);
                        }
                        dest = None;
                    }
                    if dest.is_none() {
                        let cap = self.bounds.map_or(Fixed::from_int(CELL), |(lo, hi)| {
                            let (w, d) = (hi.x - lo.x, hi.z - lo.z);
                            (w * w + d * d).sqrt()
                        });
                        'search: loop {
                            for _ in 0..30 {
                                let (a, b) = (self.random_unit(), self.random_unit());
                                let two = Fixed::from_int(2);
                                let p = Vec3 { x: start.x + (a * two - Fixed::ONE) * radius, y: pos.y, z: start.z + (b * two - Fixed::ONE) * radius };
                                let inside = self.bounds.is_none_or(|(lo, hi)| p.x >= lo.x && p.x <= hi.x && p.z >= lo.z && p.z <= hi.z);
                                if inside && explored.binary_search(&cell(p)).is_err() {
                                    dest = Some(p);
                                    break 'search;
                                }
                            }
                            if radius >= cap {
                                dest = self.random_map_point(pos.y);
                                break;
                            }
                            radius = radius + Fixed::from_int(CELL);
                        }
                    }
                    self.set_target(id, dest);
                    self.scripted.rows.insert(id, Job::Scout { start, radius, dest, explored, cloaked: true });
                }
            }
        }
    }

    /// Plans for the warp moves of this tick (ascending id).
    pub(crate) fn steer_warp_moves(&mut self) -> Vec<(EntityId, Plan)> {
        let mut plans = Vec::new();
        if self.scripted.is_empty() {
            return plans;
        }
        let ids: Vec<EntityId> = self.scripted.rows.iter().filter(|(_, j)| matches!(j, Job::Warp)).map(|(id, _)| id).collect();
        for id in ids {
            let Some(u) = self.units.get(id) else { continue };
            let Some(goal) = u.target.filter(|_| u.kind == EntityKind::Craft && u.mobile()) else { continue };
            let (vi, vw) = (u.stats.max_speed, u.stats.warp_speed);
            let (gx, gz) = (goal.x - u.pos.x, goal.z - u.pos.z);
            let to_goal = (gx * gx + gz * gz).sqrt();
            let aligned = to_goal > Fixed::ZERO && (u.heading.0 * gx + u.heading.1 * gz) / to_goal > Fixed::from_f64(0.99);
            let phase = self.approach_warp.phase(id);
            let was = phase == Some(approach::IN_WARP);
            let can = vw > vi && aligned && to_goal >= approach::warp_lead(vw, vi, db_radius(self, u));
            let warp = if was { can } else { can && to_goal > too_close_to_warp() && u.speed > Fixed::from_f64(0.9) * vi };
            let mut plan = Plan { warp, ..Plan::default() };
            if warp {
                self.approach_warp.set_phase(id, approach::IN_WARP);
            } else if phase.is_some() && u.speed > vi {
                plan.warp_out = true;
                self.approach_warp.set_phase(id, approach::WARP_OUT);
            } else {
                self.approach_warp.remove(id);
            }
            plans.push((id, plan));
        }
        plans
    }
}

fn db_radius(sim: &Sim, u: &Entity) -> Fixed {
    sim.content.as_deref().and_then(|db| db.unit(u.def)).map_or(Fixed::ZERO, |d| d.radius)
}
