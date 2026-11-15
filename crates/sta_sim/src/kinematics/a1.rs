//! Armada I's plain-move model, step P1 (SIM-66).
//!
//! Rules, in our words. Grades: **measured** on the original, **exe** read
//! from the executable (static), **ours** STA64's own stand-in.
//!
//! - *Cruise.* A plain move flies at the craft's own `warpSpeed`: the
//!   throttle multiplier is `warpSpeed / impulseSpeed` (1 when either is not
//!   positive), worked out every tick from `stats`, so a scripted warp or
//!   impulse value feeds it with no state of its own. Measured, held on
//!   straight legs.
//! - *Speed lag.* Forward speed follows the throttle with a first-order lag of
//!   `frontBackInertia / impulseSpeed` seconds of the **play physics** (every
//!   steered craft flies with one shared physics file outside cineractives:
//!   exe; the lag is measured, the same for every class). Steady speed is
//!   `impulse x throttle`. Reverse throttle uses `maxReverseSpeed`.
//! - *Throttle.* More than 45 degrees off its path point (or while the heading
//!   error is above `headingCloseEnoughForWarp`) the craft asks for the
//!   impulse throttle, so it keeps moving while it turns (measured; never a
//!   pivot on the spot). Within 45 degrees a damped controller
//!   (`braccelControlStiffness`, critically damped) chases the path point,
//!   which runs `pathLeadDistance` ahead at the cruise speed and at rest on
//!   the last point; the throttle saturates at the multiplier far out and
//!   brakes ahead of the goal, to rest short of it without overshoot
//!   (measured; structure exe).
//! - *Stop.* Within the stop radius of the final point (`tooCloseToTurn`, at
//!   least the arrival distance and the turning-circle diameter; ours for the
//!   last term) the craft stops steering and brakes onto it; the order ends
//!   when it is also slower than half its impulse speed. With no target the
//!   throttle is zero and the craft coasts about `speed x lag` (measured).
//! - *Turn.* A servo (`sy_yawAngleCoeff`, `sy_yawVelocCoeff`) asks for a
//!   share of the steady turn rate, which grows with speed as the shared
//!   curve `velocCoeffStill`, `velocWhenCoeffHalf` and is scaled by
//!   `impulseSpeed / turnRadius` (exe; the sustained rate in a sharp turn is
//!   measured). The executable banks the craft (a critically damped roll sets
//!   the yaw rate); we have no roll state, so the turn rate follows the
//!   command through a first-order lag of `4 / (rollRotationSpeed x impulse)`
//!   seconds. **Ours**; a value of `rollRotationSpeed` that is not positive
//!   means no lag.
//! - *Borg.* A `borgPhysics` craft has no bank coupling: its spin follows an
//!   error controller (`rotationStiffness`) capped at `maxRotationSpeed` with
//!   `rotationAcceleration`, and it holds its facing below `tooSlowToTurn`.
//!   exe; not measured. Its linear motion uses the same throttle and lag.
//!
//! Attack approaches (SIM-36) are an interim: a plan's speed cap limits the
//! throttle, its face point steers a craft with no target, its warp phase is
//! ignored (the multiplier already is warp). Not done: roll and sideways skid
//! state, the cineractive's own-physics swap, the speed factors 0.8 and 0.5
//! the executable applies after other orders.
//!
//! # The seam
//!
//! `sta_content` (CON-52) reads the play-physics file (the profile's setting,
//! `Profile::play_physics_file`) and each Borg craft's own keys, for
//! `Sta1Classic` only. [`params`] is the one place the model asks for them;
//! it answers "not available" (and then
//! [`Model::for_content`](super::Model::for_content) keeps the native model)
//! for every other profile and for an unusable block. The key names are on
//! [`A1Dynamics`] and [`A1Borg`]; values are the ODF's own (fractions
//! and degrees as written, the model does the unit changes).

use sta_content::ContentDb;
use sta_core::{ContentKey, Fixed, Vec3};

use super::*;

pub use sta_content::{A1Borg, A1Dynamics};

/// What the model needs for one craft.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct A1Params {
    pub play: A1Dynamics,
    /// `Some` for a Borg-physics craft.
    pub borg: Option<A1Borg>,
}

/// The one place the model asks content for its parameters. `def` is the
/// craft's definition, or [`ContentKey::NONE`] to ask only whether the play
/// physics is there at all (it is database-wide).
///
/// Content (CON-52) answers for `Sta1Classic` only: the play-physics block is
/// database-wide and present only when the profile's play file loaded usably;
/// a definition that is not a steered craft (no positive `impulseSpeed` of its
/// own: structures, planets, weapons) gets `None` and keeps the old movement,
/// as does everything when the block is absent or degenerate.
pub fn params(db: &ContentDb, def: ContentKey) -> Option<A1Params> {
    #[cfg(test)]
    if let Some(p) = seam::get(def) {
        return Some(p);
    }
    let play = *db.a1_play_physics()?;
    if def == ContentKey::NONE {
        return Some(A1Params { play, borg: None });
    }
    let unit = db.unit(def).filter(|u| u.movement.max_speed > Fixed::ZERO)?;
    Some(A1Params { play, borg: unit.a1_borg })
}

/// `e^-x` for `x >= 0`: the series on a halved argument, squared back.
///
/// Valid range: any `x`. Below zero gives one; from 23 up gives zero (e^-23 is
/// under one unit of the last place of 32.32), which also keeps the series
/// from running on an argument large enough to wrap.
pub(super) fn exp_neg(x: Fixed) -> Fixed {
    if x <= Fixed::ZERO {
        return Fixed::ONE;
    }
    if x >= Fixed::from_int(23) {
        return Fixed::ZERO;
    }
    let mut y = x;
    let mut halvings = 0;
    while y > Fixed::from_f64(0.125) && halvings < 24 {
        y = y / Fixed::from_int(2);
        halvings += 1;
    }
    let y2 = y * y;
    let mut e = Fixed::ONE - y + y2 / Fixed::from_int(2) - y2 * y / Fixed::from_int(6)
        + y2 * y2 / Fixed::from_int(24);
    for _ in 0..halvings {
        e = e * e;
    }
    e
}

/// The throttle multiplier `warpSpeed / impulseSpeed` (1 when either is not
/// positive).
pub(crate) fn multiplier(st: &sta_content::MoveStats) -> Fixed {
    if st.warp_speed > Fixed::ZERO && st.max_speed > Fixed::ZERO {
        st.warp_speed / st.max_speed
    } else {
        Fixed::ONE
    }
}

fn lag_seconds(d: &A1Dynamics) -> Fixed {
    let floor = Fixed::ONE / Fixed::from_int(64);
    if d.impulse_speed > Fixed::ZERO {
        (d.front_back_inertia / d.impulse_speed).max(floor)
    } else {
        floor
    }
}

/// The turn authority curve `c(v)`: `velocCoeffStill` at rest, one half at
/// `velocWhenCoeffHalf x impulse`, rising toward one. Valid range:
/// `velocCoeffStill` in [0, 0.5] (above that the half point would lie below
/// the rest value and the curve has a pole; mod data above it is clamped to
/// 0.5, a flat curve), so the result is in [0, 1).
pub(super) fn authority(d: &A1Dynamics, v: Fixed) -> Fixed {
    let still = d
        .veloc_coeff_still
        .min(Fixed::from_f64(0.5))
        .max(Fixed::ZERO);
    let b = Fixed::ONE / (Fixed::ONE - still);
    let half = d.veloc_when_coeff_half * d.impulse_speed;
    if half <= Fixed::ZERO {
        return still;
    }
    let a = (Fixed::from_int(2) - b) / half;
    let den = (a * v.abs() + b).max(Fixed::ONE);
    Fixed::ONE - Fixed::ONE / den
}

/// The most any turn rate may be (radians per second): well above any shipped
/// rates, and keeps one tick's rotation small for `rotate` whatever mod data
/// says.
pub(super) const MAX_TURN_RATE: f64 = 12.0;

/// The steady turn rate (radians per second) of a full servo at speed `v`,
/// at most [`MAX_TURN_RATE`].
pub(super) fn full_turn_rate(d: &A1Dynamics, v: Fixed) -> Fixed {
    if d.turn_radius <= Fixed::ZERO || d.impulse_speed <= Fixed::ZERO {
        return Fixed::ZERO;
    }
    let at_impulse = authority(d, d.impulse_speed);
    if at_impulse <= Fixed::ZERO {
        return Fixed::ZERO;
    }
    (d.impulse_speed / d.turn_radius * authority(d, v) / at_impulse)
        .min(Fixed::from_f64(MAX_TURN_RATE))
}

/// What the controllers ask of one tick.
struct Cmd {
    throttle: Fixed,
    /// The signed heading error to steer out, or `None` for no steering.
    steer: Option<Fixed>,
    /// The point whose height the craft climbs toward.
    aim: Option<Vec3>,
}

/// The signed angle from the nose to the direction of `p` (positive toward
/// `+omega`).
fn bearing_error(u: &Unit, p: Vec3) -> Fixed {
    let (dx, dz) = (p.x - u.pos.x, p.z - u.pos.z);
    let d = (dx * dx + dz * dz).sqrt();
    if d == Fixed::ZERO {
        return Fixed::ZERO;
    }
    let (tx, tz) = (dx / d, dz / d);
    atan2(
        u.heading.0 * tz - u.heading.1 * tx,
        u.heading.0 * tx + u.heading.1 * tz,
    )
}

/// Cosine of the half-angle of the cone ahead (45 degrees) inside which the
/// controller sets the throttle.
fn cone() -> Fixed {
    Fixed::from_f64(0.707)
}

/// One tick of `u` under the model. `plan` is the attack approach's request
/// this tick, if any.
pub(super) fn step(u: &mut Unit, p: &A1Params, queued: &[Vec3], plan: Option<Plan>) {
    let dt = dt();
    let cmd = if u.stats.max_speed > crate::speed_override::HELD_SPEED {
        decide(u, p, queued, plan)
    } else {
        // A held craft (impulse at the crawl): nothing to fly with.
        Cmd {
            throttle: Fixed::ZERO,
            steer: None,
            aim: None,
        }
    };
    integrate(u, p, &cmd, dt);
    if let Some(aim) = cmd.aim {
        climb(u, aim, dt);
    }
}

/// The throttle and the steering for this tick.
fn decide(u: &mut Unit, p: &A1Params, queued: &[Vec3], plan: Option<Plan>) -> Cmd {
    let st = u.stats;
    let imp = st.max_speed;
    let none = Cmd {
        throttle: Fixed::ZERO,
        steer: None,
        aim: None,
    };
    let Some(target) = u.target else {
        // No target: no throttle, and an attacker in range turns to face.
        let steer = plan.and_then(|pl| pl.face).map(|f| bearing_error(u, f));
        return Cmd { steer, ..none };
    };
    let lead = st.path_lead.max(arrive_distance());
    // The stop radius: `tooCloseToTurn`, at least the arrival distance and at
    // least the diameter of the turning circle at impulse. A goal inside that
    // diameter (mod data only; the stock `tooCloseToTurn` is larger) cannot be
    // turned onto, so the craft is treated as there instead of circling or
    // stalling until the stuck-order watch.
    let near = st
        .too_close_to_turn
        .max(arrive_distance())
        .max(Fixed::from_int(2) * p.play.turn_radius.max(Fixed::ZERO));
    let final_point = queued.last().copied().unwrap_or(target);
    let pos = u.pos;
    let planar = |q: Vec3| {
        let (dx, dz) = (q.x - pos.x, q.z - pos.z);
        (dx, dz, (dx * dx + dz * dz).sqrt())
    };
    // The course hands over to the next waypoint once the craft is within
    // the stop radius of this one, or has reached the corner region (within
    // the lead distance) and crossed the bisector plane of the two legs at
    // it (the legs being the craft's present heading in and the next leg out),
    // or, moving, heading away from it with it behind the next leg and still
    // within twice the lead distance (a corner is cut by less than the lead
    // distance, so a craft that rounded it is inside that; one that never came
    // near it has not passed it), has passed it. So a corner of any angle, an out-and-back included, is
    // rounded only after the craft has got to it.
    let (mut tgt, mut rest) = (target, queued);
    if let Some(&next) = queued.first() {
        let (_, _, td) = planar(target);
        let (tx, tz, _) = planar(target);
        let (ox, oz) = (next.x - target.x, next.z - target.z);
        let ol = (ox * ox + oz * oz).sqrt();
        let hand_over = td < near
            || ol > Fixed::ZERO && {
                let (ox, oz) = (ox / ol, oz / ol);
                let (rx, rz) = (pos.x - target.x, pos.z - target.z);
                let bisector = rx * (u.heading.0 - ox) + rz * (u.heading.1 - oz) >= Fixed::ZERO;
                let behind = u.heading.0 * tx + u.heading.1 * tz < Fixed::ZERO
                    && tx * ox + tz * oz <= Fixed::ZERO;
                (td < lead && bisector)
                    || (td < lead * Fixed::from_int(2)
                        && behind
                        && u.speed > imp / Fixed::from_int(2))
            };
        if hand_over {
            u.target = None;
            (tgt, rest) = (next, &queued[1..]);
        }
    }
    let (fx, fz, to_final) = planar(final_point);
    if rest.is_empty() && to_final < near && u.speed < imp / Fixed::from_int(2) {
        // Arrived: the order ends and the craft coasts.
        u.target = None;
        return none;
    }

    // The path point: the first point along the course (craft, tgt, rest..)
    // at least `lead` from the craft, or the last point of the course. It
    // moves at the cruise speed along its leg, and is at rest on the last point.
    let (mut prev, mut carrot) = ((Fixed::ZERO, Fixed::ZERO), None);
    for q in std::iter::once(tgt).chain(rest.iter().copied()) {
        let (bx, bz, bd) = planar(q);
        if bd >= lead {
            let (dx, dz) = (bx - prev.0, bz - prev.1);
            let len = (dx * dx + dz * dz).sqrt();
            if len > Fixed::ZERO {
                let (ux, uz) = (dx / len, dz / len);
                let a_dot = prev.0 * ux + prev.1 * uz;
                let a2 = prev.0 * prev.0 + prev.1 * prev.1;
                let s = (-a_dot + (a_dot * a_dot - (a2 - lead * lead)).max(Fixed::ZERO).sqrt())
                    .min(len);
                carrot = Some((prev.0 + ux * s, prev.1 + uz * s, (ux, uz), false));
                break;
            }
        }
        prev = (bx, bz);
    }
    let (cx, cz, seg, at_end) =
        carrot.unwrap_or((prev.0, prev.1, (Fixed::ZERO, Fixed::ZERO), true));
    let chase = (cx * cx + cz * cz).sqrt();
    if chase == Fixed::ZERO {
        return Cmd {
            aim: Some(tgt),
            ..none
        };
    }

    let m = multiplier(&st);
    let allow = to_final >= st.too_close_for_warp;
    let mut tmax = if allow { m } else { Fixed::ONE };
    if let Some(cap) = plan.and_then(|pl| pl.cap) {
        tmax = tmax.min(cap / imp);
    }
    let d = &p.play;
    let k = d.braccel_control_stiffness.max(Fixed::ZERO);
    let two_root_k = Fixed::from_int(2) * k.sqrt();
    let tau = lag_seconds(d);
    let h = u.heading;
    // Throttle of the damped controller: desired acceleration along the nose
    // to reach `along` ahead at `along_speed`, plus what the lag eats.
    let control = |along: Fixed, along_speed: Fixed| {
        let want = k * along + two_root_k * (along_speed - u.speed) + u.speed / tau;
        let t = if want >= Fixed::ZERO {
            want * tau / imp
        } else if d.max_reverse_speed > Fixed::ZERO {
            want * tau / (d.max_reverse_speed * imp)
        } else {
            Fixed::ZERO
        };
        clamp(t, -tmax, tmax)
    };

    // On the last leg and within the stop radius of the final point: no
    // steering, brake onto it. (Passing near the final point on the way to a
    // waypoint is not arriving.)
    if rest.is_empty() && to_final <= near {
        return Cmd {
            throttle: control(h.0 * fx + h.1 * fz, Fixed::ZERO),
            steer: None,
            aim: Some(tgt),
        };
    }

    let (tx, tz) = (cx / chase, cz / chase);
    let cross = h.0 * tz - h.1 * tx;
    let dot = h.0 * tx + h.1 * tz;
    let cruise = if st.warp_speed > Fixed::ZERO {
        st.warp_speed
    } else {
        imp
    };
    let carrot_speed = if at_end { Fixed::ZERO } else { cruise };
    let along = h.0 * cx + h.1 * cz;
    let along_speed = carrot_speed * (h.0 * seg.0 + h.1 * seg.1);

    // The path point inside the turning circle on its side: no steering.
    if d.turn_radius > Fixed::ZERO {
        let r = d.turn_radius;
        let (nx, nz) = if cross >= Fixed::ZERO {
            (-h.1, h.0)
        } else {
            (h.1, -h.0)
        };
        let (ex, ez) = (cx - nx * r, cz - nz * r);
        if ex * ex + ez * ez < r * r {
            return Cmd {
                throttle: control(along, along_speed),
                steer: None,
                aim: Some(tgt),
            };
        }
    }

    let err = atan2(cross, dot);
    let band = st.heading_close_for_warp * pi() / Fixed::from_int(180);
    let mut throttle = if allow && err.abs() < band {
        m
    } else {
        Fixed::ONE
    };
    if let Some(cap) = plan.and_then(|pl| pl.cap) {
        throttle = throttle.min(cap / imp);
    }
    if dot > cone() {
        throttle = control(along, along_speed);
    }
    Cmd {
        throttle,
        steer: Some(err),
        aim: Some(tgt),
    }
}

/// Speed, turn rate, heading and position for one tick under `cmd`.
fn integrate(u: &mut Unit, p: &A1Params, cmd: &Cmd, dt: Fixed) {
    let d = &p.play;
    let imp = u.stats.max_speed.max(Fixed::ZERO);
    let (v, t) = (u.speed, cmd.throttle);

    // Forward speed: first-order lag toward `impulse x throttle` (reverse
    // throttle only brakes), exact over the tick.
    let tau = lag_seconds(d);
    let e = exp_neg(dt / tau);
    let v_ss = if t >= Fixed::ZERO {
        imp * t
    } else if v > Fixed::ZERO {
        d.max_reverse_speed * imp * t
    } else {
        Fixed::ZERO
    };
    let mut dist = v_ss * dt + (v - v_ss) * tau * (Fixed::ONE - e);
    let mut nv = v_ss + (v - v_ss) * e;
    if nv < Fixed::ZERO {
        nv = Fixed::ZERO;
    }
    if dist < Fixed::ZERO {
        dist = Fixed::ZERO;
    }

    // Turn rate.
    match p.borg {
        None => {
            let command = match cmd.steer {
                Some(err) => clamp(
                    d.yaw_angle_coeff * err - d.yaw_veloc_coeff * u.omega,
                    -Fixed::ONE,
                    Fixed::ONE,
                ),
                None => Fixed::ZERO,
            };
            let goal = command * full_turn_rate(d, v);
            let w = d.roll_rotation_speed * d.impulse_speed;
            u.omega = if w > Fixed::ZERO {
                let tau_turn = Fixed::from_int(4) / w;
                let e_turn = exp_neg(dt / tau_turn);
                goal + (u.omega - goal) * e_turn
            } else {
                goal
            };
        }
        Some(b) => {
            // Below `tooSlowToTurn` the desired facing is the current one.
            let err = match cmd.steer {
                Some(err) if v >= b.too_slow_to_turn => err,
                _ => Fixed::ZERO,
            };
            let k = b.rotation_stiffness.max(Fixed::ZERO);
            let scale = b.rotation_acceleration * b.max_rotation_speed;
            if scale > Fixed::ZERO {
                let want = k * err - Fixed::from_int(2) * k.sqrt() * u.omega;
                let share = clamp(
                    (want + b.rotation_acceleration * u.omega) / scale,
                    -Fixed::ONE,
                    Fixed::ONE,
                );
                let accel = share * scale - b.rotation_acceleration * u.omega;
                let max = Fixed::from_f64(MAX_TURN_RATE);
                u.omega = clamp(u.omega + accel * dt, -max, max);
            } else {
                u.omega = Fixed::ZERO;
            }
        }
    }

    if u.omega != Fixed::ZERO {
        u.heading = rotate(u.heading, u.omega * dt);
    }
    u.speed = nv;
    u.pos.x = u.pos.x + u.heading.0 * dist;
    u.pos.z = u.pos.z + u.heading.1 * dist;
}

/// Test inputs for the seam: a synthetic parameter block (key names and the
/// authored values), in force on this thread until the guard drops. A block may
/// be limited to one definition (the database-wide ask, with
/// [`ContentKey::NONE`], is always answered).
#[cfg(test)]
pub(crate) mod seam {
    use super::A1Params;
    use sta_core::ContentKey;
    use std::cell::Cell;

    type Block = Option<(A1Params, Option<ContentKey>)>;

    thread_local! {
        static BLOCK: Cell<Block> = const { Cell::new(None) };
    }

    /// Puts the block that was in force back when dropped.
    pub(crate) struct Guard(Block);

    pub(crate) fn set(p: A1Params) -> Guard {
        Guard(BLOCK.with(|b| b.replace(Some((p, None)))))
    }

    pub(crate) fn set_only(p: A1Params, def: ContentKey) -> Guard {
        Guard(BLOCK.with(|b| b.replace(Some((p, Some(def))))))
    }

    pub(super) fn get(def: ContentKey) -> Option<A1Params> {
        BLOCK.with(|b| b.get()).and_then(|(p, only)| {
            (only.is_none() || def == ContentKey::NONE || only == Some(def)).then_some(p)
        })
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            BLOCK.with(|b| b.set(self.0));
        }
    }
}
