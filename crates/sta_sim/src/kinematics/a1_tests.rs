//! The Armada I model (SIM-66): the shapes of the rules, checked against
//! expectations derived from the parameter block the tests author.
//!
//! The block below is an authored set (not any shipped content's values): the
//! tests assert what the rules say follows from it. Grades are in the module
//! documentation of `a1.rs`. Checks that hold by construction of the integrator
//! (and would pass on a wrong model too) are labelled so; the emergent ones are
//! the orders, the corners, the arrival and the stop radius. Comparison with the
//! shipped content belongs in an install-gated test (a TODO line in the PR).

// Platform maths measure the trace here only; the model under test uses Fixed.
#![allow(clippy::disallowed_methods)]

use super::a1::{A1Borg, A1Dynamics, A1Params, seam};
use super::*;
use crate::world::{Entity, EntityKind};
use sta_content::MoveStats;
use sta_core::{ContentKey, PlayerId};

fn f(x: f64) -> Fixed {
    Fixed::from_f64(x)
}

/// The authored play-physics block.
pub(crate) fn play() -> A1Dynamics {
    A1Dynamics {
        impulse_speed: f(24.0),
        front_back_inertia: f(6.0),
        max_reverse_speed: f(3.0),
        turn_radius: f(9.0),
        veloc_coeff_still: f(0.25),
        veloc_when_coeff_half: f(0.6),
        braccel_control_stiffness: f(2.0),
        yaw_angle_coeff: f(1.2),
        yaw_veloc_coeff: f(0.2),
        roll_rotation_speed: f(0.4),
    }
}

/// The impulse speed of the authored block and the lag it implies, seconds.
const IMPULSE: f64 = 24.0;
const TAU: f64 = 6.0 / 24.0;

pub(crate) fn params() -> A1Params {
    A1Params {
        play: play(),
        borg: None,
    }
}

const BORG_SPIN: f64 = 0.6;

fn borg() -> A1Params {
    A1Params {
        play: play(),
        borg: Some(A1Borg {
            max_rotation_speed: f(BORG_SPIN),
            rotation_acceleration: f(1.5),
            rotation_stiffness: f(0.8),
            too_slow_to_turn: f(4.0),
        }),
    }
}

/// The path lead distance of the authored classes.
const LEAD: f64 = 140.0;

/// A ship class with the authored impulse, its own `warpSpeed` and `tooCloseToTurn`.
pub(crate) fn class(warp: f64, near: f64) -> MoveStats {
    MoveStats {
        warp_speed: f(warp),
        too_close_to_turn: f(near),
        path_lead: f(LEAD),
        too_close_for_warp: f(12.0),
        heading_close_for_warp: f(25.0),
        ..MoveStats::from_f32(IMPULSE as f32, 1.0, 1.0, 1.0, 1.0)
    }
}

const A: (f64, f64) = (70.0, 35.0);
const B: (f64, f64) = (90.0, 45.0);
const C: (f64, f64) = (45.0, 30.0);

fn craft(stats: MoveStats) -> Unit {
    Entity::new(
        EntityKind::Craft,
        ContentKey::NONE,
        PlayerId(1),
        Vec3 {
            x: Fixed::ZERO,
            y: Fixed::ZERO,
            z: Fixed::ZERO,
        },
        (Fixed::ZERO, Fixed::ONE),
        stats,
    )
}

fn pt(x: f64, z: f64) -> Vec3 {
    Vec3 {
        x: f(x),
        y: Fixed::ZERO,
        z: f(z),
    }
}

fn num(x: Fixed) -> f64 {
    x.to_f32() as f64
}

/// One tick's record.
#[derive(Clone, Copy, Debug)]
struct S {
    t: f64,
    x: f64,
    z: f64,
    speed: f64,
    /// Turn rate, degrees per second.
    omega: f64,
    /// Angle from the nose to the final goal, degrees (absolute).
    err: f64,
    /// Distance to the final goal.
    dist: f64,
    /// The order is over (no target left).
    done: bool,
}

/// Fly a craft at rest facing +z along `legs` (the first is its target) for
/// `secs`, as the course does: the next leg is handed over when the model
/// clears the target.
fn fly(stats: MoveStats, p: &A1Params, legs: &[Vec3], secs: f64) -> Vec<S> {
    fly_from(craft(stats), p, legs, secs)
}

fn fly_from(mut u: Unit, p: &A1Params, legs: &[Vec3], secs: f64) -> Vec<S> {
    let mut queue: Vec<Vec3> = legs[1..].to_vec();
    u.target = Some(legs[0]);
    let last = *legs.last().unwrap();
    let mut out = Vec::new();
    for i in 0..(secs * TICK_HZ as f64) as u32 {
        a1::step(&mut u, p, &queue, None);
        if u.target.is_none() && !queue.is_empty() {
            u.target = Some(queue.remove(0));
        }
        let (gx, gz) = (num(last.x - u.pos.x), num(last.z - u.pos.z));
        let (hx, hz) = (num(u.heading.0), num(u.heading.1));
        let err = (hx * gz - hz * gx)
            .atan2(hx * gx + hz * gz)
            .to_degrees()
            .abs();
        out.push(S {
            t: (i + 1) as f64 / TICK_HZ as f64,
            x: num(u.pos.x),
            z: num(u.pos.z),
            speed: num(u.speed),
            omega: num(u.omega).to_degrees(),
            err,
            dist: (gx * gx + gz * gz).sqrt(),
            done: u.target.is_none() && queue.is_empty(),
        });
    }
    out
}

/// A goal `dist` away, `off` degrees off the bow (toward +omega).
fn goal(off: f64, dist: f64) -> Vec3 {
    let a = off.to_radians();
    pt(-dist * a.sin(), dist * a.cos())
}

fn first(tr: &[S], pred: impl Fn(&S) -> bool) -> f64 {
    tr.iter().find(|s| pred(s)).map_or(f64::INFINITY, |s| s.t)
}

fn mean(xs: impl Iterator<Item = f64>) -> f64 {
    let v: Vec<f64> = xs.collect();
    v.iter().sum::<f64>() / v.len().max(1) as f64
}

/// The steady turn rate of the authored block at speed `v`, degrees per second.
fn rate_at(v: f64) -> f64 {
    num(a1::full_turn_rate(&play(), f(v))).to_degrees()
}

#[test]
fn cruise_is_the_ships_own_warp_speed_held_on_a_straight_leg() {
    for (warp, near) in [A, B, C] {
        let tr = fly(class(warp, near), &params(), &[goal(0.0, 4000.0)], 40.0);
        // By construction (the lag holds the throttle `warpSpeed / impulse`):
        // the configured speed, within 1%, whatever the class.
        let steady: Vec<f64> = tr
            .iter()
            .filter(|s| s.t > 6.0 && s.t < 30.0)
            .map(|s| s.speed)
            .collect();
        assert!(
            (mean(steady.iter().copied()) - warp).abs() < 0.01 * warp,
            "{warp}"
        );
        assert!(steady.iter().all(|v| *v > 0.99 * warp), "{warp}");
    }
}

#[test]
fn speed_from_rest_is_one_first_order_lag_of_the_configured_constant() {
    let mut at_tau = Vec::new();
    for (warp, near) in [A, B, C] {
        let tr = fly(class(warp, near), &params(), &[goal(0.0, 4000.0)], 3.0);
        let share = |t: f64| tr.iter().find(|s| s.t >= t - 1e-6).unwrap().speed / warp;
        // By construction of the lag: 63% after one `frontBackInertia /
        // impulseSpeed`, and 90% after ln(10) of them.
        assert!((share(TAU) - 0.632).abs() < 0.03, "{warp}: {}", share(TAU));
        let t90 = first(&tr, |s| s.speed >= 0.9 * warp);
        assert!(
            (t90 - TAU * 10f64.ln()).abs() < 0.08,
            "{warp}: 90% at {t90}"
        );
        at_tau.push(share(TAU));
    }
    // Shared physics: the shares are equal across classes.
    assert!(
        at_tau.windows(2).all(|w| (w[0] - w[1]).abs() < 0.005),
        "{at_tau:?}"
    );
}

/// Emergent: a turn from rest is limited by the servo and the speed curve, so
/// the time to line up lies between what the highest steady rate allows and a
/// few times that, and the craft is up to cruise soon after.
#[test]
fn a_180_degree_order_turns_at_the_rate_the_play_physics_allows() {
    let tr = fly(class(A.0, A.1), &params(), &[goal(180.0, 1500.0)], 10.0);
    let fastest = rate_at(A.0);
    let slowest = rate_at(0.0);
    assert!(fastest > slowest && slowest > 0.0, "{slowest} {fastest}");
    let t10 = first(&tr, |s| s.err < 10.0);
    eprintln!("180: within 10 deg {t10}, rates {slowest}..{fastest}");
    // It cannot line up faster than the highest rate over 170 degrees, and the
    // servo and the lag cost no more than a few times that.
    assert!(t10 >= 170.0 / fastest, "{t10}");
    assert!(t10 <= 4.0 * 170.0 / slowest.max(0.5 * fastest), "{t10}");
    // The turn rate it reaches is on the curve: never above the rate at cruise,
    // and above the rate at rest (speed raises the authority).
    let peak = tr.iter().map(|s| s.omega.abs()).fold(0.0, f64::max);
    assert!(peak <= fastest * 1.001, "{peak} above {fastest}");
    assert!(
        peak > slowest,
        "{peak} not above the rate at rest {slowest}"
    );
    // And it then gets up to cruise.
    let t90 = first(&tr, |s| s.speed >= 0.9 * A.0);
    assert!(t90 > t10 - 1.0 && t90 < t10 + 3.0, "{t90} after {t10}");
}

#[test]
fn a_90_degree_order_from_rest_lines_up_sooner_than_a_180() {
    let t = |off: f64| {
        let tr = fly(class(B.0, B.1), &params(), &[goal(off, 1500.0)], 8.0);
        first(&tr, |s| s.err < 10.0)
    };
    let (t90, t180) = (t(90.0), t(180.0));
    eprintln!("lined up: 90 deg {t90}, 180 deg {t180}");
    assert!(t90 < t180, "{t90} {t180}");
    assert!(t90 >= 80.0 / rate_at(B.0), "{t90}");
}

/// Rule: the craft moves while it turns, at no more than the impulse speed.
#[test]
fn the_ship_does_not_pivot_on_the_spot_it_moves_while_turning() {
    for (warp, near) in [A, B, C] {
        let tr = fly(class(warp, near), &params(), &[goal(178.0, 1500.0)], 4.0);
        let t45 = first(&tr, |s| s.err < 45.0);
        let turning: Vec<&S> = tr.iter().filter(|s| s.t < t45).collect();
        let m = mean(turning.iter().map(|s| s.speed));
        let hi = turning.iter().map(|s| s.speed).fold(0.0, f64::max);
        eprintln!("{warp}: turning phase to {t45} s, mean {m}, max {hi}");
        // Impulse throttle while more than 45 degrees off: it speeds up toward
        // the impulse speed on the lag, and never above it.
        assert!(hi <= IMPULSE + 0.01, "{warp}: max {hi}");
        assert!(m > 0.3 * IMPULSE, "{warp}: mean {m}");
        // Moving from the first tenth of a second on: no stationary pivot.
        let want = IMPULSE * (1.0 - (-0.2 / TAU).exp());
        assert!(
            tr[5].speed > 0.7 * want,
            "{warp}: {} vs {want}",
            tr[5].speed
        );
        // The arc stays small: the craft turns on the spot of its own circle.
        let w = turning
            .iter()
            .map(|s| (s.x * s.x + s.z * s.z).sqrt())
            .fold(0.0, f64::max);
        assert!(w < 6.0 * 9.0, "{warp}: arc {w}");
    }
}

#[test]
fn a_path_bend_below_a_right_angle_keeps_most_of_the_speed() {
    let a = 78f64.to_radians();
    let legs = [
        pt(0.0, 3000.0),
        pt(-3000.0 * a.sin(), 3000.0 + 3000.0 * a.cos()),
    ];
    let tr = fly(class(B.0, B.1), &params(), &legs, 80.0);
    let near_bend =
        |s: &&S| s.t > 8.0 && ((s.x * s.x + (s.z - 3000.0) * (s.z - 3000.0)).sqrt()) < 300.0;
    let at_bend = tr
        .iter()
        .filter(near_bend)
        .map(|s| s.speed)
        .fold(f64::MAX, f64::min);
    eprintln!("bend: slowest {at_bend}");
    assert!(at_bend > 0.85 * B.0, "{at_bend}");
    assert!(tr.last().unwrap().done, "{:?}", tr.last());
}

#[test]
fn stopping_at_cruise_coasts_speed_times_the_lag() {
    // By construction of the lag: a craft at cruise, then no target, coasts
    // `speed x frontBackInertia / impulseSpeed`.
    let mut u = craft(class(A.0, A.1));
    u.speed = f(A.0);
    let p = params();
    let start = u.pos.z;
    for _ in 0..(3 * TICK_HZ) {
        a1::step(&mut u, &p, &[], None);
    }
    let coast = num(u.pos.z - start);
    assert!((coast - A.0 * TAU).abs() < 0.03 * A.0 * TAU, "{coast}");
    assert!(u.speed < f(0.5), "{:?}", u.speed);
}

/// Emergent: braking starts before the goal, never speeds up again, and the
/// craft ends inside its stop radius without passing the goal.
#[test]
fn arrival_brakes_before_the_goal_and_stops_short_without_overshoot() {
    for (warp, near, goal_z) in [(A.0, A.1, 2000.0), (B.0, B.1, 2400.0)] {
        let tr = fly(class(warp, near), &params(), &[pt(0.0, goal_z)], 40.0);
        let peak = tr.iter().map(|s| s.speed).fold(0.0, f64::max);
        let top = tr
            .iter()
            .position(|s| s.speed >= 0.995 * warp)
            .expect("reaches cruise");
        let start = tr[top..]
            .iter()
            .position(|s| s.speed < 0.995 * peak)
            .map(|i| i + top)
            .expect("brakes");
        let braking_at = tr[start].dist;
        eprintln!(
            "{warp}: braking starts {braking_at} out ({} x cruise)",
            braking_at / warp
        );
        // Not later than the coast distance (speed x lag), not absurdly early.
        assert!(
            (warp * TAU..=2.5 * warp).contains(&braking_at),
            "{warp}: {braking_at}"
        );
        assert!(
            tr[start..]
                .windows(2)
                .all(|w| w[1].speed <= w[0].speed + 0.02),
            "{warp}: speed rises while braking"
        );
        // It never passes the goal; it ends inside its stop radius, and slow.
        let end = tr.last().unwrap();
        assert!(end.speed < 0.5 && end.done, "{warp}: {end:?}");
        eprintln!("{warp}: stops {} short", end.dist);
        assert!(
            end.dist > 0.0 && end.dist <= near,
            "{warp}: {} short",
            end.dist
        );
        assert!(tr.iter().all(|s| s.z <= goal_z), "{warp}: overshoot");
    }
}

#[test]
fn braking_never_speeds_up_again_on_any_approach_angle() {
    for off in [0.0, 30.0, 90.0, 150.0] {
        let tr = fly(class(A.0, A.1), &params(), &[goal(off, 1200.0)], 30.0);
        let brake = tr
            .iter()
            .position(|s| s.dist < 1.6 * A.0 && s.speed > 0.85 * A.0)
            .expect("cruise reached");
        let rises = tr[brake..]
            .windows(2)
            .filter(|w| w[1].speed > w[0].speed + 0.05)
            .count();
        assert_eq!(rises, 0, "{off} deg");
    }
}

#[test]
fn a_scripted_warp_speed_is_the_cruise_and_below_impulse_only_aligned_legs_slow() {
    // Warp above impulse: cruise is that value on a straight leg.
    let tr = fly(class(36.0, 40.0), &params(), &[goal(0.0, 2000.0)], 20.0);
    let m = mean(
        tr.iter()
            .filter(|s| s.t > 5.0 && s.t < 15.0)
            .map(|s| s.speed),
    );
    assert!((m - 36.0).abs() < 0.5, "{m}");
    // Warp below impulse: the aligned leg is at that value, a turning start at impulse.
    let tr = fly(class(12.0, 40.0), &params(), &[goal(180.0, 2000.0)], 20.0);
    let turning = tr
        .iter()
        .filter(|s| s.t < 1.0)
        .map(|s| s.speed)
        .fold(0.0, f64::max);
    let m = mean(
        tr.iter()
            .filter(|s| s.t > 6.0 && s.t < 15.0)
            .map(|s| s.speed),
    );
    assert!(turning > 12.0 * 1.2, "{turning}");
    assert!((m - 12.0).abs() < 0.3, "{m}");
}

#[test]
fn a_borg_ship_spins_by_its_own_keys_and_is_always_warp_eligible() {
    let mut stats = class(A.0, A.1);
    stats.heading_close_for_warp = f(180.0);
    let tr = fly(stats, &borg(), &[goal(180.0, 1500.0)], 40.0);
    // By construction of the spin integrator: never above `maxRotationSpeed`.
    let spin = tr.iter().map(|s| s.omega.abs()).fold(0.0, f64::max);
    eprintln!("borg: max spin {spin}");
    assert!(spin <= BORG_SPIN.to_degrees() + 0.5, "{spin}");
    // Throttle is the multiplier from the start (no impulse phase): it reaches
    // 90% of cruise while still turning.
    let t90 = first(&tr, |s| s.speed >= 0.9 * A.0);
    assert!(tr.iter().find(|s| s.t >= t90).unwrap().err > 45.0, "{t90}");
    // And it does line up and arrive.
    assert!(tr.last().unwrap().done);
}

/// The closest the trace came to `p`.
fn closest(tr: &[S], p: Vec3) -> f64 {
    let (px, pz) = (num(p.x), num(p.z));
    tr.iter()
        .map(|s| ((s.x - px).powi(2) + (s.z - pz).powi(2)).sqrt())
        .fold(f64::MAX, f64::min)
}

/// A course that goes out 1500 units, turns by `turn` degrees at the corner
/// and goes 1500 more.
fn corner(turn: f64) -> [Vec3; 3] {
    let a = turn.to_radians();
    [
        pt(0.0, 1500.0),
        pt(-1500.0 * a.sin(), 1500.0 + 1500.0 * a.cos()),
        pt(-3000.0 * a.sin(), 1500.0 + 3000.0 * a.cos()),
    ]
}

/// Rule (STA64's own): a queued waypoint is handed over only once the craft
/// has reached the corner region, so it passes within the lead distance of
/// every waypoint, whatever the angle of the corner, and the course is still
/// flown through to its end.
#[test]
fn a_corner_of_any_angle_is_reached_before_the_course_goes_on() {
    for turn in [30.0, 78.0, 90.0, 135.0, 170.0] {
        let legs = corner(turn);
        let tr = fly(class(A.0, A.1), &params(), &legs, 120.0);
        let end = tr.last().unwrap();
        assert!(
            end.done && end.dist < 12.0 && end.speed < 0.5,
            "{turn}: {end:?}"
        );
        let c = closest(&tr, legs[0]);
        eprintln!("corner {turn}: closest {c}");
        assert!(c < LEAD, "{turn} degrees: corner missed by {c}");
    }
}

/// An out-and-back queue reaches its far point (the first hand-over dropped it
/// on the first tick) and comes back to the end.
#[test]
fn an_out_and_back_queue_reaches_the_far_point() {
    let legs = [pt(0.0, 1500.0), pt(0.0, 300.0)];
    let tr = fly(class(A.0, A.1), &params(), &legs, 90.0);
    let far = tr.iter().map(|s| s.z).fold(0.0, f64::max);
    assert!(far > 1500.0 - LEAD, "turned back at {far}");
    let end = tr.last().unwrap();
    assert!(end.done && end.dist < 12.0, "{end:?}");
    let legs = [pt(0.0, 900.0), pt(0.0, 1800.0), pt(0.0, 500.0)];
    let tr = fly(class(A.0, A.1), &params(), &legs, 120.0);
    assert!(closest(&tr, legs[1]) < LEAD);
    assert!(tr.last().unwrap().done);
}

/// A moving craft whose first waypoint lies far behind it and the next ahead
/// does not skip the far one: "behind the next leg and heading away" counts as
/// passed only near the waypoint.
#[test]
fn a_far_waypoint_behind_a_moving_craft_is_not_skipped() {
    let mut u = craft(class(A.0, A.1));
    u.speed = f(60.0);
    let legs = [pt(0.0, -2000.0), pt(0.0, 2000.0)];
    let tr = fly_from(u, &params(), &legs, 150.0);
    let c = closest(&tr, legs[0]);
    eprintln!("far point behind: closest {c}");
    assert!(c < LEAD, "the far waypoint was skipped: closest {c}");
    let end = tr.last().unwrap();
    assert!(end.done && end.dist < 12.0, "{end:?}");
}

#[test]
fn a_waypoint_flows_through_and_the_last_leg_stops() {
    let legs = [pt(0.0, 1500.0), pt(1500.0, 1500.0), pt(1500.0, 3000.0)];
    let tr = fly(class(A.0, A.1), &params(), &legs, 120.0);
    let end = tr.last().unwrap();
    assert!(end.done && end.dist < 12.0 && end.speed < 0.5, "{end:?}");
    assert!(closest(&tr, legs[0]) < LEAD);
    assert!(closest(&tr, legs[1]) < LEAD);
    // Never stops on the way: speed stays above impulse between the legs.
    let mid = tr
        .iter()
        .filter(|s| s.z > 600.0 && s.dist > 400.0)
        .map(|s| s.speed)
        .fold(f64::MAX, f64::min);
    assert!(mid > IMPULSE, "{mid}");
}

/// Rule (STA64's own): a goal inside the diameter of the circle the craft
/// turns on (and outside the arrival distance) counts as reached, and one just
/// outside it is turned onto and reached; neither stalls or circles until the
/// stuck-order watch. Mod data only (a stock `tooCloseToTurn` is larger).
#[test]
fn a_goal_abeam_near_the_turning_circle_is_reached_not_circled() {
    let diameter = 18.0;
    for (off, side) in [(10.0, 1.0), (10.0, -1.0), (25.0, 1.0), (25.0, -1.0)] {
        let mut stats = class(A.0, 3.0);
        stats.too_close_for_warp = f(100000.0);
        let tr = fly(stats, &params(), &[pt(-off * side, 0.0)], 30.0);
        let end = tr.last().unwrap();
        let t_done = first(&tr, |s| s.done);
        eprintln!("abeam {off} {side}: done at {t_done}, {end:?}");
        assert!(
            t_done < 20.0 && end.dist <= diameter + 0.5 && end.speed < 0.5,
            "{off}/{side}: {end:?}"
        );
    }
}

/// Mod data degrades safely: the turn authority curve and the exponential
/// helper stay in range at their edges.
#[test]
fn mod_data_edges_give_no_wild_turn_rates_and_no_wrap() {
    // exp_neg: one at and under zero, falling, zero from 23 on; no wrap.
    assert_eq!(a1::exp_neg(f(-3.0)), Fixed::ONE);
    assert_eq!(a1::exp_neg(Fixed::ZERO), Fixed::ONE);
    assert!((num(a1::exp_neg(f(1.0))) - (-1f64).exp()).abs() < 1e-3);
    assert!((num(a1::exp_neg(f(5.0))) - (-5f64).exp()).abs() < 1e-3);
    let mut last = 1.0;
    let mut x = 0.0;
    while x < 40.0 {
        let e = num(a1::exp_neg(f(x)));
        assert!((0.0..=1.0).contains(&e) && e <= last + 1e-6, "{x}: {e}");
        last = e;
        x += 0.25;
    }
    for big in [23, 24, 100, 5000, 1_000_000, 2_000_000_000] {
        assert_eq!(a1::exp_neg(Fixed::from_int(big)), Fixed::ZERO, "{big}");
    }
    // The authority and the turn rate: in range for any velocCoeffStill.
    let max = a1::MAX_TURN_RATE;
    for still in [-2.0, 0.0, 0.3, 0.5, 0.51, 0.9, 1.0, 5.0, 1.0e6] {
        for half in [0.0, 0.001, 0.7, 50.0] {
            let mut d = play();
            d.veloc_coeff_still = f(still);
            d.veloc_when_coeff_half = f(half);
            let mut prev = 0.0;
            for v in [0.0, 1.0, 5.0, 10.0, 20.0, 40.0, 80.0, 400.0, 100000.0] {
                let c = num(a1::authority(&d, f(v)));
                assert!(
                    (0.0..1.0 + 1e-9).contains(&c),
                    "still {still} half {half} v {v}: {c}"
                );
                let r = num(a1::full_turn_rate(&d, f(v)));
                assert!(
                    (0.0..=max + 1e-6).contains(&r),
                    "still {still} half {half} v {v}: rate {r}"
                );
                // No pole: the curve never falls as the speed rises.
                if half > 0.0 {
                    assert!(
                        c + 1e-6 >= prev,
                        "still {still} half {half} v {v}: {c} < {prev}"
                    );
                    prev = c;
                }
            }
        }
    }
    // A degenerate block flown for a while: finite, a unit heading, a bounded rate.
    for (still, radius, stiffness, roll) in [
        (0.9, 7.5, 1.5, 0.5),
        (0.0, 0.0001, 1.0e6, 1000.0),
        (5.0, 1.0e-9, 0.0, 0.0),
        (0.3, 1.0e9, -3.0, -1.0),
    ] {
        let mut p = params();
        p.play.veloc_coeff_still = f(still);
        p.play.turn_radius = f(radius);
        p.play.braccel_control_stiffness = f(stiffness);
        p.play.roll_rotation_speed = f(roll);
        let tr = fly(class(A.0, A.1), &p, &[goal(150.0, 1500.0)], 25.0);
        for s in &tr {
            assert!(
                s.omega.abs() <= max.to_degrees() + 1e-3,
                "{still}/{radius}: {s:?}"
            );
            assert!(
                s.speed >= 0.0 && s.speed <= A.0 + 1.0,
                "{still}/{radius}: {s:?}"
            );
            assert!(
                s.x.abs() < 5000.0 && s.z.abs() < 5000.0,
                "{still}/{radius}: {s:?}"
            );
        }
    }
}

/// Sim-level: the model switches on with parameters and with nothing else.
mod through_the_sim {
    use super::*;
    use crate::command::{Command, CommandKind, Order};
    use crate::movement::approach_tests;
    use crate::{Sim, WorldSource};
    use sta_content::{ContentDb, Profile};
    use sta_core::EntityId;
    use std::sync::Arc;

    fn empty(profile: Profile) -> ContentDb {
        let base = sta_content::fake::db();
        ContentDb::builder()
            .profile(profile)
            .features(base.features().clone())
            .build()
            .unwrap()
    }

    fn game(profile: Profile) -> (Sim, EntityId) {
        let base = sta_content::fake::db();
        let mut b = ContentDb::builder()
            .profile(profile)
            .features(base.features().clone());
        for w in base.weapons().cloned() {
            b = b.weapon(w);
        }
        for mut u in base.units().cloned() {
            if u.odf == "fx_frigate" || u.odf == "fx_cruiser" {
                u.movement = class(A.0, A.1);
            }
            b = b.unit(u);
        }
        let db = Arc::new(b.build().unwrap());
        let mut sim = approach_tests::sim_with(Arc::clone(&db));
        let s = sim
            .spawn_def(
                db.key("fx_frigate").unwrap(),
                PlayerId(1),
                pt(0.0, 0.0),
                (Fixed::ZERO, Fixed::ONE),
            )
            .unwrap();
        (sim, s)
    }

    fn run(sim: &mut Sim, s: EntityId, to: Vec3, secs: u32) -> Vec<(Vec3, Fixed, Fixed)> {
        let mut out = Vec::new();
        for i in 0..secs * TICK_HZ {
            let cmds = if i == 0 {
                vec![Command {
                    player: PlayerId(1),
                    seq: 0,
                    kind: CommandKind::Order(Order::Move {
                        units: vec![s],
                        to,
                        queue: false,
                        mode: None,
                    }),
                }]
            } else {
                vec![]
            };
            WorldSource::step(sim, &cmds);
            let u = sim.unit(s).unwrap();
            out.push((u.pos, u.speed, u.omega));
        }
        out
    }

    #[test]
    fn without_parameters_armada_i_flies_exactly_as_the_native_profile() {
        // Nothing in the seam: the content answers "not available".
        let db = empty(Profile::Sta1Classic);
        assert_eq!(Model::for_content(&db), Model::Native);
        assert!(a1::params(&db, ContentKey::NONE).is_none());
        let (mut a, sa) = game(Profile::Sta1Classic);
        let (mut b, sb) = game(Profile::Native);
        for to in [pt(0.0, 2000.0), pt(-900.0, -400.0)] {
            assert_eq!(
                run(&mut a, sa, to, 40)
                    .iter()
                    .map(|r| (r.0, r.1, r.2))
                    .collect::<Vec<_>>(),
                run(&mut b, sb, to, 40)
            );
        }
    }

    #[test]
    fn parameters_switch_the_model_on_for_armada_i_only() {
        let _g = seam::set(params());
        for (profile, want) in [
            (Profile::Sta1Classic, Model::A1),
            (Profile::Native, Model::Native),
            (Profile::Sta2Classic, Model::Smooth),
            (Profile::Fo4, Model::Smooth),
        ] {
            assert_eq!(Model::for_content(&empty(profile)), want, "{profile:?}");
        }
        // Model::of is unchanged.
        assert_eq!(Model::of(Profile::Sta1Classic), Model::Native);
    }

    #[test]
    fn a_plain_move_through_the_sim_cruises_at_warp_without_a_warp_job() {
        let _g = seam::set(params());
        let (mut sim, s) = game(Profile::Sta1Classic);
        let tr = run(&mut sim, s, pt(0.0, 3000.0), 20);
        assert!(sim.scripted.job(s).is_none(), "a warp job was made");
        let mid = num(tr[(10 * TICK_HZ) as usize].1);
        assert!((mid - A.0).abs() < 1.0, "{mid}");
    }

    fn advance(sim: &mut Sim, s: EntityId, secs: u32) -> Vec<f64> {
        (0..secs * TICK_HZ)
            .map(|_| {
                WorldSource::step(sim, &[]);
                num(sim.unit(s).unwrap().speed)
            })
            .collect()
    }

    fn set_warp(sim: &mut Sim, s: EntityId, w: i32) {
        use crate::command::Edit;
        let set = Command {
            player: PlayerId::SCRIPT,
            seq: 0,
            kind: CommandKind::Edit(Edit::WarpSpeed(s, Some(Fixed::from_int(w)))),
        };
        WorldSource::step(sim, &[set]);
    }

    /// The scripted warp value is the throttle multiplier, read every tick: no
    /// warp job is made (on main the SIM-64 job flies it), and a change in the
    /// middle of a leg is followed.
    #[test]
    fn the_scripted_warp_speed_of_the_sim_is_the_cruise_multiplier() {
        let _g = seam::set(params());
        let (mut sim, s) = game(Profile::Sta1Classic);
        set_warp(&mut sim, s, 25);
        let tr = run(&mut sim, s, pt(0.0, 6000.0), 12);
        assert!(sim.scripted.job(s).is_none(), "a warp job was made");
        assert!((num(tr.last().unwrap().1) - 25.0).abs() < 0.5);
        // Raised mid-move: the same leg goes on at the new value.
        set_warp(&mut sim, s, 50);
        assert!(sim.scripted.job(s).is_none(), "a warp job was made");
        let v = advance(&mut sim, s, 6);
        assert!((v.last().unwrap() - 50.0).abs() < 0.5, "{v:?}");
        // And lowered again.
        set_warp(&mut sim, s, 30);
        let v = advance(&mut sim, s, 6);
        assert!((v.last().unwrap() - 30.0).abs() < 0.5, "{v:?}");
        assert!(sim.scripted.job(s).is_none(), "a warp job was made");
    }

    /// The scripted-warp job is skipped for a craft the model flies and kept
    /// for one it has no parameters for.
    #[test]
    fn the_warp_job_is_skipped_per_unit() {
        let (mut sim, s) = game(Profile::Sta1Classic);
        let db = sim.content.clone().unwrap();
        let other = sim
            .spawn_def(
                db.key("fx_cruiser").unwrap(),
                PlayerId(1),
                pt(300.0, 0.0),
                (Fixed::ZERO, Fixed::ONE),
            )
            .unwrap();
        let _g = seam::set_only(params(), db.key("fx_frigate").unwrap());
        assert_eq!(Model::for_content(&db), Model::A1);
        WorldSource::step(
            &mut sim,
            &[Command {
                player: PlayerId(1),
                seq: 0,
                kind: CommandKind::Order(Order::Move {
                    units: vec![s, other],
                    to: pt(0.0, 5000.0),
                    queue: false,
                    mode: None,
                }),
            }],
        );
        assert!(sim.scripted.job(s).is_none(), "the A1 craft got a job");
        assert!(
            sim.scripted.job(other).is_some(),
            "the other craft lost its job"
        );
    }

    #[test]
    fn nested_seam_guards_restore_the_outer_block() {
        let db = empty(Profile::Sta1Classic);
        assert!(a1::params(&db, ContentKey::NONE).is_none());
        let outer = params();
        let mut inner = params();
        inner.play.impulse_speed = f(33.0);
        let g1 = seam::set(outer);
        {
            let _g2 = seam::set(inner);
            assert_eq!(a1::params(&db, ContentKey::NONE), Some(inner));
        }
        assert_eq!(a1::params(&db, ContentKey::NONE), Some(outer));
        drop(g1);
        assert!(a1::params(&db, ContentKey::NONE).is_none());
    }

    /// A run that exercises turn, cruise, a bend and the arrival, as a digest of
    /// the sim. With `queued`, the second order is queued behind the first and a
    /// third behind that (a corner, then the final leg).
    fn digest_run(queued: bool) -> u64 {
        let _g = seam::set(params());
        let (mut sim, s) = game(Profile::Sta1Classic);
        let mut h = crate::world::Fnv::new();
        let courses: Vec<Vec<Vec3>> = if queued {
            vec![vec![
                pt(0.0, 1500.0),
                pt(-1200.0, 1500.0),
                pt(-1200.0, -600.0),
            ]]
        } else {
            vec![vec![pt(0.0, 1500.0)], vec![pt(-1200.0, -600.0)]]
        };
        for course in courses {
            let mut cmds: Vec<Command> = course
                .iter()
                .enumerate()
                .map(|(i, to)| Command {
                    player: PlayerId(1),
                    seq: i as u32,
                    kind: CommandKind::Order(Order::Move {
                        units: vec![s],
                        to: *to,
                        queue: i > 0,
                        mode: None,
                    }),
                })
                .collect();
            // The queued course is long enough that 60 s ended before the final-leg brake.
            let secs = if queued { 120 } else { 60 };
            for i in 0..secs * TICK_HZ {
                let sent = if i == 0 {
                    std::mem::take(&mut cmds)
                } else {
                    vec![]
                };
                WorldSource::step(&mut sim, &sent);
                let u = sim.unit(s).unwrap();
                for v in [u.pos.x, u.pos.z, u.speed, u.omega] {
                    h.fixed(v);
                }
            }
            if queued {
                // The run covers the final-leg brake: at rest on the last point.
                let u = sim.unit(s).unwrap();
                let end = course[course.len() - 1];
                let (dx, dz) = (num(u.pos.x - end.x), num(u.pos.z - end.z));
                assert!(num(u.speed) < 1.0 && dx * dx + dz * dz < 60.0 * 60.0, "speed {} at ({dx}, {dz}) from the last point", num(u.speed));
            }
        }
        h.0
    }

    #[test]
    fn two_runs_agree_and_match_the_pinned_digests() {
        let got = [digest_run(false), digest_run(true)];
        assert_eq!(got, [digest_run(false), digest_run(true)]);
        assert_eq!(
            got,
            [A1_GOLDEN, A1_GOLDEN_QUEUED],
            "{:#x} {:#x}",
            got[0],
            got[1]
        );
    }

    /// Pinned with the authored parameter block; the queued one covers the
    /// hand-over and the final-leg logic.
    const A1_GOLDEN: u64 = 0xf0d2c437462e3c7d;
    const A1_GOLDEN_QUEUED: u64 = 0xd5e2_bd6b_9d6f_727a;
}

/// CON-52: the parameters come from content, for Armada I only. Authored ODFs
/// through an in-memory loader (no shipped values).
mod from_content {
    use super::*;
    use sta_content::{ContentDb, Overrides, Profile};
    use std::collections::BTreeMap;

    const FILES: [(&str, &str); 6] = [
        ("fastphys.odf", "classLabel = \"physics\"\nimpulseSpeed = 20\nfrontBackInertia = 8\nmaxReverseSpeed = 0.5\nturnRadius = 9\nvelocCoeffStill = 0.25\nvelocWhenCoeffHalf = 0.6\nbraccelControlStiffness = 2\nsy_yawAngleCoeff = 1.2\nsy_yawVelocCoeff = 0.2\nrollRotationSpeed = 0.6\n"),
        ("ship.odf", "[GameObjectClass]\nclassLabel = \"craft\"\nunitName = \"S\"\nphysicsFile = \"shipphys.odf\"\n"),
        ("shipphys.odf", "classLabel = \"physics\"\nimpulseSpeed = 20\nwarpSpeed = 60\ntooCloseToTurn = 40\npathLeadDistance = 120\n"),
        ("drone.odf", "[GameObjectClass]\nclassLabel = \"craft\"\nunitName = \"D\"\nphysicsFile = \"dronephys.odf\"\n"),
        ("dronephys.odf", "classLabel = \"physics\"\nimpulseSpeed = 20\nborgPhysics = 1\nmaxRotationSpeed = 0.4\n"),
        ("yard.odf", "[GameObjectClass]\nclassLabel = \"shipyard\"\nunitName = \"Y\"\n"),
    ];

    fn db(profile: Profile) -> ContentDb {
        let map: BTreeMap<String, &str> = FILES.iter().map(|(n, t)| ((*n).to_owned(), *t)).collect();
        let stems: Vec<String> = ["ship", "drone", "yard"].iter().map(|s| (*s).to_owned()).collect();
        sta_content::odf::load_with(profile, &Overrides::default(), &stems, &mut |n, v| {
            let n = if n.ends_with(".odf") { n.to_owned() } else { format!("{n}.odf") };
            if v == 0 { map.get(&n.to_ascii_lowercase()).map(|t| (*t).to_owned()) } else { None }
        })
        .unwrap()
        .0
    }

    #[test]
    fn armada_i_content_switches_the_model_on_for_craft_only() {
        let db = db(Profile::Sta1Classic);
        assert_eq!(Model::for_content(&db), Model::A1);
        let key = |n: &str| db.key(n).unwrap();
        let ship = a1::params(&db, key("ship")).expect("a craft has parameters");
        assert!(ship.borg.is_none());
        let spin = a1::params(&db, key("drone")).unwrap().borg.map(|b| num(b.max_rotation_speed));
        assert!(spin.is_some_and(|v| (v - 0.4).abs() < 1e-6), "{spin:?}");
        assert!(a1::params(&db, key("yard")).is_none(), "a structure keeps the old movement");
        assert!(a1::params(&db, ContentKey::NONE).is_some());
    }

    #[test]
    fn other_profiles_stay_on_their_models() {
        for (profile, model) in [(Profile::Fo4, Model::Smooth), (Profile::Sta2Classic, Model::Smooth), (Profile::Native, Model::Native)] {
            let db = db(profile);
            assert_eq!(Model::for_content(&db), model);
            assert!(a1::params(&db, db.key("ship").unwrap()).is_none());
        }
    }

    #[test]
    fn a_craft_with_content_parameters_reaches_its_goal() {
        let db = db(Profile::Sta1Classic);
        let k = db.key("ship").unwrap();
        let p = a1::params(&db, k).unwrap();
        let stats = db.unit(k).unwrap().movement;
        for off in [0.0, 90.0, 180.0] {
            let tr = fly(stats, &p, &[goal(off, 900.0)], 150.0);
            let last = tr.last().unwrap();
            assert!(last.done && last.dist < 60.0, "{off} degrees: done {} at {}", last.done, last.dist);
        }
    }
}
