//! The Armada I model (SIM-66) against what was measured on the original.
//!
//! Parameter blocks are authored inputs through the seam: the key names and
//! the stock values of the play physics and of the three ship classes. The
//! original's timings below include an input latency of 0.1 to 0.2 s that the
//! sim does not have (an order is obeyed on the tick it is given), so the
//! windows are the measured ranges widened by about 0.2 s on the low side.

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
    A1Params { play: play(), borg: None }
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
        Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: Fixed::ZERO },
        (Fixed::ZERO, Fixed::ONE),
        stats,
    )
}

fn pt(x: f64, z: f64) -> Vec3 {
    Vec3 { x: f(x), y: Fixed::ZERO, z: f(z) }
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
        let err = (hx * gz - hz * gx).atan2(hx * gx + hz * gz).to_degrees().abs();
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
        // Measured: 80.0, 99.6 to 99.9 and 50.0 over 4.5 to 27 s legs.
        let m = mean(tr.iter().filter(|s| s.t > 6.0 && s.t < 30.0).map(|s| s.speed));
        assert!((m - warp).abs() < 0.01 * warp, "{warp}: {m}");
        let lo = tr.iter().filter(|s| s.t > 6.0 && s.t < 30.0).map(|s| s.speed).fold(f64::MAX, f64::min);
        assert!(lo > 0.99 * warp, "{warp}: dips to {lo}");
    }
}

#[test]
fn speed_from_rest_is_one_first_order_lag_for_every_class() {
    let mut at_tau = Vec::new();
    for (warp, near) in [CRUISER, DESTROYER, CONSTRUCTION] {
        let tr = fly(class(warp, near), &params(), &[goal(0.0, 4000.0)], 3.0);
        let share = |t: f64| tr.iter().find(|s| s.t >= t - 1e-6).unwrap().speed / warp;
        // Measured: tau 0.365 to 0.375 s; 90% of cruise 0.94 to 0.96 s after the order.
        assert!((share(0.375) - 0.632).abs() < 0.03, "{warp}: {}", share(0.375));
        assert!((share(1.1) - 0.95).abs() < 0.02, "{warp}: {}", share(1.1));
        let t90 = first(&tr, |s| s.speed >= 0.9 * warp);
        assert!((0.80..=1.0).contains(&t90), "{warp}: 90% at {t90}");
        at_tau.push(share(0.375));
    }
    // Shared physics: the shares are equal across classes.
    assert!(at_tau.windows(2).all(|w| (w[0] - w[1]).abs() < 0.005), "{at_tau:?}");
}

#[test]
fn a_180_degree_order_turns_and_gets_up_to_cruise_on_the_measured_times() {
    let tr = fly(class(CRUISER.0, CRUISER.1), &params(), &[goal(180.0, 1000.0)], 8.0);
    let t10 = first(&tr, |s| s.err < 10.0);
    let t90 = first(&tr, |s| s.speed >= 72.0);
    eprintln!("180: within 10 deg {t10}, 90% {t90}");
    // Measured: within 10 degrees after 1.8 to 2.0 s, 90% of cruise after 2.1 to 2.45 s.
    assert!((1.6..=2.2).contains(&t10), "{t10}");
    assert!((1.9..=2.6).contains(&t90), "{t90}");
    // Peak turn rate over 0.2 s windows (measured 130 to 145 deg/s sustained).
    let w = 6;
    let peak = (0..tr.len() - w).map(|i| mean(tr[i..i + w].iter().map(|s| s.omega.abs()))).fold(0.0, f64::max);
    eprintln!("180: peak turn rate {peak}");
    assert!((125.0..=160.0).contains(&peak), "{peak}");
    // Reached about 0.8 s after the order.
    let at = tr.iter().find(|s| s.t >= 0.8 - 1e-6).unwrap().omega.abs();
    assert!(at > 0.7 * peak, "{at} at 0.8 s");
}

#[test]
fn a_90_degree_order_from_rest_on_the_measured_times() {
    let tr = fly(class(DESTROYER.0, DESTROYER.1), &params(), &[goal(90.0, 1500.0)], 6.0);
    let t10 = first(&tr, |s| s.err < 10.0);
    let t90 = first(&tr, |s| s.speed >= 90.0);
    eprintln!("90: within 10 deg {t10}, 90% {t90}");
    // Measured: within 10 degrees after 1.3 s, 90% of cruise after 1.5 s.
    assert!((1.1..=1.6).contains(&t10), "{t10}");
    assert!((1.3..=1.9).contains(&t90), "{t90}");
}

#[test]
fn the_ship_does_not_pivot_on_the_spot_it_moves_while_turning() {
    for (warp, near) in [CRUISER, DESTROYER, CONSTRUCTION] {
        let tr = fly(class(warp, near), &params(), &[goal(178.0, 1500.0)], 4.0);
        let t45 = first(&tr, |s| s.err < 45.0);
        let turning: Vec<&S> = tr.iter().filter(|s| s.t > 0.2 && s.t < t45).collect();
        let m = mean(turning.iter().map(|s| s.speed));
        let hi = turning.iter().map(|s| s.speed).fold(0.0, f64::max);
        eprintln!("{warp}: turning phase to {t45} s, mean {m}, max {hi}");
        // Measured: 13 to 17 u/s mean on a tight arc, at most 19 to 20.
        assert!((13.0..=18.0).contains(&m), "{warp}: mean {m}");
        assert!(hi <= 20.5, "{warp}: max {hi}");
        // It moved from the first tenth of a second on: no stationary pivot.
        assert!(tr[5].speed > 3.0, "{warp}: {}", tr[5].speed);
        // The arc is tight: well inside 30 units across for the whole turn.
        let w = turning.iter().map(|s| (s.x * s.x + s.z * s.z).sqrt()).fold(0.0, f64::max);
        assert!(w < 40.0, "{warp}: arc {w}");
    }
}

#[test]
fn a_path_bend_at_cruise_keeps_most_of_the_speed() {
    // A 78 degree bend in the course, from a ship already at cruise.
    let a = 78f64.to_radians();
    let legs = [pt(0.0, 3000.0), pt(-3000.0 * a.sin(), 3000.0 + 3000.0 * a.cos())];
    let tr = fly(class(DESTROYER.0, DESTROYER.1), &params(), &legs, 80.0);
    let near_bend = |s: &&S| s.t > 8.0 && ((s.x * s.x + (s.z - 3000.0) * (s.z - 3000.0)).sqrt()) < 300.0;
    let at_bend = tr.iter().filter(near_bend).map(|s| s.speed).fold(f64::MAX, f64::min);
    eprintln!("bend: slowest {at_bend}");
    // Measured: about 95% of cruise at a 78 degree bend.
    assert!(at_bend > 0.85 * 100.0, "{at_bend}");
    // And it does follow the course round the bend.
    assert!(tr.last().unwrap().done, "{:?}", tr.last());
}

#[test]
fn stopping_at_cruise_coasts_speed_times_the_lag() {
    // A cruiser at cruise, then no target: speed x tau (measured 30 u at 80).
    let mut u = craft(class(CRUISER.0, CRUISER.1));
    u.speed = f(80.0);
    let p = params();
    let start = u.pos.z;
    for _ in 0..(3 * TICK_HZ) {
        a1::step(&mut u, &p, &[], None);
    }
    let coast = num(u.pos.z - start);
    eprintln!("coast {coast}");
    assert!((27.0..=33.0).contains(&coast), "{coast}");
    assert!(u.speed < f(0.5), "{:?}", u.speed);
}

/// Braking start, arrival and overshoot on a long straight leg.
#[test]
fn arrival_brakes_from_about_one_and_a_half_cruise_out_and_stops_short_without_overshoot() {
    for (warp, near, goal_z) in [(CRUISER.0, CRUISER.1, 2000.0), (DESTROYER.0, DESTROYER.1, 2400.0)] {
        let tr = fly(class(warp, near), &params(), &[pt(0.0, goal_z)], 40.0);
        // The braking starts where the speed first falls under its cruise value.
        let peak = tr.iter().map(|s| s.speed).fold(0.0, f64::max);
        let top = tr.iter().position(|s| s.speed >= 0.995 * warp).expect("reaches cruise");
        let start = tr[top..].iter().position(|s| s.speed < 0.995 * peak).map(|i| i + top).expect("brakes");
        let braking_at = tr[start].dist;
        eprintln!("{warp}: braking starts {braking_at} out ({} x cruise)", braking_at / warp);
        // Measured: about 1.6 x cruise before the goal (127 u at 80, 157 at 100).
        // The speed falls half a percent under cruise a little after the throttle drops.
        assert!((1.35 * warp..=1.75 * warp).contains(&braking_at), "{warp}: {braking_at}");
        // No speed rise after braking starts.
        assert!(tr[start..].windows(2).all(|w| w[1].speed <= w[0].speed + 0.02), "{warp}: speed rises while braking");
        // It never passes the goal and ends a few units short (measured 5.5 u).
        let end = tr.last().unwrap();
        assert!(end.speed < 0.5 && end.done, "{warp}: {end:?}");
        let short = end.dist;
        eprintln!("{warp}: stops {short} short");
        assert!((2.0..=12.0).contains(&short), "{warp}: {short} short");
        assert!(tr.iter().all(|s| s.z <= goal_z), "{warp}: overshoot");
    }
}

#[test]
fn braking_never_speeds_up_again_on_any_approach_angle() {
    for off in [0.0, 30.0, 90.0, 150.0] {
        let tr = fly(class(CRUISER.0, CRUISER.1), &params(), &[goal(off, 1200.0)], 30.0);
        let brake = tr.iter().position(|s| s.dist < 1.6 * 80.0 && s.speed > 70.0).expect("cruise reached");
        let rises = tr[brake..].windows(2).filter(|w| w[1].speed > w[0].speed + 0.05).count();
        assert_eq!(rises, 0, "{off} deg");
    }
}

#[test]
fn a_scripted_warp_speed_is_the_cruise_and_below_impulse_only_aligned_legs_slow() {
    // Warp 25: cruise 25 on a straight leg.
    let tr = fly(class(25.0, 50.0), &params(), &[goal(0.0, 2000.0)], 20.0);
    let m = mean(tr.iter().filter(|s| s.t > 5.0 && s.t < 15.0).map(|s| s.speed));
    assert!((m - 25.0).abs() < 0.5, "{m}");
    // Warp 10 (below impulse 20): aligned legs at 10, a turning start at impulse.
    let tr = fly(class(10.0, 50.0), &params(), &[goal(180.0, 2000.0)], 20.0);
    let turning = tr.iter().filter(|s| s.t < 1.0).map(|s| s.speed).fold(0.0, f64::max);
    let m = mean(tr.iter().filter(|s| s.t > 6.0 && s.t < 15.0).map(|s| s.speed));
    assert!(turning > 12.0, "{turning}");
    assert!((m - 10.0).abs() < 0.3, "{m}");
}

#[test]
fn a_borg_ship_spins_by_its_own_keys_and_is_always_warp_eligible() {
    let mut stats = class(80.0, 50.0);
    stats.heading_close_for_warp = f(180.0);
    let tr = fly(stats, &borg(), &[goal(180.0, 1500.0)], 40.0);
    // Spin never above maxRotationSpeed (0.75 rad/s = 43 deg/s).
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

#[test]
fn a_waypoint_flows_through_and_the_last_leg_stops() {
    let legs = [pt(0.0, 1500.0), pt(1500.0, 1500.0), pt(1500.0, 3000.0)];
    let tr = fly(class(CRUISER.0, CRUISER.1), &params(), &legs, 120.0);
    let end = tr.last().unwrap();
    assert!(end.done && end.dist < 12.0 && end.speed < 0.5, "{end:?}");
    // Never stops on the way: speed stays above impulse between the legs.
    let mid = tr.iter().filter(|s| s.z > 600.0 && s.dist > 400.0).map(|s| s.speed).fold(f64::MAX, f64::min);
    assert!(mid > 20.0, "{mid}");
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
        ContentDb::builder().profile(profile).features(base.features().clone()).build().unwrap()
    }

    fn game(profile: Profile) -> (Sim, EntityId) {
        let base = sta_content::fake::db();
        let mut b = ContentDb::builder().profile(profile).features(base.features().clone());
        for w in base.weapons().cloned() {
            b = b.weapon(w);
        }
        for mut u in base.units().cloned() {
            if u.odf == "fx_frigate" {
                u.movement = class(80.0, 50.0);
            }
            b = b.unit(u);
        }
        let db = Arc::new(b.build().unwrap());
        let mut sim = approach_tests::sim_with(Arc::clone(&db));
        let s = sim.spawn_def(db.key("fx_frigate").unwrap(), PlayerId(1), pt(0.0, 0.0), (Fixed::ZERO, Fixed::ONE)).unwrap();
        (sim, s)
    }

    fn run(sim: &mut Sim, s: EntityId, to: Vec3, secs: u32) -> Vec<(Vec3, Fixed, Fixed)> {
        let mut out = Vec::new();
        for i in 0..secs * TICK_HZ {
            let cmds = if i == 0 {
                vec![Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Move { units: vec![s], to, queue: false }) }]
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
            assert_eq!(run(&mut a, sa, to, 40).iter().map(|r| (r.0, r.1, r.2)).collect::<Vec<_>>(), run(&mut b, sb, to, 40));
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

    #[test]
    fn the_scripted_warp_speed_of_the_sim_is_the_cruise_multiplier() {
        use crate::command::Edit;
        let _g = seam::set(params());
        let (mut sim, s) = game(Profile::Sta1Classic);
        let set = Command { player: PlayerId::SCRIPT, seq: 0, kind: CommandKind::Edit(Edit::WarpSpeed(s, Some(Fixed::from_int(25)))) };
        WorldSource::step(&mut sim, &[set]);
        let tr = run(&mut sim, s, pt(0.0, 3000.0), 20);
        let mid = num(tr[(12 * TICK_HZ) as usize].1);
        assert!((mid - 25.0).abs() < 0.5, "{mid}");
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
