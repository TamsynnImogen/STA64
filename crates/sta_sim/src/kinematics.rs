//! How a unit flies to its current leg (SIM-15): one movement model per
//! content profile, picked by [`Model::of`].
//!
//! [`movement`](crate::movement) decides *where* a unit goes (its target and
//! the legs queued after it); this module decides *how* it gets there, one
//! tick at a time, in fixed point and without platform maths (see the crate
//! rules). Each model reads its own share of [`MoveStats`](sta_content::MoveStats);
//! fields a model does not read change nothing.
//!
//! # Models
//!
//! **Native** (STA64's own, SIM-01). Speed eases toward cruise scaled by how
//! well the unit faces its target (never below [`MIN_ALIGN`]), the turn rate
//! is the fastest that can still stop on the heading, the unit brakes to land
//! exactly on each point and stops at every waypoint. Reads the first five
//! `MoveStats` fields as rates directly (`forwardAccel` per second of cruise,
//! `turnOmega` radians per second, `turnAlpha` radians per second squared).
//!
//! **Smooth** (Armada II's "smooth" physics). Stock Armada II, its STA2
//! Classic mod and Fleet Operations 4 all run the same executable, so they
//! share this model; only their physics ODFs differ (stock: quick, car-like
//! ships that cannot turn at rest; FO: slow acceleration, slow turn
//! acceleration, full turning at rest). Behaviour as analysed from the stock
//! executable, in our words:
//!
//! - *Scale.* Rates scale with `combatSpeed` (`K`), not `impulseSpeed`
//!   (cruise speed, the cap). Acceleration and braking are both
//!   `1.3 K forwardAccel` per second (`backwardAccel` is read by the game but
//!   never used by this controller, so it is not read here). The turn rate
//!   limit is `0.5 K turnOmega` radians per second, the turn acceleration
//!   `turnAlpha` times that.
//! - *At rest.* With `s` the share of cruise speed, the turn rate limit is
//!   scaled by `f + (1 - f) s` for `f = turnOmegaFractionAtRest`, the turn
//!   acceleration likewise by `turnAlphaFractionAtRest`. 0 is a car (no
//!   turning when stopped), 1 a spaceship.
//! - *Controllers.* Speed and turn rate follow a time-optimal bang-bang
//!   controller: full acceleration, cruise at the limit, full braking timed
//!   to stop on the goal (or to pass it at an end speed), integrated exactly
//!   within the tick.
//! - *Steering.* The goal heading is the direction to a chase point. Inside
//!   `turnControlAngle` of it, the controller aims only `err * (err / angle)`
//!   (squared with `turnControlSquared`) of the remaining error, so the turn
//!   eases in instead of snapping.
//! - *Speed.* A unit not facing its chase point within 45 degrees asks for
//!   full cruise speed (no slowing to turn) and turns as it goes. Inside the
//!   cone, speed is set by distance: reach the chase point at the chase
//!   point's own speed, braking to rest only for the last point. Inside
//!   `forwardControlDistance` the distance is damped to `d * d / distance`.
//! - *Too close to turn.* Within `tooCloseToTurn` of the final point, or
//!   when the chase point lies inside the full-speed turning circle, the unit
//!   stops steering and only controls its speed along the nose: it does not
//!   circle round to a point behind it but stays where it is.
//! - *Paths.* The chase point runs `pathLeadDistance` ahead of the unit on
//!   its path and moves at cruise speed, at rest on the last leg. A unit
//!   moves on to the next waypoint once within `pathLeadDistance` of the
//!   current one, so it flies through waypoints without stopping.
//! - *Arrival.* Within `tooCloseToTurn` of the final point and below half
//!   of `combatSpeed`, the unit is told to stop: it brakes to rest where it
//!   is, without snapping onto the point.
//!
//! The game runs this at 10 Hz and re-aims the damped heading once a tick;
//! we re-aim every one of our ticks, so the damped tail of a turn settles a
//! little faster than the game's. Heights move as in the native model (the
//! game pitches; we have no pitch).
//!
//! Units with no target brake to rest under every model.

use sta_content::Profile;
use sta_core::{Fixed, TICK_HZ, Vec3};

use crate::{Sim, Unit};

/// The movement model a profile flies by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    /// STA64's own (the SIM-01 model): no physics file to follow, so tuned
    /// for feel. Stops at every waypoint.
    Native,
    /// Armada II's "smooth" physics (stock, STA2 Classic, FO4).
    Smooth,
}

impl Model {
    /// The model of a profile. Armada I (`Sta1Classic`) has not been
    /// analysed and flies the native model until it is.
    pub fn of(profile: Profile) -> Model {
        match profile {
            Profile::Sta2Classic | Profile::Fo4 => Model::Smooth,
            Profile::Native | Profile::Sta1Classic => Model::Native,
        }
    }

    /// Whether a unit flies through a waypoint that has another leg queued
    /// behind it, instead of coming to rest on it.
    pub fn flows_through_waypoints(self) -> bool {
        matches!(self, Model::Smooth)
    }
}

/// What a step needs to know beyond the unit itself.
#[derive(Debug, Clone, Copy, Default)]
pub struct Leg<'a> {
    /// The legs queued after the current target, in order (empty for
    /// models that stop at every waypoint).
    pub queued: &'a [Vec3],
}

pub(crate) fn dt() -> Fixed {
    Fixed::ONE / Fixed::from_int(TICK_HZ as i32)
}

pub(crate) fn clamp(v: Fixed, lo: Fixed, hi: Fixed) -> Fixed {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// Move `v` toward `goal` by at most `step`.
pub(crate) fn approach(v: Fixed, goal: Fixed, step: Fixed) -> Fixed {
    if v < goal {
        if goal - v < step { goal } else { v + step }
    } else if v - goal < step {
        goal
    } else {
        v - step
    }
}

/// Rotate a unit vector by a small angle `a` (radians; |a| well under 1):
/// sin and cos from their Taylor series, then renormalised.
pub(crate) fn rotate(h: (Fixed, Fixed), a: Fixed) -> (Fixed, Fixed) {
    let a2 = a * a;
    let s = a - a * a2 / Fixed::from_int(6);
    let c = Fixed::ONE - a2 / Fixed::from_int(2) + a2 * a2 / Fixed::from_int(24);
    let x = h.0 * c - h.1 * s;
    let z = h.0 * s + h.1 * c;
    let len = (x * x + z * z).sqrt();
    if len == Fixed::ZERO {
        h
    } else {
        (x / len, z / len)
    }
}

/// [`rotate`] for any angle, in steps of at most a quarter radian.
fn rotate_by(mut h: (Fixed, Fixed), a: Fixed) -> (Fixed, Fixed) {
    let quarter = Fixed::ONE / Fixed::from_int(4);
    let mut left = a;
    while left.abs() > quarter {
        let s = if left < Fixed::ZERO {
            -quarter
        } else {
            quarter
        };
        h = rotate(h, s);
        left = left - s;
    }
    if left != Fixed::ZERO {
        rotate(h, left)
    } else {
        h
    }
}

fn pi() -> Fixed {
    Fixed::from_f64(std::f64::consts::PI)
}

/// `atan2(y, x)` in radians, `(-pi, pi]`: an octant reduction and a
/// polynomial for `atan` on `[0, 1]` (Abramowitz and Stegun 4.4.49, error
/// under 1e-5 rad).
pub(crate) fn atan2(y: Fixed, x: Fixed) -> Fixed {
    let (ax, ay) = (x.abs(), y.abs());
    if ax == Fixed::ZERO && ay == Fixed::ZERO {
        return Fixed::ZERO;
    }
    let (r, swap) = if ay <= ax {
        (ay / ax, false)
    } else {
        (ax / ay, true)
    };
    let r2 = r * r;
    let c = |v: f64| Fixed::from_f64(v);
    let mut a = r
        * (c(0.999_866)
            + r2 * (c(-0.330_299_5)
                + r2 * (c(0.180_141) + r2 * (c(-0.085_133) + r2 * c(0.020_835_1)))));
    if swap {
        a = pi() / Fixed::from_int(2) - a;
    }
    if x < Fixed::ZERO {
        a = pi() - a;
    }
    if y < Fixed::ZERO { -a } else { a }
}

/// Smallest share of cruise speed a unit keeps while turning toward a target
/// it is not facing (native model).
pub(crate) const MIN_ALIGN: f64 = 0.1;

/// Distance at which a unit counts as arrived (native model; the smooth
/// model's floor for `tooCloseToTurn`).
pub(crate) fn arrive_distance() -> Fixed {
    Fixed::from_int(4)
}

/// What an attack approach (SIM-36, [`crate::movement`]) asks of one unit's
/// step this tick, beyond its target.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Plan {
    /// Speed limit in place of `impulseSpeed` (`combatSpeed` on a stock
    /// approach); never above it.
    pub cap: Option<Fixed>,
    /// In warp: fly straight along the heading toward `warpSpeed`.
    pub warp: bool,
    /// With no target: turn to face this point while braking.
    pub face: Option<Vec3>,
}

/// Speed change per second in and out of warp, as a multiple of `warpSpeed`:
/// impulse to warp and warp to combat speed each take about 0.3 to 0.5 s
/// (measured on stock A2 and Roots, `state` marks every 0.25 s; the
/// executable's warp-out runs 0.1 to 3 s, static). No ODF key sets it.
fn warp_rate() -> Fixed {
    Fixed::from_int(2)
}

/// One tick of `u` under an approach plan: warp, the warp-out down to the
/// cap, then the model's own step at the capped speed, or turning to face.
pub(crate) fn step_planned(model: Model, u: &mut Unit, leg: Leg, plan: Plan) {
    let dt = dt();
    let warp = u.stats.warp_speed;
    if plan.warp && warp > u.stats.max_speed {
        u.speed = approach(u.speed, warp, warp * warp_rate() * dt);
        u.omega = Fixed::ZERO;
        advance(u, dt);
        if let Some(t) = u.target {
            climb(u, t, dt);
        }
        return;
    }
    let cap = plan.cap.map_or(u.stats.max_speed, |c| c.min(u.stats.max_speed));
    if u.speed > u.stats.max_speed {
        // Dropping out of warp: straight on, down to the cap.
        u.speed = approach(u.speed, cap, warp.max(u.speed) * warp_rate() * dt);
        u.omega = Fixed::ZERO;
        advance(u, dt);
        return;
    }
    if u.target.is_none() {
        if let Some(p) = plan.face {
            face(model, u, p, dt);
            return;
        }
    }
    let impulse = u.stats.max_speed;
    u.stats.max_speed = cap;
    step(model, u, leg);
    u.stats.max_speed = impulse;
}

/// Brake to rest while turning to face `p` (an attacker in range turns its
/// nose to the target, SIM-36: read from the executable, static).
fn face(model: Model, u: &mut Unit, p: Vec3, dt: Fixed) {
    let (dx, dz) = (p.x - u.pos.x, p.z - u.pos.z);
    let d = (dx * dx + dz * dz).sqrt();
    let (cross, dot) = if d == Fixed::ZERO {
        (Fixed::ZERO, Fixed::ONE)
    } else {
        let (tx, tz) = (dx / d, dz / d);
        (u.heading.0 * tz - u.heading.1 * tx, u.heading.0 * tx + u.heading.1 * tz)
    };
    let err = atan2(cross, dot);
    match model {
        Model::Smooth => smooth::face(u, err, dt),
        Model::Native => {
            let st = u.stats;
            let accel = st.forward_accel * st.max_speed;
            u.speed = approach(u.speed, Fixed::ZERO, accel * dt);
            let want = clamp(err / dt, -st.turn_omega, st.turn_omega);
            u.omega = approach(u.omega, want, st.turn_alpha * dt);
            advance(u, dt);
        }
    }
}

impl Sim {
    /// Step every mobile unit under the content's movement model; units
    /// with an approach plan (ascending id) step under it.
    pub(crate) fn step_kinematics(&mut self, plans: &[(sta_core::EntityId, Plan)]) {
        let model = Model::of(
            self.content
                .as_ref()
                .map(|c| c.profile())
                .unwrap_or_default(),
        );
        let flows = model.flows_through_waypoints();
        let obstacles = if self.confinement.is_active() { self.obstacles() } else { Vec::new() };
        let db = self.content.as_deref();
        for u in self.units.iter_mut().filter(|u| u.mobile()) {
            let queued = match self.movement.course(u.id) {
                Some(c) if flows => c.waypoints.as_slice(),
                _ => &[],
            };
            let previous = u.pos;
            match plans.binary_search_by_key(&u.id, |p| p.0) {
                Ok(i) => step_planned(model, u, Leg { queued }, plans[i].1),
                // Out of warp with no attack left: the warp-out still runs.
                Err(_) if u.speed > u.stats.max_speed => step_planned(model, u, Leg { queued }, Plan::default()),
                Err(_) => step(model, u, Leg { queued }),
            }
            if self.confinement.is_active() && u.pos != previous {
                let radius = db.and_then(|db| db.unit(u.def)).map_or(Fixed::ZERO, |d| d.radius * Fixed::from_int(3) / Fixed::from_int(4));
                crate::avoid::constrain_position(&mut self.confinement, u, previous, radius, &obstacles);
            }
        }
        if self.confinement.is_active() {
            for u in self.units.iter_mut() {
                if self.confinement.ticks(u) {
                    let previous = u.pos;
                    let radius = db.and_then(|db| db.unit(u.def)).map_or(Fixed::ZERO, |d| d.radius * Fixed::from_int(3) / Fixed::from_int(4));
                    crate::avoid::constrain_position(&mut self.confinement, u, previous, radius, &obstacles);
                }
            }
        }
    }
}

/// One tick of `u` under `model`.
pub fn step(model: Model, u: &mut Unit, leg: Leg) {
    match model {
        Model::Native => native::step(u),
        Model::Smooth => smooth::step(u, leg),
    }
}

/// Heading and position from the current turn rate and speed.
fn advance(u: &mut Unit, dt: Fixed) {
    if u.omega != Fixed::ZERO {
        u.heading = rotate(u.heading, u.omega * dt);
    }
    u.pos.x = u.pos.x + u.heading.0 * u.speed * dt;
    u.pos.z = u.pos.z + u.heading.1 * u.speed * dt;
}

/// Height: straight toward the target's, no faster than the cruise speed.
fn climb(u: &mut Unit, target: Vec3, dt: Fixed) {
    let st = u.stats;
    let dy = target.y - u.pos.y;
    u.pos.y = u.pos.y + clamp(dy, -st.max_speed * dt, st.max_speed * dt);
}

/// STA64's own model, unchanged from SIM-01 (its digests are pinned).
mod native {
    use super::*;

    pub(super) fn step(u: &mut Unit) {
        let dt = dt();
        let st = u.stats;
        let accel = st.forward_accel * st.max_speed;
        let Some(target) = u.target else {
            u.speed = approach(u.speed, Fixed::ZERO, accel * dt);
            u.omega = approach(u.omega, Fixed::ZERO, st.turn_alpha * dt);
            advance(u, dt);
            return;
        };
        let (dx, dz) = (target.x - u.pos.x, target.z - u.pos.z);
        let dist = (dx * dx + dz * dz).sqrt();
        if dist <= arrive_distance().max(u.speed * dt) {
            u.pos = target;
            u.target = None;
            u.speed = Fixed::ZERO;
            u.omega = Fixed::ZERO;
            return;
        }
        let (tx, tz) = (dx / dist, dz / dist);
        // cross > 0: the target lies toward +omega; dot: how aligned we are.
        let cross = u.heading.0 * tz - u.heading.1 * tx;
        let dot = u.heading.0 * tx + u.heading.1 * tz;

        let frac = if st.max_speed > Fixed::ZERO {
            clamp(u.speed / st.max_speed, Fixed::ZERO, Fixed::ONE)
        } else {
            Fixed::ZERO
        };
        let omega_max = st.turn_omega * (st.turn_at_rest + (Fixed::ONE - st.turn_at_rest) * frac);
        let sign = if cross < Fixed::ZERO {
            -Fixed::ONE
        } else {
            Fixed::ONE
        };
        // Remaining turn angle from the sine and the side: asin(c) ~ c + c^3/6
        // ahead, pi minus that behind.
        let c = cross.abs();
        let ahead = c + c * c * c / Fixed::from_int(6);
        let angle = if dot >= Fixed::ZERO {
            ahead
        } else {
            pi() - ahead
        };
        // The fastest turn that can still stop on the heading with the turn
        // acceleration available: omega = sqrt(2 alpha angle). Asking for more
        // overshoots and weaves; this settles without oscillating.
        // Near the heading, also no more than closes the angle this tick, so the
        // discrete steps do not overshoot and chatter across the course.
        let want_omega = sign
            * (Fixed::from_int(2) * st.turn_alpha * angle)
                .sqrt()
                .min(omega_max)
                .min(angle / dt);
        u.omega = approach(u.omega, want_omega, st.turn_alpha * dt);

        // Speed: slower when facing away, and able to stop at the target.
        let align = clamp(dot, Fixed::from_f64(MIN_ALIGN), Fixed::ONE);
        let brake = (Fixed::from_int(2) * accel * dist).sqrt();
        let want_speed = (st.max_speed * align).min(brake);
        u.speed = approach(u.speed, want_speed, accel * dt);

        climb(u, target, dt);
        advance(u, dt);
    }
}

/// Armada II's smooth physics; see the module docs.
mod smooth {
    use super::*;

    /// Linear acceleration per unit of `combatSpeed * forwardAccel`.
    fn accel_scale() -> Fixed {
        Fixed::from_f64(1.3)
    }

    /// Turn rate limit per unit of `combatSpeed * turnOmega`.
    fn turn_scale() -> Fixed {
        Fixed::ONE / Fixed::from_int(2)
    }

    /// The controllers' tolerance (distance, angle and rate alike).
    fn tol() -> Fixed {
        Fixed::from_f64(0.001)
    }

    /// Within this distance of its chase point, the speed asked for outside
    /// the cone falls off linearly.
    fn slow_radius() -> Fixed {
        Fixed::from_int(20)
    }

    /// Cosine of the half-angle of the cone ahead (45 degrees) inside which
    /// speed is set by distance.
    fn cone() -> Fixed {
        Fixed::from_f64(0.707)
    }

    /// Share of `combatSpeed` under which a unit near its final point
    /// counts as arrived.
    fn arrive_share() -> Fixed {
        Fixed::ONE / Fixed::from_int(2)
    }

    /// Most segments one controller call splits a tick into.
    const MAX_SEGMENTS: usize = 8;

    /// One stretch of constant acceleration: `(distance, new rate)`.
    fn integrate(v: Fixed, acc: Fixed, t: Fixed) -> (Fixed, Fixed) {
        (v * t + acc * t * t / Fixed::from_int(2), v + acc * t)
    }

    /// Rate `v` toward `goal` at acceleration `a`, for `dt`; then constant.
    /// Returns the distance covered and the new rate.
    pub(super) fn vc(v: Fixed, goal: Fixed, a: Fixed, dt: Fixed) -> (Fixed, Fixed) {
        let gap = goal - v;
        if gap.abs() < tol() || a <= Fixed::ZERO {
            let v = if gap.abs() < tol() { goal } else { v };
            return (v * dt, v);
        }
        let acc = if gap > Fixed::ZERO { a } else { -a };
        let t = (gap.abs() / a).min(dt);
        let (s, nv) = integrate(v, acc, t);
        let nv = if t < dt { goal } else { nv };
        (s + nv * (dt - t), nv)
    }

    /// Time-optimal move of a position `d` ahead to rest there, from rate
    /// `v`, with acceleration `a` and rate limit `vmax`, for `dt`. Signed.
    /// Returns the distance covered and the new rate.
    pub(super) fn pc(
        mut d: Fixed,
        mut v: Fixed,
        a: Fixed,
        vmax: Fixed,
        dt: Fixed,
    ) -> (Fixed, Fixed) {
        if a <= Fixed::ZERO {
            return (v * dt, v);
        }
        let two = Fixed::from_int(2);
        let mut moved = Fixed::ZERO;
        let mut left = dt;
        for _ in 0..MAX_SEGMENTS {
            if left <= Fixed::ZERO {
                break;
            }
            // Work in the frame where the rate is not negative.
            let sg = if v < Fixed::ZERO || (v == Fixed::ZERO && d < Fixed::ZERO) {
                -Fixed::ONE
            } else {
                Fixed::ONE
            };
            let (dd, vv) = (d * sg, v * sg);
            if vv < tol() && dd.abs() < tol() {
                v = Fixed::ZERO;
                break;
            }
            let rem = dd - vv * vv / (two * a);
            let arg = vv * vv / two + a * dd;
            let t_peak = if arg > Fixed::ZERO {
                (arg.sqrt() - vv) / a
            } else {
                Fixed::ZERO
            };
            let (acc, t) = if t_peak > Fixed::ZERO && rem > tol() {
                if vv < vmax - tol() {
                    (a, ((vmax - vv) / a).min(t_peak))
                } else if vv > vmax + tol() {
                    (-a, (vv - vmax) / a)
                } else {
                    (Fixed::ZERO, rem / vv)
                }
            } else {
                (-a, (vv + tol() / two) / a)
            };
            let t = if t > Fixed::ZERO { t.min(left) } else { left };
            let (s, nv) = integrate(vv, acc, t);
            d = (dd - s) * sg;
            v = nv * sg;
            moved = moved + s * sg;
            left = left - t;
        }
        if left > Fixed::ZERO {
            moved = moved + v * left;
        }
        (moved, v)
    }

    /// Forward speed `v` (not negative) to pass a point `d` ahead at speed
    /// `ve` (positive), limit `vmax`, acceleration `a`, for `dt`. Returns
    /// the distance covered and the new speed.
    pub(super) fn pc_through(
        mut d: Fixed,
        mut v: Fixed,
        ve: Fixed,
        a: Fixed,
        vmax: Fixed,
        dt: Fixed,
    ) -> (Fixed, Fixed) {
        if a <= Fixed::ZERO {
            return (v * dt, v);
        }
        let two = Fixed::from_int(2);
        let mut moved = Fixed::ZERO;
        let mut left = dt;
        for _ in 0..MAX_SEGMENTS {
            if left <= Fixed::ZERO {
                break;
            }
            if d <= tol() {
                // Reached or passed: hold the end speed.
                let (s, nv) = vc(v, ve, a, left);
                return (moved + s, nv);
            }
            let brake = if v > ve {
                (v * v - ve * ve) / (two * a)
            } else {
                Fixed::ZERO
            };
            let rem = d - brake;
            let (acc, t) = if rem > tol() {
                if v < vmax - tol() {
                    let peak2 = (v * v + ve * ve) / two + a * d;
                    (a, ((vmax - v) / a).min((peak2.sqrt() - v) / a))
                } else if v > vmax + tol() {
                    (-a, (v - vmax) / a)
                } else {
                    (Fixed::ZERO, rem / v)
                }
            } else if v > ve + tol() {
                (-a, (v - ve) / a)
            } else {
                (Fixed::ZERO, left)
            };
            let t = if t > Fixed::ZERO { t.min(left) } else { left };
            let (s, nv) = integrate(v, acc, t);
            d = d - s;
            v = nv.max(Fixed::ZERO);
            moved = moved + s;
            left = left - t;
        }
        if left > Fixed::ZERO {
            moved = moved + v * left;
        }
        (moved, v)
    }

    /// A unit's rate limits this tick.
    struct Limits {
        accel: Fixed,
        omega: Fixed,
        alpha: Fixed,
        /// The turn rate limit at full speed (sets the turning circle).
        omega_full: Fixed,
    }

    fn limits(u: &Unit) -> Limits {
        let st = u.stats;
        let k = st.combat_speed;
        let s = if st.max_speed > Fixed::ZERO {
            clamp(u.speed / st.max_speed, Fixed::ZERO, Fixed::ONE)
        } else {
            Fixed::ZERO
        };
        let omega_full = turn_scale() * k * st.turn_omega;
        let blend = |f: Fixed| f + (Fixed::ONE - f) * s;
        Limits {
            accel: accel_scale() * k * st.forward_accel,
            omega: omega_full * blend(st.turn_at_rest),
            alpha: st.turn_alpha * omega_full * blend(st.turn_alpha_at_rest),
            omega_full,
        }
    }

    /// Turn by `dyaw`, then move `ds` along the heading the tick started
    /// with (as the game does).
    fn apply(u: &mut Unit, dyaw: Fixed, ds: Fixed) {
        let h = u.heading;
        u.pos.x = u.pos.x + h.0 * ds;
        u.pos.z = u.pos.z + h.1 * ds;
        if dyaw != Fixed::ZERO {
            u.heading = rotate_by(h, dyaw);
        }
    }

    /// Brake to rest while turning by `err` radians (to face a target).
    pub(super) fn face(u: &mut Unit, err: Fixed, dt: Fixed) {
        let lim = limits(u);
        let (ds, v) = vc(u.speed, Fixed::ZERO, lim.accel, dt);
        let (dyaw, w) = pc(err, u.omega, lim.alpha, lim.omega, dt);
        u.speed = v.max(Fixed::ZERO);
        u.omega = w;
        apply(u, dyaw, ds);
    }

    /// No target: brake to rest and stop turning.
    fn halt(u: &mut Unit, lim: &Limits, dt: Fixed) {
        let (ds, v) = vc(u.speed, Fixed::ZERO, lim.accel, dt);
        let (dyaw, w) = vc(u.omega, Fixed::ZERO, lim.alpha, dt);
        u.speed = v.max(Fixed::ZERO);
        u.omega = w;
        apply(u, dyaw, ds);
    }

    pub(super) fn step(u: &mut Unit, leg: Leg) {
        let dt = dt();
        let st = u.stats;
        let lim = limits(u);
        let Some(target) = u.target else {
            halt(u, &lim, dt);
            return;
        };
        let lead = st.path_lead.max(arrive_distance());
        let near = st.too_close_to_turn.max(arrive_distance());
        let final_point = leg.queued.last().copied().unwrap_or(target);

        // Within the lead distance of a waypoint with legs behind it, aim
        // at the next leg; the course hands it over at the end of the tick.
        let pos = u.pos;
        let to = |p: Vec3| {
            let (dx, dy, dz) = (p.x - pos.x, p.y - pos.y, p.z - pos.z);
            (dx, dy, dz, (dx * dx + dy * dy + dz * dz).sqrt())
        };
        let (aim, last_leg) = match leg.queued.first() {
            Some(&next) if to(target).3 < lead => {
                u.target = None;
                (next, leg.queued.len() == 1)
            }
            _ => (target, leg.queued.is_empty()),
        };

        let (dx, _, dz, _) = to(aim);
        let flat = (dx * dx + dz * dz).sqrt();
        let (fx, _, fz, _) = to(final_point);
        let to_final = (fx * fx + fz * fz).sqrt();
        if last_leg && to_final < near && u.speed < arrive_share() * st.combat_speed {
            u.target = None;
            halt(u, &lim, dt);
            return;
        }
        if flat == Fixed::ZERO {
            halt(u, &lim, dt);
            climb(u, aim, dt);
            return;
        }

        // The chase point: `lead` ahead toward the aim, or the aim itself;
        // it moves at cruise speed, at rest on the last leg.
        let (tx, tz) = (dx / flat, dz / flat);
        let chase_dist = flat.min(lead);
        let (cx, cz) = (tx * chase_dist, tz * chase_dist);
        let h = u.heading;
        let cross = h.0 * tz - h.1 * tx;
        let dot = h.0 * tx + h.1 * tz;
        let chase_speed = if last_leg { Fixed::ZERO } else { st.max_speed };

        // Too close to turn: the final point is near, or the chase point
        // lies inside the full-speed turning circle on its side.
        let inside_circle = if lim.omega_full > Fixed::ZERO {
            let r = st.max_speed / lim.omega_full;
            let (nx, nz) = if cross >= Fixed::ZERO {
                (-h.1, h.0)
            } else {
                (h.1, -h.0)
            };
            let (ex, ez) = (cx - nx * r, cz - nz * r);
            ex * ex + ez * ez < r * r
        } else {
            true
        };
        let height_ok = (aim.y - u.pos.y).abs() <= st.too_close_to_turn;
        let close = height_ok && (to_final <= st.too_close_to_turn || inside_circle);

        // Speed by distance along the nose, or full speed.
        let along = h.0 * cx + h.1 * cz;
        let by_distance = close || dot > cone();
        let (ds, v) = if by_distance {
            let fcd = st.forward_control_distance;
            let mut d = along;
            if fcd > Fixed::ZERO && d.abs() < fcd {
                d = d * d.abs() / fcd;
            }
            let d = d.max(Fixed::ZERO);
            let ve = (chase_speed * dot).max(Fixed::ZERO);
            if ve > Fixed::ZERO {
                pc_through(d, u.speed, ve, lim.accel, st.max_speed, dt)
            } else {
                pc(d, u.speed, lim.accel, st.max_speed, dt)
            }
        } else {
            let want = st.max_speed * (chase_dist / slow_radius()).min(Fixed::ONE);
            vc(u.speed, want, lim.accel, dt)
        };

        // Turn toward the chase point, damped near the heading; or not at all.
        let (dyaw, w) = if close {
            vc(u.omega, Fixed::ZERO, lim.alpha, dt)
        } else {
            let err = atan2(cross, dot);
            let band = st.turn_control_angle * pi() / Fixed::from_int(180);
            let mut aim_err = err;
            if band > Fixed::ZERO && err.abs() < band {
                let share = err.abs() / band;
                aim_err = aim_err * share;
                if st.turn_control_squared {
                    aim_err = aim_err * share;
                }
            }
            pc(aim_err, u.omega, lim.alpha, lim.omega, dt)
        };

        u.speed = v.max(Fixed::ZERO);
        u.omega = w;
        climb(u, aim, dt);
        apply(u, dyaw, ds);
    }
}

#[cfg(test)]
mod tests;
