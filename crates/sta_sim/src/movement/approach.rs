//! Attack approach (SIM-36): how a craft with an attack target closes on it.
//! Implements D5, D9.
//!
//! Distances are surface to surface: `D` = centre distance minus both
//! bounding radii. `W` is the attacker's weapon range: the longest range of
//! its regular weapons (no specials, point defence or resource beams; which
//! weapons the engine takes is not established, the ships measured have one
//! range), plus the target's radius when the target is a craft.
//!
//! - **Closing in.** The attacker flies toward a point `R_target + R_own + 50`
//!   short of the target, on its own side, while `D > W - buffer - R_target`,
//!   with `buffer = min(0.2 W + 5, 50)` (stock) or, under `attack_buffer_roots`,
//!   `0.2 W + 5` below 100 and 80 from there (the Roots engine). Then it brakes
//!   where it is and turns to face the target. The test runs every tick, so a
//!   target that moves away restarts the approach. Read from the executable
//!   (both engines), static; the stop distances (fdestroy 348 stock, 312 Roots;
//!   Galaxy 612 stock, 581 Roots, centre to centre) measured. STA64's weapons
//!   measure range centre to centre, so the approach also closes until the
//!   target is within the attack range by that measure.
//! - **Speed.** When the target is slower than `combatSpeed` and nearer than
//!   `cfg_MIN_MOVE_WARP_DISTANCE` (`RTS_CFG.h`, typed in `Rules::approach`),
//!   the approach is capped at `combatSpeed`: at once under
//!   `attack_combat_speed` (stock), only once in weapon range otherwise (the
//!   Roots engine, which closes at `impulseSpeed`). Read from the executable,
//!   measured.
//! - **Warp.** From the warp distance out, a craft whose `warpSpeed` exceeds
//!   its `impulseSpeed` goes to warp once it faces the goal (cosine above 0.99)
//!   at more than 0.9 of impulse speed. The combat-speed cap ends it (stock:
//!   at the warp distance), else the engine's lead rule near the goal (Roots:
//!   about 450 to 500 from the target). Stock (1100) and STA2 Classic on Roots
//!   (1100) warp; Roots and Fleet Ops (999999) never do. Measured on stock and
//!   on Roots + Classic (distances 700 to 3800); the entry conditions and the
//!   lead rule read from the executable, static.
//! - **The shape of warp** (`warp_shape`, #1102 part 2; stock Armada II): no speed ramp. The
//!   planners here only decide whether a craft warps; the movement step
//!   (`kinematics::step_shaped`) holds its speed over a short ramp, shifts it forward and puts
//!   it at warp speed, and mirrors that on the way out. The heading is tested to enter warp
//!   and not to stay at it: a craft at warp steers with its normal controller and follows a
//!   new line at warp. Measured, repeated (the turn at warp once). Without the switch speed
//!   ramps both ways and a craft at warp flies straight and brakes to turn.
//! - Not modelled: the engine's AI fleets (a fleet member at low or medium
//!   movement autonomy may not close in or warp; STA64 has no AI fleets yet),
//!   the engine's 10 Hz AI tick.

use serde::{Deserialize, Serialize};
use sta_content::{ContentDb, UnitDef};
use sta_core::{EntityId, Fixed, TICK_HZ, Tick, Vec3};

use crate::kinematics::Plan;
use crate::movement::gravity::GravityWells;
use crate::world::{Entity, EntityKind, Fnv, Table};
use crate::Sim;

/// Phase of a craft in [`Warp`].
pub(crate) const IN_WARP: u8 = 1;
/// Dropping out: speed falls straight to the approach's limit.
pub(crate) const WARP_OUT: u8 = 2;
/// Entering warp under `warp_shape`: on its ramp, still at the speed it had.
pub(crate) const ENTERING: u8 = 3;

/// Stretch factor: the gain on the ratio of warp to impulse speed (read from the executable, static).
fn stretch_gain() -> Fixed {
    Fixed::from_f64(0.6)
}
/// Stretch factor: the offset added to the impulse speed (read from the executable, static).
fn stretch_offset() -> Fixed {
    Fixed::from_f64(0.1)
}
/// Stretch factor: its lowest value (read from the executable, static; not measured).
fn stretch_min() -> Fixed {
    Fixed::from_f64(1.5)
}
/// Stretch factor: its highest value (read from the executable, static; measured).
fn stretch_max() -> Fixed {
    Fixed::from_int(6)
}
/// Warp ramp: its shortest length in seconds (read from the executable, static; measured).
fn ramp_min() -> Fixed {
    Fixed::from_f64(0.1)
}
/// Warp ramp: its longest length in seconds (read from the executable, static; not measured).
fn ramp_max() -> Fixed {
    Fixed::from_int(3)
}

/// The stretch factor of a craft whose warp speed is `vw` and impulse speed `vi` (both as this
/// tick's speed ceiling leaves them): the gain times `vw / (vi + offset)`, within its bounds,
/// and the lowest value when warp is no faster than impulse. Read from the executable, static.
pub(crate) fn stretch(vw: Fixed, vi: Fixed) -> Fixed {
    if vw <= vi {
        return stretch_min();
    }
    (stretch_gain() * vw / (vi + stretch_offset())).max(stretch_min()).min(stretch_max())
}

/// The length of a warp ramp in ticks: `2 R (S - 1) / dv` seconds for a craft of bounding
/// radius `R`, stretch factor `S` and speed change `dv`, within its bounds. Read from the
/// executable, static. INTERIM: rounded up to whole ticks of STA64; the original's time step
/// differs.
pub(crate) fn ramp_ticks(radius: Fixed, s: Fixed, dv: Fixed) -> u32 {
    let span = Fixed::from_int(2) * radius * (s - Fixed::ONE);
    // the quotient is only formed where it lies below the upper bound (a tiny `dv` would overflow)
    let secs = if dv <= Fixed::ZERO || dv * ramp_max() <= span { ramp_max() } else { (span / dv).max(ramp_min()) };
    whole_ticks(secs)
}

/// `secs` in ticks, rounded up, at least one.
fn whole_ticks(secs: Fixed) -> u32 {
    let bits = (secs * Fixed::from_int(TICK_HZ as i32)).to_bits().max(0);
    u32::try_from((bits + Fixed::ONE.to_bits() - 1) >> 32).unwrap_or(u32::MAX).max(1)
}

/// The longest ramp [`ramp_ticks`] gives.
pub(crate) fn longest_ramp() -> u32 {
    whole_ticks(ramp_max())
}

/// The ramp of a craft entering or leaving warp under `warp_shape` (#1102 part 2), fixed on
/// the tick it starts: that tick, its length in ticks, and the whole extra forward shift
/// `(S - 1) R` the craft is given over one half of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Ramp {
    pub start: Tick,
    pub ticks: u32,
    pub shift: Fixed,
}

impl Ramp {
    /// The ramp of a craft that starts to enter warp on tick `now` at speed `v0` (at most its
    /// impulse speed).
    pub(crate) fn entering(now: Tick, radius: Fixed, vw: Fixed, vi: Fixed, v0: Fixed) -> Ramp {
        let s = stretch(vw, vi);
        Ramp { start: now, ticks: ramp_ticks(radius, s, vw - v0), shift: (s - Fixed::ONE) * radius }
    }

    /// The ramp of a craft that starts to leave warp on tick `now` at speed `v`.
    pub(crate) fn leaving(now: Tick, radius: Fixed, vw: Fixed, vi: Fixed, v: Fixed) -> Ramp {
        let s = stretch(vw, vi);
        Ramp { start: now, ticks: ramp_ticks(radius, s, v.max(vw) - vi), shift: (s - Fixed::ONE) * radius }
    }

    /// The extra forward shift of tick `now`: entering, the whole shift is spread evenly over
    /// the later half of the ramp; leaving, over the first half. Measured (the halves and the
    /// total), repeated.
    ///
    /// INTERIM: leaving, the measurement shows one step of the original at impulse speed
    /// before the shifted ones. That step is shorter than a tick of STA64, so it cannot be put
    /// here without inventing a part of a tick: the shift starts with the exit's first tick.
    pub(crate) fn shift_at(&self, now: Tick, entering: bool) -> Fixed {
        let len = Fixed::from_int(self.ticks.max(1) as i32);
        let at = |k: u64| Fixed::from_int(k.min(u64::from(self.ticks)) as i32) / len;
        let done = now.saturating_sub(self.start);
        let (a, b) = (at(done), at(done + 1));
        let half = Fixed::ONE / Fixed::from_int(2);
        let share = if entering { b.max(half) - a.max(half) } else { b.min(half) - a.min(half) };
        self.shift * Fixed::from_int(2) * share
    }

    /// Whether this ramp is one the rules can have made for a craft of bounding radius
    /// `radius` by tick `now`: a length within the longest there is, a shift between nothing
    /// and what the highest stretch factor gives that radius, a start not in the future. A
    /// stored ramp is checked with this when a snapshot is loaded, so a crafted one cannot make
    /// an entry that never ends or move a craft by an arbitrary amount.
    pub(crate) fn plausible(&self, now: Tick, radius: Fixed) -> bool {
        (1..=longest_ramp()).contains(&self.ticks)
            && self.shift >= Fixed::ZERO
            && self.shift <= (stretch_max() - Fixed::ONE) * radius.max(Fixed::ZERO)
            && self.start <= now
    }

    /// Whether tick `now` is the last of the ramp (or lies behind it).
    pub(crate) fn ends_at(&self, now: Tick) -> bool {
        now.saturating_sub(self.start) + 1 >= u64::from(self.ticks)
    }
}

/// Which crafts are in warp, entering it or dropping out of it, ascending id; under
/// `warp_shape` with the ramp of each one that has entered or is leaving.
#[derive(Debug, Clone, Default)]
pub struct Warp {
    on: Table<u8>,
    /// Only under `warp_shape`, only for ids in `on`: the entry ramp of a craft entering or at
    /// warp (kept at warp for the view), the exit ramp of one that is leaving.
    ramps: Table<Ramp>,
}

impl Warp {
    pub(crate) fn is_empty(&self) -> bool {
        self.on.is_empty()
    }

    /// In warp or entering it (not dropping out).
    pub fn contains(&self, id: EntityId) -> bool {
        matches!(self.on.get(id), Some(&IN_WARP | &ENTERING))
    }

    pub(crate) fn set_phase(&mut self, id: EntityId, phase: u8) {
        self.on.insert(id, phase);
    }

    pub(crate) fn phase(&self, id: EntityId) -> Option<u8> {
        self.on.get(id).copied()
    }

    pub(crate) fn ramp(&self, id: EntityId) -> Option<Ramp> {
        self.ramps.get(id).copied()
    }

    /// Gives the craft `id`, which has a phase, its ramp.
    pub(crate) fn set_ramp(&mut self, id: EntityId, ramp: Ramp) {
        if self.on.get(id).is_some() {
            self.ramps.insert(id, ramp);
        }
    }

    pub(crate) fn remove(&mut self, id: EntityId) {
        self.on.remove(id);
        self.ramps.remove(id);
    }

    /// A finished or replaced order can still be coasting out of warp. Under `warp_shape`: a
    /// craft at warp starts to leave (the movement step makes its exit ramp); one still entering
    /// leaves on the ramp it has (it keeps the ramp's start), and one that had not begun is
    /// simply out.
    pub(crate) fn end(&mut self, id: EntityId) {
        match self.phase(id) {
            Some(IN_WARP) => {
                self.set_phase(id, WARP_OUT);
                self.ramps.remove(id);
            }
            Some(ENTERING) if self.ramps.get(id).is_some() => self.set_phase(id, WARP_OUT),
            Some(ENTERING) => self.remove(id),
            _ => {}
        }
    }

    /// A warp nothing asks for any more: the craft leaves it when it is still `fast` (above its
    /// impulse speed) or, under `warp_shape`, has a ramp; a craft that is leaving goes on
    /// leaving; anything else is forgotten.
    pub(crate) fn stop(&mut self, id: EntityId, fast: bool) {
        let ramp = self.ramps.get(id).is_some();
        match self.phase(id) {
            Some(WARP_OUT) if fast || ramp => {}
            Some(IN_WARP | ENTERING) if fast || ramp => self.end(id),
            _ => self.remove(id),
        }
    }

    /// Under `warp_shape`: what the planner decided for the craft `id` this tick. The movement
    /// step runs the ramps and moves the phases on.
    pub(crate) fn decide(&mut self, id: EntityId, warp: bool) {
        match (warp, self.phase(id)) {
            (true, Some(IN_WARP | ENTERING)) => {}
            // a leave that has nothing left to do does not stand in the way
            (true, _) if self.ramps.get(id).is_none() => {
                self.on.insert(id, ENTERING);
            }
            (true, _) => {}
            (false, _) => self.end(id),
        }
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        // The earlier form while nothing of `warp_shape` is stored, so a game without it keeps
        // its digests.
        if self.ramps.is_empty() && !self.on.iter().any(|(_, p)| *p == ENTERING) {
            h.str("attack-warp-v1");
            h.u64(self.on.len() as u64);
            for (id, phase) in self.on.iter() {
                h.u64(id.0 as u64);
                h.u64(*phase as u64);
            }
            return;
        }
        h.str("attack-warp-v2");
        h.u64(self.on.len() as u64);
        for (id, phase) in self.on.iter() {
            h.u64(id.0 as u64);
            h.u64(*phase as u64);
            match self.ramps.get(id) {
                Some(r) => {
                    h.u64(1);
                    h.u64(r.start);
                    h.u64(u64::from(r.ticks));
                    h.u64(r.shift.to_bits() as u64);
                }
                None => h.u64(0),
            }
        }
    }

    pub(crate) fn to_rec(&self) -> Vec<(EntityId, u8)> {
        self.on.iter().map(|(id, p)| (id, *p)).collect()
    }

    pub(crate) fn ramps_rec(&self) -> Vec<(EntityId, Ramp)> {
        self.ramps.iter().map(|(id, r)| (id, *r)).collect()
    }

    /// Whether stored ramps fit stored phase rows: ascending ids, each with a length and with
    /// a phase row of its own.
    pub(crate) fn ramps_fit(rows: &[(EntityId, u8)], ramps: &[(EntityId, Ramp)]) -> bool {
        ramps.windows(2).all(|w| w[0].0 < w[1].0)
            && ramps.iter().all(|(id, r)| r.ticks > 0 && rows.iter().any(|(row, _)| row == id))
    }

    /// `None` when the ids do not ascend, a phase is unknown, or the ramps do not fit
    /// ([`Warp::ramps_fit`]).
    pub(crate) fn from_rec(rows: Vec<(EntityId, u8)>, ramps: Vec<(EntityId, Ramp)>) -> Option<Warp> {
        if rows.iter().any(|(_, p)| !matches!(*p, IN_WARP | WARP_OUT | ENTERING)) || !Self::ramps_fit(&rows, &ramps) {
            return None;
        }
        Some(Warp { on: Table::from_rows(rows)?, ramps: Table::from_rows(ramps)? })
    }
}

/// The attack range of a unit: the longest weapon that attacks units (not a tool), if any.
pub(crate) fn attack_range(db: &ContentDb, def: &UnitDef) -> Option<Fixed> {
    def.hardpoints
        .iter()
        .filter_map(|hp| db.weapon(hp.weapon.key))
        .filter(|w| crate::combat::weapons::attacks_units(w) && w.resource.is_none())
        .map(|w| w.range)
        .max()
}

/// The class key `canAttack` (default 1).
pub(crate) fn class_may_attack(def: &UnitDef) -> bool {
    def.stats.can_attack.unwrap_or(true)
}

/// May `def` take up a fight by itself (assist, answering an attacker)? Automatic
/// pursuit and assist require the class key `canAttack` (default 1) and a weapon
/// that attacks units and is not marked by its `special` key as a special weapon
/// (read from the executable, static; also in Roots). A craft
/// that fails stays where it is; whether it flees is not measured, not modelled.
pub(crate) fn engages_alone(db: &ContentDb, def: &UnitDef) -> bool {
    class_may_attack(def)
        && def.hardpoints.iter().filter_map(|hp| db.weapon(hp.weapon.key)).any(|w| {
            crate::combat::weapons::attacks_units(w) && w.resource.is_none()
        })
}

/// Gap kept to the target's surface by the goal point (`sk_ATTACK_BUFFER`,
/// read from the executable, static).
fn goal_gap() -> Fixed {
    Fixed::from_int(50)
}

/// The engine's warp lead: drop out of warp when the goal is nearer than
/// `(vw + vi) / 2 + 2 (k - 1) vi / (vw - vi) R` with `k` the stretch factor ([`stretch`]; read
/// from the executable, static).
pub(crate) fn warp_lead(vw: Fixed, vi: Fixed, r: Fixed) -> Fixed {
    let k = stretch(vw, vi);
    (vw + vi) / Fixed::from_int(2) + Fixed::from_int(2) * (k - Fixed::ONE) * vi / (vw - vi) * r
}

/// The engines system's slot in `UnitDef::system_hit_points` and in a subsystem row.
const ENGINES: usize = 0;

/// The share of its hit points the engines system must still have for warp, as a fraction
/// numerator and denominator (a constant of the rule, not a data key; #1102).
pub(crate) const WARP_ENGINES_SHARE: (i128, i128) = (3, 5);

/// Whether an engines system with `hit_points` and `damage` (fixed-point bits, as a
/// subsystem row holds it) is healthy enough for warp: what is left of it, over its maximum,
/// is at or above [`WARP_ENGINES_SHARE`]; just below fails. Read from the executable, static;
/// the threshold is not measured.
///
/// STA64's own choice, the evidence not settling it: a craft whose engines system has no
/// hit points (so it cannot be damaged) is allowed.
pub(crate) fn engines_share_met(hit_points: Fixed, damage: i64) -> bool {
    let max = i128::from(hit_points.to_bits());
    if max <= 0 {
        return true;
    }
    let left = (max - i128::from(damage)).max(0);
    left * WARP_ENGINES_SHARE.1 >= max * WARP_ENGINES_SHARE.0
}

/// Whether this tick's speed ceiling leaves room for warp: the craft's impulse speed must
/// lie strictly below it. `cap` is the craft's nebula speed cap of this tick, `(impulse,
/// warp)`, the one thing in STA64 that lowers the ceiling; with none there is no ceiling.
/// (A plan's own cap blocks warp where the plan is made, and a scripted speed is the
/// craft's impulse itself, not a cap.) Read from the executable, static; not measured.
/// How a nebula's speed limit maps to the ceiling under Armada I's coefficient rule is not
/// settled; `warp_gates` is off for that profile, so the question does not arise there.
pub(crate) fn ceiling_allows_warp(impulse: Fixed, cap: Option<(Fixed, Fixed)>) -> bool {
    cap.is_none_or(|(_, ceiling)| impulse < ceiling)
}

/// Distance and alignment of the current steering aim, in the map plane.
pub(crate) fn warp_leg(unit: &Entity, goal: Vec3) -> (Fixed, bool) {
    let (gx, gz) = (goal.x - unit.pos.x, goal.z - unit.pos.z);
    let distance = (gx * gx + gz * gz).sqrt();
    let aligned = distance > Fixed::ZERO
        && (unit.heading.0 * gx + unit.heading.1 * gz) / distance > Fixed::from_f64(0.99);
    (distance, aligned)
}

impl Sim {
    /// The gates a craft's own state puts on warp this tick, for entering and for staying
    /// (#1102): its engines system is on and has its share of hit points
    /// ([`engines_share_met`]; a craft with no damage row is undamaged), and the tick's
    /// speed ceiling leaves room ([`ceiling_allows_warp`]). A craft that fails one while at
    /// warp leaves it the ordinary way, and warps again by itself once it passes.
    ///
    /// Under the feature switch `warp_gates` (the unit's, as `ContentDb::unit_features`
    /// resolves it); off, or without content, every craft passes. INTERIM, not settled:
    /// engines held by a special weapon or in stasis are not counted as offline here.
    pub(crate) fn warp_gates_pass(&self, u: &Entity, caps: &[(EntityId, (Fixed, Fixed))]) -> bool {
        let Some(db) = self.content.as_deref() else { return true };
        let def = db.unit(u.def);
        if !def.map_or(db.features(), |d| db.unit_features(d)).warp_gates {
            return true;
        }
        if u.flags.contains(crate::view::UnitFlags::ENGINES_OFF) {
            return false;
        }
        let hit_points = def.map_or(Fixed::ZERO, |d| d.system_hit_points[ENGINES]);
        let damage = self.subsystems.get(u.id).map_or(0, |row| row.damage[ENGINES]);
        let cap = caps.binary_search_by_key(&u.id, |c| c.0).ok().map(|i| caps[i].1);
        engines_share_met(hit_points, damage) && ceiling_allows_warp(u.stats.max_speed, cap)
    }

    /// Whether `u` enters, flies and leaves warp by the measured shape (#1102 part 2): the
    /// feature switch `warp_shape`, the unit's, as `ContentDb::unit_features` resolves it.
    pub(crate) fn warp_shape_on(&self, u: &Entity) -> bool {
        crate::kinematics::warp_shape(self.content.as_deref(), u)
    }

    /// The view's warp rows (#1102 part 1), ascending id. Under `warp_shape` a row is the
    /// stored ramp of its craft; otherwise it is derived from the warp record and the craft's
    /// speed. Never hashed.
    pub(crate) fn warp_rows(&self) -> Vec<crate::view::WarpRow> {
        use crate::view::{WarpPhase, WarpRow};
        self.approach_warp.to_rec().into_iter().filter_map(|(id, phase)| {
            let u = self.units.get(id)?;
            let stretch = stretch(u.stats.warp_speed, u.stats.max_speed);
            if let Some(ramp) = self.approach_warp.ramp(id) {
                let phase = match phase {
                    ENTERING => WarpPhase::Entering,
                    IN_WARP => WarpPhase::AtWarp,
                    _ => WarpPhase::Leaving,
                };
                return Some(WarpRow { id, phase, ramp_start: ramp.start, ramp_len: ramp.ticks, stretch });
            }
            if self.warp_shape_on(u) {
                // no ramp: not begun yet, or loaded from a snapshot that stored none
                let phase = if phase == WARP_OUT { WarpPhase::Leaving } else { WarpPhase::AtWarp };
                return Some(WarpRow { id, phase, ramp_start: self.tick, ramp_len: 0, stretch });
            }
            let leaving = phase == WARP_OUT;
            let (len, done) = crate::kinematics::warp_ramp(&u.stats, u.speed, !leaving);
            let (phase, started) = if leaving {
                (WarpPhase::Leaving, done)
            } else if u.speed < u.stats.warp_speed {
                (WarpPhase::Entering, done)
            } else {
                (WarpPhase::AtWarp, len)
            };
            Some(WarpRow { id, phase, ramp_start: self.tick.saturating_sub(u64::from(started)), ramp_len: len, stretch })
        }).collect()
    }

    /// Set the course of every craft with an attack target and say how it
    /// steps this tick (ascending id). Runs before the movement step.
    /// `caps`: the nebula speed caps of this tick (ascending id).
    pub(crate) fn steer_attackers(&mut self, gravity: &GravityWells, caps: &[(EntityId, (Fixed, Fixed))]) -> Vec<(EntityId, Plan)> {
        let Some(db) = self.content.clone() else { return Vec::new() };
        let f = db.features();
        let min_warp = db.rules().approach().min_move_warp_distance;
        let mut plans = Vec::new();
        let attacks = self.weapons.attacks();
        // What the attacker's team sees. A target it cannot see (out of sensor
        // range, or cloaked to it) is flown to where the craft was last steered,
        // at impulse and without warp; the order stands and the approach resumes
        // once the target is seen again, so nothing homes on what is not seen.
        // (The Roots + Classic attack on a target 3800 away on mp02eye ran at
        // impulse to about 1400, then warped: measured once; this reading of it
        // is not established.) Seatless sims (bench, legacy) are omniscient, as
        // their views are.
        let seats = !self.players.is_empty();
        let seen = if attacks.is_empty() || !seats { Vec::new() } else { crate::sensors::compute(&self.units, Some(&db), crate::relations::Diplomacy::new(&self.players, &self.relations), &self.specials, self.tick) };
        let dip = crate::relations::Diplomacy::new(&self.players, &self.relations);
        for (id, tid) in attacks {
            let (Some(u), Some(t)) = (self.units.get(id), self.units.get(tid)) else { continue };
            let cloaked = !seats && t.flags.contains(crate::view::UnitFlags::CLOAKED) && !dip.confirmed_ally(t.owner, u.owner);
            let visible = !seats || seen.iter().find(|(p, _)| *p == u.owner).is_some_and(|(_, ids)| ids.binary_search(&tid).is_ok());
            if cloaked || !visible {
                let last = u.target.or_else(|| (!self.directors_cut).then_some(t.pos));
                let craft = u.kind == EntityKind::Craft && u.mobile();
                self.approach_warp.stop(id, craft && u.speed > u.stats.max_speed);
                if craft {
                    if let Some(u) = self.units.get_mut(id) {
                        u.target = last;
                    }
                    plans.push((id, Plan::default()));
                }
                continue;
            }
            if u.kind != EntityKind::Craft || !u.mobile() || t.hull[1] <= Fixed::ZERO || self.production.is_welding(id) || self.production.is_deploying(id) || self.economy.is_mining(id) {
                continue;
            }
            let Some(def) = db.unit(u.def) else { continue };
            let Some(range) = attack_range(&db, def) else { continue };
            let r_own = def.radius;
            let r_t = db.unit(t.def).map_or(Fixed::ZERO, |d| d.radius);
            let (dx, dz) = (u.pos.x - t.pos.x, u.pos.z - t.pos.z);
            let centre = (dx * dx + dz * dz).sqrt();
            if self.directors_cut && centre <= range
                && let Some(p) = crate::kinematics::a1::combat_params(&db, u.def)
            {
                let aim = super::directors_cut::pursuit_point(
                    u, t, centre, r_own + r_t, p.play.turn_radius, db.rules().a1_dogfight(),
                );
                self.approach_warp.remove(id);
                self.units.get_mut(id).expect("attacker exists").target = None;
                plans.push((id, Plan { dogfight: Some(aim), ..Plan::default() }));
                continue;
            }
            let d = (centre - r_own - r_t).max(Fixed::ZERO);
            let w = range + if t.kind == EntityKind::Craft { r_t } else { Fixed::ZERO };
            let base = w / Fixed::from_int(5) + Fixed::from_int(5);
            let buffer = if f.attack_buffer_roots {
                if base >= Fixed::from_int(100) { Fixed::from_int(80) } else { base }
            } else {
                base.min(goal_gap())
            };
            let mut plan = Plan::default();
            let closer = d > w - buffer - r_t || centre > range;
            let goal = if closer {
                let off = r_t + r_own + goal_gap();
                let (ux, uz) = if centre == Fixed::ZERO { (Fixed::ZERO, Fixed::ZERO) } else { (dx / centre, dz / centre) };
                Some(Vec3 { x: t.pos.x + ux * off, y: t.pos.y, z: t.pos.z + uz * off })
            } else {
                plan.face = Some(t.pos);
                None
            };
            let (vi, vw, combat) = (u.stats.max_speed, u.stats.warp_speed, u.stats.combat_speed);
            let far = d >= min_warp;
            let phase = self.approach_warp.phase(id);
            let shape = self.warp_shape_on(u);
            let mut warp = self.approach_warp.contains(id);
            if let Some(g) = goal {
                let (to_goal, aligned) = warp_leg(u, g);
                if !far && t.speed < combat && (f.attack_combat_speed || d < w) {
                    plan.cap = Some(combat);
                }
                // `warp_shape`: the heading is tested to enter warp, not to stay at it (read
                // from the executable, static; a craft at warp follows a new line at warp: measured).
                let on_line = aligned || (shape && warp);
                let can = self.warp_allowed() && vw > vi && vi > crate::speed_override::HELD_SPEED && plan.cap.is_none() && on_line
                    && to_goal >= warp_lead(vw, vi, r_own) && !gravity.suppresses(u.pos, r_own) && self.warp_gates_pass(u, caps);
                let still_braking = phase == Some(WARP_OUT) && (u.speed > vi || self.approach_warp.ramp(id).is_some());
                warp = if warp { can } else { can && far && u.speed > Fixed::from_f64(0.9) * vi && !still_braking };
                // LUA-31: a keyed craft's warp speed below impulse caps the aligned far leg.
                if plan.cap.is_none() && far && aligned && u.stats.too_close_for_warp > Fixed::ZERO && to_goal > u.stats.too_close_for_warp {
                    plan.cap = crate::warp_override::slow_leg(vw, vi);
                }
            } else {
                warp = false;
            }
            plan.warp = warp;
            plan.wants_warp = goal.is_some() && far && vw > vi;
            let limit = plan.cap.unwrap_or(vi);
            if shape {
                self.approach_warp.decide(id, warp);
            } else if warp {
                self.approach_warp.on.insert(id, IN_WARP);
            } else if phase.is_some() && u.speed > limit {
                // Out of warp: straight down to the limit (combat speed on stock).
                plan.warp_out = true;
                self.approach_warp.on.insert(id, WARP_OUT);
            } else {
                self.approach_warp.remove(id);
            }
            if let Some(u) = self.units.get_mut(id) {
                u.target = goal;
            }
            plans.push((id, plan));
        }
        // Crafts whose attack ended leave warp.
        let stale: Vec<EntityId> = self.approach_warp.on.iter().map(|(id, _)| id).filter(|id| plans.binary_search_by_key(id, |p| p.0).is_err() && !self.scripted.is_warp(*id)).collect();
        for id in stale {
            let fast = self.units.get(id).is_some_and(|unit| unit.mobile() && unit.speed > unit.stats.max_speed);
            self.approach_warp.stop(id, fast);
        }
        plans
    }

    /// Navigation may supply a different aim after attack planning. Brake
    /// straight before following a detour that requires a turn.
    pub(crate) fn brake_for_attack_detours(&mut self, plans: &mut [(EntityId, Plan)]) {
        for (id, plan) in plans.iter_mut().filter(|(_, plan)| plan.warp) {
            let Some(unit) = self.units.get(*id) else { continue };
            if unit.target.is_some_and(|goal| warp_leg(unit, goal).1) {
                continue;
            }
            // `warp_shape`: a craft that was at warp before this tick keeps warp and steers
            // round (measured for a new warp order at warp; an attack detour is taken to be the
            // same). One that would only now enter waits until it is on the line.
            if self.warp_shape_on(unit) && self.approach_warp.ramp(*id).is_some() {
                continue;
            }
            plan.warp = false;
            if unit.speed > unit.stats.max_speed {
                plan.warp_out = true;
                self.approach_warp.end(*id);
            } else {
                self.approach_warp.remove(*id);
            }
        }
    }
}
