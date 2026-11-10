//! The movement models on their own (SIM-15). Physics values are the stock
//! and Fleet Operations 4.0 ODFs' own; the bench `move-*` scenarios run the
//! same craft through the whole sim.

use super::*;
use crate::world::{Entity, EntityKind};
use sta_content::MoveStats;
use sta_core::{ContentKey, PlayerId};

fn v(x: i32, y: i32, z: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::from_int(y),
        z: Fixed::from_int(z),
    }
}

fn f(x: f64) -> Fixed {
    Fixed::from_f64(x)
}

/// Stock `sbattphys.odf` (the battleship).
fn stock_battleship() -> MoveStats {
    MoveStats {
        combat_speed: f(40.0),
        turn_alpha_at_rest: Fixed::ZERO,
        too_close_to_turn: f(50.0),
        path_lead: f(150.0),
        turn_control_angle: f(30.0),
        turn_control_squared: false,
        forward_control_distance: f(100.0),
        ..MoveStats::from_f32(80.0, 3.0, 1.0, 1.0, 0.0)
    }
}

/// Fleet Operations 4.0 `smooth_physics_100.odf` (Galaxy class).
fn fo_100() -> MoveStats {
    MoveStats {
        combat_speed: f(120.0),
        turn_alpha_at_rest: Fixed::ONE,
        too_close_to_turn: f(100.0),
        path_lead: f(400.0),
        turn_control_angle: f(20.0),
        turn_control_squared: false,
        forward_control_distance: f(70.0),
        ..MoveStats::from_f32(120.0, 0.2, 0.145, 0.24, 1.0)
    }
}

fn craft(stats: MoveStats, heading: (Fixed, Fixed)) -> Unit {
    Entity::new(
        EntityKind::Craft,
        ContentKey::NONE,
        PlayerId(1),
        v(0, 0, 0),
        heading,
        stats,
    )
}

const NORTH: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);

/// Fly `u` along `legs` (the first is its target), handing over the next
/// leg as the course does. Returns the tick it stopped with nothing left,
/// and the lowest speed seen on the way once it first reached 90% of
/// cruise, until the last leg.
fn fly(u: &mut Unit, legs: &[Vec3], max: u32) -> (Option<u32>, Fixed) {
    let mut queue: Vec<Vec3> = legs[1..].to_vec();
    u.target = Some(legs[0]);
    let mut slowest = u.stats.max_speed;
    let mut moving = false;
    for t in 0..max {
        step(Model::Smooth, u, Leg { queued: &queue });
        if u.target.is_none() && !queue.is_empty() {
            u.target = Some(queue.remove(0));
        }
        moving |= u.speed >= u.stats.max_speed * f(0.9);
        if moving && !queue.is_empty() {
            slowest = slowest.min(u.speed);
        }
        if u.target.is_none() && u.speed == Fixed::ZERO && queue.is_empty() {
            return (Some(t), slowest);
        }
    }
    (None, slowest)
}

#[test]
#[allow(clippy::disallowed_methods)] // platform trig as the reference only
fn atan2_matches_trig() {
    for i in -180..=180 {
        let a = (i as f64).to_radians();
        let got = atan2(f(a.sin() * 3.0), f(a.cos() * 3.0)).to_f32() as f64;
        let want = (a.sin()).atan2(a.cos());
        let d = (got - want).abs();
        let diff = d.min((d - std::f64::consts::TAU).abs());
        assert!(diff < 2e-5, "{i} deg: {got} vs {want}");
    }
}

#[test]
fn rotate_by_large_angles_stays_unit_and_right() {
    let h = rotate_by(NORTH, pi());
    assert!(
        (h.1 + Fixed::ONE).abs() < f(1e-6),
        "{:?}",
        (h.0.to_f32(), h.1.to_f32())
    );
    let l = h.0 * h.0 + h.1 * h.1;
    assert!((l - Fixed::ONE).abs() < f(1e-6));
}

/// The position controller lands on its goal at rest, without overshoot.
#[test]
fn position_controller_settles() {
    let (mut d, mut w) = (f(1.0), Fixed::ZERO);
    let mut worst = Fixed::ZERO;
    for _ in 0..300 {
        let (s, nw) = smooth::pc(d, w, f(2.088), f(8.7), dt());
        d = d - s;
        w = nw;
        worst = worst.min(d);
    }
    assert!(d.abs() < f(0.002), "{}", d.to_f32());
    assert!(w.abs() < f(0.01), "{}", w.to_f32());
    assert!(worst > f(-0.002), "overshoot {}", worst.to_f32());
}

/// Acceleration is `1.3 combatSpeed forwardAccel`: stock battleship
/// 156 u/s^2 (cruise in about half a second), FO 31.2 u/s^2 (about 3.85 s).
#[test]
fn acceleration_scales_with_combat_speed() {
    for (stats, ticks) in [(stock_battleship(), 16), (fo_100(), 116)] {
        let mut u = craft(stats, NORTH);
        u.target = Some(v(0, 0, 5000));
        let mut t: i32 = 0;
        while u.speed < stats.max_speed - f(0.01) {
            step(Model::Smooth, &mut u, Leg::default());
            t += 1;
            assert!(t < 1000);
        }
        assert!(
            (t - ticks).abs() <= 1,
            "{t} ticks to cruise, want about {ticks}"
        );
    }
}

/// `turnOmegaFractionAtRest = 0`: a stock battleship cannot turn at rest,
/// so it pulls away first; FO turns at once.
#[test]
fn turning_at_rest_follows_the_fractions() {
    let mut a = craft(stock_battleship(), NORTH);
    a.target = Some(v(2000, 0, 0));
    step(Model::Smooth, &mut a, Leg::default());
    assert_eq!(a.heading, NORTH);
    assert!(a.speed > Fixed::ZERO);

    let mut b = craft(fo_100(), NORTH);
    b.target = Some(v(2000, 0, 0));
    step(Model::Smooth, &mut b, Leg::default());
    assert_ne!(b.heading, NORTH);
}

/// No slowing to turn: facing away, a unit asks for full speed.
#[test]
fn no_slowing_to_turn() {
    let mut u = craft(fo_100(), NORTH);
    u.target = Some(v(0, 0, -3000));
    for _ in 0..90 {
        step(Model::Smooth, &mut u, Leg::default());
    }
    assert!(u.speed > f(80.0), "{}", u.speed.to_f32());
}

/// A point behind and inside `tooCloseToTurn`: no circling round, the unit
/// stays where it is and counts as arrived.
#[test]
fn too_close_to_turn_stays() {
    let mut u = craft(fo_100(), NORTH);
    let (t, _) = fly(&mut u, &[v(0, 0, -60)], 600);
    assert!(t.is_some());
    assert_eq!(u.heading, NORTH);
    assert!(u.pos.z.abs() < f(1.0), "{}", u.pos.z.to_f32());
}

/// Arrival brakes to rest near the point, without snapping onto it.
#[test]
fn arrives_short_of_the_point_at_rest() {
    for stats in [stock_battleship(), fo_100()] {
        let mut u = craft(stats, NORTH);
        let (t, _) = fly(&mut u, &[v(0, 0, 1500)], 3000);
        assert!(t.is_some(), "did not stop");
        let gap = (Fixed::from_int(1500) - u.pos.z).abs();
        assert!(
            gap < stats.too_close_to_turn,
            "stopped {} from the point",
            gap.to_f32()
        );
        assert!(u.pos.x.abs() < f(0.01));
    }
}

/// Waypoints with legs behind them are flown through, not stopped at.
#[test]
fn waypoint_chain_flows_through() {
    for stats in [stock_battleship(), fo_100()] {
        let mut u = craft(stats, NORTH);
        let legs = [v(0, 0, 2000), v(2000, 0, 2000), v(2000, 0, 4000)];
        let (t, slowest) = fly(&mut u, &legs, 6000);
        assert!(t.is_some(), "did not finish");
        assert!(
            slowest > stats.max_speed / Fixed::from_int(2),
            "slowed to {}",
            slowest.to_f32()
        );
        let (dx, dz) = (u.pos.x - legs[2].x, u.pos.z - legs[2].z);
        assert!((dx * dx + dz * dz).sqrt() < stats.too_close_to_turn);
    }
}

/// A 180-degree turn settles on the new course without weaving across it.
#[test]
fn reversal_settles_without_weaving() {
    for stats in [stock_battleship(), fo_100()] {
        let mut u = craft(stats, NORTH);
        u.target = Some(v(0, 0, -6000));
        let mut crossings = 0;
        let mut last: Option<bool> = None;
        for _ in 0..30 * 40 {
            step(Model::Smooth, &mut u, Leg::default());
            let Some(t) = u.target else { break };
            let (dx, dz) = (t.x - u.pos.x, t.z - u.pos.z);
            let d = (dx * dx + dz * dz).sqrt();
            let side = (u.heading.0 * dz - u.heading.1 * dx) / d;
            let ahead = u.heading.0 * dx + u.heading.1 * dz > Fixed::ZERO;
            if !ahead || side.abs() < f(1e-3) {
                continue;
            }
            let s = side > Fixed::ZERO;
            if last.is_some_and(|l| l != s) {
                crossings += 1;
            }
            last = Some(s);
        }
        assert!(crossings <= 2, "weaved {crossings} times");
    }
}

/// Golden: every tick of a waypoint chain and a reversal under the smooth
/// model, hashed, so any change to its arithmetic shows on every platform
/// CI runs (the bench `move-*` scenarios pin the same in readable numbers).
#[test]
fn smooth_trajectory_golden() {
    let mut h = crate::world::Fnv::new();
    for stats in [stock_battleship(), fo_100()] {
        for legs in [
            &[v(0, 0, 2000), v(2000, 0, 2000), v(2000, 0, 4000)][..],
            &[v(0, 0, -3000)][..],
        ] {
            let mut u = craft(stats, NORTH);
            let mut queue: Vec<Vec3> = legs[1..].to_vec();
            u.target = Some(legs[0]);
            for _ in 0..1500 {
                step(Model::Smooth, &mut u, Leg { queued: &queue });
                if u.target.is_none() && !queue.is_empty() {
                    u.target = Some(queue.remove(0));
                }
                for f in [
                    u.pos.x,
                    u.pos.y,
                    u.pos.z,
                    u.heading.0,
                    u.heading.1,
                    u.speed,
                    u.omega,
                ] {
                    h.fixed(f);
                }
            }
        }
    }
    assert_eq!(h.0, SMOOTH_GOLDEN, "{:#x}", h.0);
}
const SMOOTH_GOLDEN: u64 = 0x6ef9_9e4d_0759_6b0f;

/// The native model is the one profiles without a measured model fly.
#[test]
fn models_by_profile() {
    assert_eq!(Model::of(Profile::Native), Model::Native);
    assert_eq!(Model::of(Profile::Sta1Classic), Model::Native);
    assert_eq!(Model::of(Profile::Sta2Classic), Model::Smooth);
    assert_eq!(Model::of(Profile::Fo4), Model::Smooth);
}
