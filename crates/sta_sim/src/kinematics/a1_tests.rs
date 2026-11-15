//! The Armada I model (SIM-66): the rules' targets, each with its evidence
//! grade (measured / read from the executable, static / STA64's own stand-in).
//!
//! Parameter blocks are authored inputs through the seam: the key names and
//! plausible values for the play physics and three ship classes. The original
//! has an input latency of 0.1 to 0.2 s that the sim does not (an order is
//! obeyed on the tick it is given), so a time-to-reach window is widened by
//! 0.2 s on the low side only, and the mean speed of a phase that starts at the
//! order is taken over the phase preceded by that latency. Rates, speeds and
//! distances get no widening beyond rounding. Checks that hold by construction
//! of the integrator (and so would pass on a wrong model too) are labelled so;
//! the emergent ones are the orders, the corners and the arrival.

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

/// The play physics' keys with the stock values (the file the executable names
/// `fastphys.odf`).
pub(crate) fn play() -> A1Dynamics {
    A1Dynamics {
        impulse_speed: f(20.0),
        front_back_inertia: f(7.5),
        max_reverse_speed: f(4.0),
        turn_radius: f(7.5),
        veloc_coeff_still: f(0.3),
        veloc_when_coeff_half: f(0.7),
        braccel_control_stiffness: f(1.5),
        yaw_angle_coeff: f(1.0),
        yaw_veloc_coeff: f(0.15),
        roll_rotation_speed: f(0.5),
    }
}

pub(crate) fn params() -> A1Params {
    A1Params {
        play: play(),
        borg: None,
    }
}

fn borg() -> A1Params {
    A1Params {
        play: play(),
        borg: Some(A1Borg {
            max_rotation_speed: f(0.75),
            rotation_acceleration: f(2.0),
            rotation_stiffness: f(1.0),
            too_slow_to_turn: f(5.0),
        }),
    }
}

/// A ship class: impulse 20, own `warpSpeed` and `tooCloseToTurn`.
pub(crate) fn class(warp: f64, near: f64) -> MoveStats {
    MoveStats {
        warp_speed: f(warp),
        too_close_to_turn: f(near),
        path_lead: f(150.0),
        too_close_for_warp: f(10.0),
        heading_close_for_warp: f(30.0),
        ..MoveStats::from_f32(20.0, 1.0, 1.0, 1.0, 1.0)
    }
}

const CRUISER: (f64, f64) = (80.0, 50.0);
const DESTROYER: (f64, f64) = (100.0, 40.0);
const CONSTRUCTION: (f64, f64) = (50.0, 50.0);

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
    let mut u = craft(stats);
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

#[test]
fn cruise_is_the_ships_own_warp_speed_held_on_a_straight_leg() {
    for (warp, near) in [CRUISER, DESTROYER, CONSTRUCTION] {
        let tr = fly(class(warp, near), &params(), &[goal(0.0, 4000.0)], 40.0);
        // By construction (the lag holds the throttle `warpSpeed / impulse`).
        // Target (measured): the ship's own warpSpeed, within 1%.
        let m = mean(
            tr.iter()
                .filter(|s| s.t > 6.0 && s.t < 30.0)
                .map(|s| s.speed),
        );
        assert!((m - warp).abs() < 0.01 * warp, "{warp}: {m}");
        let lo = tr
            .iter()
            .filter(|s| s.t > 6.0 && s.t < 30.0)
            .map(|s| s.speed)
            .fold(f64::MAX, f64::min);
        assert!(lo > 0.99 * warp, "{warp}: dips to {lo}");
    }
}

#[test]
fn speed_from_rest_is_one_first_order_lag_for_every_class() {
    let mut at_tau = Vec::new();
    for (warp, near) in [CRUISER, DESTROYER, CONSTRUCTION] {
        let tr = fly(class(warp, near), &params(), &[goal(0.0, 4000.0)], 3.0);
        let share = |t: f64| tr.iter().find(|s| s.t >= t - 1e-6).unwrap().speed / warp;
        // By construction of the lag. Targets (measured): one constant
        // 0.37 s for every class, 90% of cruise 0.94 to 0.96 s after the order.
        assert!(
            (share(0.375) - 0.632).abs() < 0.03,
            "{warp}: {}",
            share(0.375)
        );
        assert!((share(1.1) - 0.95).abs() < 0.02, "{warp}: {}", share(1.1));
        let t90 = first(&tr, |s| s.speed >= 0.9 * warp);
        assert!((0.80..=0.97).contains(&t90), "{warp}: 90% at {t90}");
        at_tau.push(share(0.375));
    }
    // Shared physics: the shares are equal across classes.
    assert!(
        at_tau.windows(2).all(|w| (w[0] - w[1]).abs() < 0.005),
        "{at_tau:?}"
    );
}

#[test]
fn a_180_degree_order_turns_and_gets_up_to_cruise_on_the_measured_times() {
    let tr = fly(
        class(CRUISER.0, CRUISER.1),
        &params(),
        &[goal(180.0, 1000.0)],
        8.0,
    );
    let t10 = first(&tr, |s| s.err < 10.0);
    let t90 = first(&tr, |s| s.speed >= 72.0);
    eprintln!("180: within 10 deg {t10}, 90% {t90}");
    // Targets (measured, low side widened by the latency): within 10 degrees
    // after 1.8 to 2.0 s, 90% of cruise after 2.1 to 2.45 s.
    assert!((1.6..=2.0).contains(&t10), "{t10}");
    assert!((1.9..=2.45).contains(&t90), "{t90}");
    // Peak turn rate over 0.2 s windows; target (measured): 130 to 145 deg/s
    // sustained. A rate is not changed by the latency, so no widening.
    let w = 6;
    let peak = (0..tr.len() - w)
        .map(|i| mean(tr[i..i + w].iter().map(|s| s.omega.abs())))
        .fold(0.0, f64::max);
    eprintln!("180: peak turn rate {peak}");
    assert!((130.0..=145.0).contains(&peak), "{peak}");
    // Reached about 0.8 s after the order.
    let at = tr.iter().find(|s| s.t >= 0.8 - 1e-6).unwrap().omega.abs();
    assert!(at > 0.7 * peak, "{at} at 0.8 s");
}

#[test]
fn a_90_degree_order_from_rest_on_the_measured_times() {
    let tr = fly(
        class(DESTROYER.0, DESTROYER.1),
        &params(),
        &[goal(90.0, 1500.0)],
        6.0,
    );
    let t10 = first(&tr, |s| s.err < 10.0);
    let t90 = first(&tr, |s| s.speed >= 90.0);
    eprintln!("90: within 10 deg {t10}, 90% {t90}");
    // Targets (measured, low side widened by the latency): within 10 degrees
    // after 1.3 s, 90% of cruise after 1.5 s (rounded up by 0.05 s).
    assert!((1.1..=1.35).contains(&t10), "{t10}");
    assert!((1.3..=1.55).contains(&t90), "{t90}");
}

#[test]
fn the_ship_does_not_pivot_on_the_spot_it_moves_while_turning() {
    for (warp, near) in [CRUISER, DESTROYER, CONSTRUCTION] {
        let tr = fly(class(warp, near), &params(), &[goal(178.0, 1500.0)], 4.0);
        let t45 = first(&tr, |s| s.err < 45.0);
        let turning: Vec<&S> = tr.iter().filter(|s| s.t < t45).collect();
        // The mean over the phase from the order to 45 degrees, as the original
        // measured it from the order: a stand-still of its input latency (0.1 to
        // 0.2 s) comes first, so the sim's mean, spread over that longer phase,
        // is a range, and the measured range must contain a value of it.
        let sum = turning.iter().map(|s| s.speed).sum::<f64>();
        let over = |lat: f64| sum / (turning.len() as f64 + lat * TICK_HZ as f64);
        let (m_lo, m_hi) = (over(0.2), over(0.1));
        let hi = turning.iter().map(|s| s.speed).fold(0.0, f64::max);
        eprintln!("{warp}: turning phase to {t45} s, mean {m_lo} to {m_hi}, max {hi}");
        // Targets (measured): mean 13.8 to 16.4 u/s on a tight arc, never above
        // impulse 20 (its throttle is 1 there).
        assert!(
            m_hi >= 13.8 && m_lo <= 16.4,
            "{warp}: mean {m_lo} to {m_hi}"
        );
        assert!(hi <= 20.0 + 0.01, "{warp}: max {hi}");
        // It moved from the first tenth of a second on: no stationary pivot.
        assert!(tr[5].speed > 3.0, "{warp}: {}", tr[5].speed);
        // The arc is tight: well inside 30 units across for the whole turn.
        let w = turning
            .iter()
            .map(|s| (s.x * s.x + s.z * s.z).sqrt())
            .fold(0.0, f64::max);
        assert!(w < 40.0, "{warp}: arc {w}");
    }
}

#[test]
fn a_path_bend_at_cruise_keeps_most_of_the_speed() {
    // A 78 degree bend in the course, from a ship already at cruise.
    let a = 78f64.to_radians();
    let legs = [
        pt(0.0, 3000.0),
        pt(-3000.0 * a.sin(), 3000.0 + 3000.0 * a.cos()),
    ];
    let tr = fly(class(DESTROYER.0, DESTROYER.1), &params(), &legs, 80.0);
    let near_bend =
        |s: &&S| s.t > 8.0 && ((s.x * s.x + (s.z - 3000.0) * (s.z - 3000.0)).sqrt()) < 300.0;
    let at_bend = tr
        .iter()
        .filter(near_bend)
        .map(|s| s.speed)
        .fold(f64::MAX, f64::min);
    eprintln!("bend: slowest {at_bend}");
    // Target (measured): about 95% of cruise at a 78 degree bend.
    assert!(at_bend > 0.92 * 100.0, "{at_bend}");
    // And it does follow the course round the bend.
    assert!(tr.last().unwrap().done, "{:?}", tr.last());
}

#[test]
fn stopping_at_cruise_coasts_speed_times_the_lag() {
    // By construction of the lag. Target (measured): a cruiser at cruise, then
    // no target, coasts speed x tau (30 u at 80).
    let mut u = craft(class(CRUISER.0, CRUISER.1));
    u.speed = f(80.0);
    let p = params();
    let start = u.pos.z;
    for _ in 0..(3 * TICK_HZ) {
        a1::step(&mut u, &p, &[], None);
    }
    let coast = num(u.pos.z - start);
    eprintln!("coast {coast}");
    assert!((29.0..=31.0).contains(&coast), "{coast}");
    assert!(u.speed < f(0.5), "{:?}", u.speed);
}

/// Braking start, arrival and overshoot on a long straight leg.
#[test]
fn arrival_brakes_from_about_one_and_a_half_cruise_out_and_stops_short_without_overshoot() {
    for (warp, near, goal_z) in [
        (CRUISER.0, CRUISER.1, 2000.0),
        (DESTROYER.0, DESTROYER.1, 2400.0),
    ] {
        let tr = fly(class(warp, near), &params(), &[pt(0.0, goal_z)], 40.0);
        // The braking starts where the speed first falls under its cruise value.
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
        // Target (measured): braking starts about 1.6 x cruise before the goal.
        // Deviation, stated: this detector reads the speed, which falls half a
        // percent under cruise only after the throttle drops (the lag, about
        // 0.1 x cruise of distance), so it sees 1.4 to 1.6, not 1.6.
        assert!(
            (1.40 * warp..=1.65 * warp).contains(&braking_at),
            "{warp}: {braking_at}"
        );
        // No speed rise after braking starts.
        assert!(
            tr[start..]
                .windows(2)
                .all(|w| w[1].speed <= w[0].speed + 0.02),
            "{warp}: speed rises while braking"
        );
        // It never passes the goal and ends a few units short (target, measured: 5.5 u).
        let end = tr.last().unwrap();
        assert!(end.speed < 0.5 && end.done, "{warp}: {end:?}");
        let short = end.dist;
        eprintln!("{warp}: stops {short} short");
        assert!((4.5..=7.0).contains(&short), "{warp}: {short} short");
        assert!(tr.iter().all(|s| s.z <= goal_z), "{warp}: overshoot");
    }
}

#[test]
fn braking_never_speeds_up_again_on_any_approach_angle() {
    for off in [0.0, 30.0, 90.0, 150.0] {
        let tr = fly(
            class(CRUISER.0, CRUISER.1),
            &params(),
            &[goal(off, 1200.0)],
            30.0,
        );
        let brake = tr
            .iter()
            .position(|s| s.dist < 1.6 * 80.0 && s.speed > 70.0)
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
    // Warp 25: cruise 25 on a straight leg.
    let tr = fly(class(25.0, 50.0), &params(), &[goal(0.0, 2000.0)], 20.0);
    let m = mean(
        tr.iter()
            .filter(|s| s.t > 5.0 && s.t < 15.0)
            .map(|s| s.speed),
    );
    assert!((m - 25.0).abs() < 0.5, "{m}");
    // Warp 10 (below impulse 20): aligned legs at 10, a turning start at impulse.
    let tr = fly(class(10.0, 50.0), &params(), &[goal(180.0, 2000.0)], 20.0);
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
    assert!(turning > 12.0, "{turning}");
    assert!((m - 10.0).abs() < 0.3, "{m}");
}

#[test]
fn a_borg_ship_spins_by_its_own_keys_and_is_always_warp_eligible() {
    let mut stats = class(80.0, 50.0);
    stats.heading_close_for_warp = f(180.0);
    let tr = fly(stats, &borg(), &[goal(180.0, 1500.0)], 40.0);
    // Spin never above maxRotationSpeed (0.75 rad/s = 43 deg/s): by
    // construction of the spin integrator (read from the executable, static).
    let spin = tr.iter().map(|s| s.omega.abs()).fold(0.0, f64::max);
    eprintln!("borg: max spin {spin}");
    assert!(spin <= 43.0 + 0.5, "{spin}");
    // Throttle is the multiplier from the start (no impulse phase): it reaches
    // 90% of cruise within the lag, while still turning.
    let t90 = first(&tr, |s| s.speed >= 72.0);
    assert!(t90 < 2.0, "{t90}");
    assert!(tr.iter().find(|s| s.t >= t90).unwrap().err > 45.0);
    // It does line up and arrive.
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
/// has reached the corner region, so it passes within the lead distance
/// (150 here) of every waypoint, whatever the angle of the corner, and the
/// course is still flown through to its end.
#[test]
fn a_corner_of_any_angle_is_reached_before_the_course_goes_on() {
    for turn in [30.0, 78.0, 90.0, 135.0, 170.0] {
        let legs = corner(turn);
        let tr = fly(class(CRUISER.0, CRUISER.1), &params(), &legs, 120.0);
        let end = tr.last().unwrap();
        assert!(
            end.done && end.dist < 12.0 && end.speed < 0.5,
            "{turn}: {end:?}"
        );
        let c = closest(&tr, legs[0]);
        eprintln!("corner {turn}: closest {c}");
        assert!(c < 150.0, "{turn} degrees: corner missed by {c}");
    }
}

/// An out-and-back queue reaches its far point (the old hand-over dropped it on
/// the first tick) and comes back to the end.
#[test]
fn an_out_and_back_queue_reaches_the_far_point() {
    let legs = [pt(0.0, 1500.0), pt(0.0, 300.0)];
    let tr = fly(class(CRUISER.0, CRUISER.1), &params(), &legs, 90.0);
    let far = tr.iter().map(|s| s.z).fold(0.0, f64::max);
    eprintln!("out and back: far {far}");
    assert!(far > 1500.0 - 150.0, "turned back at {far}");
    let end = tr.last().unwrap();
    assert!(end.done && end.dist < 12.0, "{end:?}");
    // Three points, the middle one the far end.
    let legs = [pt(0.0, 900.0), pt(0.0, 1800.0), pt(0.0, 500.0)];
    let tr = fly(class(CRUISER.0, CRUISER.1), &params(), &legs, 120.0);
    assert!(closest(&tr, legs[1]) < 150.0);
    assert!(tr.last().unwrap().done);
}

#[test]
fn a_waypoint_flows_through_and_the_last_leg_stops() {
    let legs = [pt(0.0, 1500.0), pt(1500.0, 1500.0), pt(1500.0, 3000.0)];
    let tr = fly(class(CRUISER.0, CRUISER.1), &params(), &legs, 120.0);
    let end = tr.last().unwrap();
    assert!(end.done && end.dist < 12.0 && end.speed < 0.5, "{end:?}");
    // It reaches the corner region before going on.
    assert!(closest(&tr, legs[0]) < 150.0);
    assert!(closest(&tr, legs[1]) < 150.0);
    // Never stops on the way: speed stays above impulse between the legs.
    let mid = tr
        .iter()
        .filter(|s| s.z > 600.0 && s.dist > 400.0)
        .map(|s| s.speed)
        .fold(f64::MAX, f64::min);
    assert!(mid > 20.0, "{mid}");
}

/// Rule (STA64's own): a goal inside the diameter of the circle the craft
/// turns on (and outside the arrival distance) counts as reached, and one just
/// outside it is turned onto and reached; neither stalls or circles until the
/// stuck-order watch. Mod data only (the stock `tooCloseToTurn` is larger).
#[test]
fn a_goal_abeam_near_the_turning_circle_is_reached_not_circled() {
    for (off, side) in [(10.0, 1.0), (10.0, -1.0), (20.0, 1.0), (20.0, -1.0)] {
        // The circle is 7.5 across its radius, 15 across; tooCloseToTurn 3.
        let mut stats = class(80.0, 3.0);
        stats.too_close_for_warp = f(100000.0);
        let tr = fly(stats, &params(), &[pt(-off * side, 0.0)], 30.0);
        let end = tr.last().unwrap();
        let t_done = first(&tr, |s| s.done);
        eprintln!("abeam {off} {side}: done at {t_done}, {end:?}");
        assert!(
            t_done < 20.0 && end.dist <= 15.0 + 0.5 && end.speed < 0.5,
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
        let tr = fly(class(80.0, 50.0), &p, &[goal(150.0, 1500.0)], 25.0);
        for s in &tr {
            assert!(
                s.omega.abs() <= max.to_degrees() + 1e-3,
                "{still}/{radius}: {s:?}"
            );
            assert!(
                s.speed >= 0.0 && s.speed <= 80.0 + 1.0,
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
                u.movement = class(80.0, 50.0);
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
        assert!((mid - 80.0).abs() < 1.0, "{mid}");
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

    /// A run that exercises turn, cruise, a bend and the arrival, as a digest of the sim.
    fn digest_run() -> u64 {
        let _g = seam::set(params());
        let (mut sim, s) = game(Profile::Sta1Classic);
        let mut h = crate::world::Fnv::new();
        for to in [pt(0.0, 1500.0), pt(-1200.0, -600.0)] {
            for r in run(&mut sim, s, to, 45) {
                for v in [r.0.x, r.0.z, r.1, r.2] {
                    h.fixed(v);
                }
            }
        }
        h.0
    }

    #[test]
    fn two_runs_agree_and_match_the_pinned_digest() {
        let (a, b) = (digest_run(), digest_run());
        assert_eq!(a, b);
        assert_eq!(a, A1_GOLDEN, "{a:#x}");
    }

    const A1_GOLDEN: u64 = 0xfba7b5152d19676f;
}
