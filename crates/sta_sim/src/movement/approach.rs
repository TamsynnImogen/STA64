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
//!   short of the target, on its own side, while `D > W - buffer`, with
//!   `buffer = min(W / 5 + 5, 50) + R_target` (stock, `attack_range_buffer`);
//!   without the buffer it closes until `D < W` (Roots). Then it brakes where
//!   it is and turns to face the target. The test runs every tick, so a target
//!   that moves away restarts the approach. Read from the executable, static;
//!   the stop distances (fdestroy 348 stock, 312 Roots; Galaxy 612 stock, 581
//!   Roots, centre to centre) measured.
//! - **Warp.** While `D >= cfg_MIN_MOVE_WARP_DISTANCE` (`RTS_CFG.h`, typed in
//!   `Rules::approach`) a craft whose `warpSpeed` exceeds its `impulseSpeed`
//!   goes to warp once it faces the goal (cosine above 0.99) at more than 0.9
//!   of impulse speed. It drops out near the goal (the engine's lead rule
//!   below) and, under `attack_warp_out_at_distance` (stock), as soon as `D`
//!   falls under the warp distance. Stock (1100) and STA2 Classic on Roots
//!   (1100) warp; Roots and Fleet Ops (999999) never do. Measured on stock
//!   and on Roots + Classic (distances 700 to 3800); the entry conditions and
//!   the lead rule read from the executable, static.
//! - **Speed below the warp distance.** Under `attack_combat_speed` (stock)
//!   the approach runs at `combatSpeed` when the target is slower than that;
//!   otherwise at `impulseSpeed` (Roots). Measured.
//! - Not modelled: the engine's AI fleets (a fleet member at low or medium
//!   movement autonomy may not close in or warp; STA64 has no AI fleets yet),
//!   dewarp zones of planets and black holes, the engine's 10 Hz AI tick.

use sta_content::{ContentDb, UnitDef, WeaponKind};
use sta_core::{EntityId, Fixed, Vec3};

use crate::kinematics::Plan;
use crate::world::{EntityKind, Fnv, Table};
use crate::Sim;

/// Phase of a craft in [`Warp`].
pub(crate) const IN_WARP: u8 = 1;
/// Dropping out: speed falls straight to the approach's limit.
pub(crate) const WARP_OUT: u8 = 2;

/// Which crafts are in warp, or dropping out of it, on an attack approach,
/// ascending id.
#[derive(Debug, Clone, Default)]
pub struct Warp {
    on: Table<u8>,
}

impl Warp {
    pub(crate) fn is_empty(&self) -> bool {
        self.on.is_empty()
    }

    /// In warp (not dropping out).
    pub fn contains(&self, id: EntityId) -> bool {
        self.on.get(id) == Some(&IN_WARP)
    }

    fn phase(&self, id: EntityId) -> Option<u8> {
        self.on.get(id).copied()
    }

    pub(crate) fn remove(&mut self, id: EntityId) {
        self.on.remove(id);
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.str("attack-warp-v1");
        h.u64(self.on.len() as u64);
        for (id, phase) in self.on.iter() {
            h.u64(id.0 as u64);
            h.u64(*phase as u64);
        }
    }

    pub(crate) fn to_rec(&self) -> Vec<(EntityId, u8)> {
        self.on.iter().map(|(id, p)| (id, *p)).collect()
    }

    /// `None` when the ids do not ascend or a phase is unknown.
    pub(crate) fn from_rec(rows: Vec<(EntityId, u8)>) -> Option<Warp> {
        if rows.iter().any(|(_, p)| *p != IN_WARP && *p != WARP_OUT) {
            return None;
        }
        Some(Warp { on: Table::from_rows(rows)? })
    }
}

/// The attack range of a unit: the longest regular weapon, if any.
pub(crate) fn attack_range(db: &ContentDb, def: &UnitDef) -> Option<Fixed> {
    def.hardpoints
        .iter()
        .filter_map(|hp| db.weapon(hp.weapon.key))
        .filter(|w| w.kind != WeaponKind::PointDefence && !crate::specials::is_special(w) && w.resource.is_none())
        .map(|w| w.range)
        .max()
}

/// Gap kept to the target's surface by the goal point (`sk_ATTACK_BUFFER`,
/// read from the executable, static).
fn goal_gap() -> Fixed {
    Fixed::from_int(50)
}

/// The engine's warp lead: drop out of warp when the goal is nearer than
/// `(vw + vi) / 2 + 2 (k - 1) vi / (vw - vi) R` with
/// `k = clamp(0.6 vw / (vi + 0.1), 1.5, 6)` (read from the executable, static).
fn warp_lead(vw: Fixed, vi: Fixed, r: Fixed) -> Fixed {
    let k = (Fixed::from_f64(0.6) * vw / (vi + Fixed::from_f64(0.1))).max(Fixed::from_f64(1.5)).min(Fixed::from_int(6));
    (vw + vi) / Fixed::from_int(2) + Fixed::from_int(2) * (k - Fixed::ONE) * vi / (vw - vi) * r
}

impl Sim {
    /// Set the course of every craft with an attack target and say how it
    /// steps this tick (ascending id). Runs before the movement step.
    pub(crate) fn steer_attackers(&mut self) -> Vec<(EntityId, Plan)> {
        let Some(db) = self.content.clone() else { return Vec::new() };
        let f = db.features();
        let min_warp = db.rules().approach().min_move_warp_distance;
        let mut plans = Vec::new();
        for (id, tid) in self.weapons.attacks() {
            let (Some(u), Some(t)) = (self.units.get(id), self.units.get(tid)) else { continue };
            if u.kind != EntityKind::Craft || !u.mobile() || t.hull[1] <= Fixed::ZERO || self.production.is_welding(id) {
                continue;
            }
            let Some(def) = db.unit(u.def) else { continue };
            let Some(range) = attack_range(&db, def) else { continue };
            let r_own = def.radius;
            let r_t = db.unit(t.def).map_or(Fixed::ZERO, |d| d.radius);
            let (dx, dz) = (u.pos.x - t.pos.x, u.pos.z - t.pos.z);
            let centre = (dx * dx + dz * dz).sqrt();
            let d = (centre - r_own - r_t).max(Fixed::ZERO);
            let w = range + if t.kind == EntityKind::Craft { r_t } else { Fixed::ZERO };
            let buffer = if f.attack_range_buffer {
                (w / Fixed::from_int(5) + Fixed::from_int(5)).min(goal_gap()) + r_t
            } else {
                Fixed::ZERO
            };
            let mut plan = Plan::default();
            let closer = d > w - buffer;
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
            let mut warp = phase == Some(IN_WARP);
            if let Some(g) = goal {
                let (gx, gz) = (g.x - u.pos.x, g.z - u.pos.z);
                let to_goal = (gx * gx + gz * gz).sqrt();
                let aligned = to_goal > Fixed::ZERO && (u.heading.0 * gx + u.heading.1 * gz) / to_goal > Fixed::from_f64(0.99);
                let can = vw > vi && aligned && to_goal >= warp_lead(vw, vi, r_own);
                warp = if warp {
                    can && !(f.attack_warp_out_at_distance && !far)
                } else {
                    can && far && u.speed > Fixed::from_f64(0.9) * vi
                };
                if !warp && !far && f.attack_combat_speed && t.speed < combat {
                    plan.cap = Some(combat);
                }
            } else {
                warp = false;
            }
            plan.warp = warp;
            let limit = plan.cap.unwrap_or(vi);
            if warp {
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
        let stale: Vec<EntityId> = self.approach_warp.on.iter().map(|(id, _)| id).filter(|id| plans.binary_search_by_key(id, |p| p.0).is_err()).collect();
        for id in stale {
            self.approach_warp.remove(id);
        }
        plans
    }
}
