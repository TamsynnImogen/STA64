//! The movement models on their own (SIM-15). The craft are invented: round
//! numbers of a heavy craft that cannot turn at rest and a light one that
//! can, chosen to exercise the same code paths. They are no unit's or
//! game's data.

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

/// Invented heavy craft: no turn acceleration at rest (it pulls away before it
/// turns), a high speed acceleration and a short turn-in distance.
fn heavy_craft() -> MoveStats {
    MoveStats {
        combat_speed: f(50.0),
        turn_alpha_at_rest: Fixed::ZERO,
        too_close_to_turn: f(60.0),
        path_lead: f(200.0),
        turn_control_angle: f(35.0),
        turn_control_squared: false,
        forward_control_distance: f(120.0),
        ..MoveStats::from_f32(90.0, 2.0, 0.9, 1.0, 0.0)
    }
}

/// Invented light craft: turns at once at rest, accelerates slowly, long
/// path lead.
fn light_craft() -> MoveStats {
    MoveStats {
        combat_speed: f(100.0),
        turn_alpha_at_rest: Fixed::ONE,
        too_close_to_turn: f(90.0),
        path_lead: f(350.0),
        turn_control_angle: f(25.0),
        turn_control_squared: false,
        forward_control_distance: f(80.0),
        ..MoveStats::from_f32(100.0, 0.25, 0.15, 0.25, 1.0)
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
        step(Model::Smooth, u, Leg { queued: &queue, a1: None });
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

/// Acceleration is `1.3 combatSpeed forwardAccel`: the heavy craft 130 u/s^2
/// (cruise 90 in about 0.7 s), the light one 32.5 u/s^2 (cruise 100 in about
/// 3.1 s). The tick counts follow from those numbers at 30 ticks a second.
#[test]
fn acceleration_scales_with_combat_speed() {
    for (stats, accel, cruise) in [(heavy_craft(), 130.0, 90.0), (light_craft(), 32.5, 100.0)] {
        let ticks = (cruise / accel * 30.0_f64).ceil() as i32;
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

/// `turnOmegaFractionAtRest = 0`: the heavy craft cannot turn at rest, so it
/// pulls away first; the light one turns at once.
#[test]
fn turning_at_rest_follows_the_fractions() {
    let mut a = craft(heavy_craft(), NORTH);
    a.target = Some(v(2000, 0, 0));
    step(Model::Smooth, &mut a, Leg::default());
    assert_eq!(a.heading, NORTH);
    assert!(a.speed > Fixed::ZERO);

    let mut b = craft(light_craft(), NORTH);
    b.target = Some(v(2000, 0, 0));
    step(Model::Smooth, &mut b, Leg::default());
    assert_ne!(b.heading, NORTH);
}

/// A craft that cannot turn at rest (both fractions 0) and stops with turn left over
/// settles instead of spinning for ever (capture of a capital ship that kept turning on the
/// spot with no target; the spin-down rate is interim).
#[test]
fn leftover_turn_settles_for_a_craft_that_cannot_turn_at_rest() {
    let mut u = craft(heavy_craft(), NORTH);
    u.speed = f(40.0);
    u.omega = f(-5.0);
    for _ in 0..2000 {
        step(Model::Smooth, &mut u, Leg::default());
    }
    assert_eq!(u.speed, Fixed::ZERO);
    assert_eq!(u.omega, Fixed::ZERO, "still spinning");
    // Same with a target close enough to count as arrived.
    let mut w = craft(heavy_craft(), NORTH);
    w.omega = f(0.8);
    w.target = Some(v(1, 0, 1));
    for _ in 0..2000 {
        step(Model::Smooth, &mut w, Leg::default());
    }
    assert_eq!(w.omega, Fixed::ZERO, "still spinning with a target at hand");
}

/// Invented stats for the #1039 tests: `combatSpeed` 30, so a full-speed turn limit of
/// 0.5 * 30 * 0.08 = 1.2 rad/s and a turn acceleration of 0.9 of it per second (1.08 rad/s^2),
/// speed acceleration 1.3 * 30 * 2 = 78 u/s^2.
fn invented(turn_at_rest: f64, turn_alpha_at_rest: f64) -> MoveStats {
    MoveStats {
        combat_speed: f(30.0),
        turn_at_rest: f(turn_at_rest),
        turn_alpha_at_rest: f(turn_alpha_at_rest),
        ..MoveStats::from_f32(60.0, 2.0, 0.08, 0.9, 0.0)
    }
}

const ACCEL: f64 = 78.0;
const SPIN_ALPHA: f64 = 1.08;

/// Ticks to bring `speed` and then a turn rate of at most `omega` to zero: the time to rest,
/// then the turn's braking at the full-speed turn acceleration (the interim spin-down), one
/// tick of slack each.
fn bound(speed: f64, omega: f64) -> usize {
    ((speed / ACCEL * 30.0).ceil() + 1.0 + (omega / SPIN_ALPHA * 30.0).ceil() + 1.0) as usize
}

/// Runs `tick` for `bound` ticks, then checks the turn rate is exactly zero, and stays zero.
fn settles_within(mut u: Unit, bound: usize, what: &str, mut tick: impl FnMut(&mut Unit)) {
    for _ in 0..bound {
        tick(&mut u);
    }
    assert_eq!(u.omega, Fixed::ZERO, "{what}: still turning at {} rad/s after {bound} ticks", u.omega.to_f32());
    for _ in 0..30 {
        tick(&mut u);
        assert_eq!(u.omega, Fixed::ZERO, "{what}: the turn started again");
    }
}

/// #1039, the facing step: a craft in range of its attack target brakes while turning to face
/// it (`Plan::face`). A craft with no turn acceleration at rest (`turnAlphaFractionAtRest` 0)
/// keeps a turn left over when it stops, for ever, whether it can turn at rest or not. The
/// spin-down rate is interim (STA64's own; the original's is not measured).
#[test]
fn leftover_turn_settles_while_facing_an_attack_target() {
    for side in [-1.0, 1.0] {
        let plan = Plan { face: Some(v((2000.0 * side) as i32, 0, 0)), ..Plan::default() };
        for omega in [-1.0, 1.0] {
            // Cannot turn or accelerate its turn at rest, under way at half cruise: braked to
            // rest first (omega up to the full-speed limit 1.2 while moving).
            let mut u = craft(invented(0.0, 0.0), NORTH);
            u.speed = f(30.0);
            u.omega = f(omega);
            settles_within(u, bound(30.0, 1.2), "car-like, moving", |u| step_planned(Model::Smooth, u, Leg::default(), plan));
        }
    }
}

/// The same for a craft that can turn at rest (limit 0.6 rad/s there) but has no turn
/// acceleration at rest: a turn within the limit kept its rate for ever while "facing".
#[test]
fn leftover_turn_settles_while_facing_with_no_turn_acceleration_at_rest() {
    for side in [-1.0, 1.0] {
        let plan = Plan { face: Some(v((2000.0 * side) as i32, 0, 0)), ..Plan::default() };
        for omega in [-0.4, 0.4] {
            let mut u = craft(invented(0.5, 0.0), NORTH);
            u.omega = f(omega);
            settles_within(u, bound(0.0, 0.4), "turns at rest, no acceleration, at rest", |u| step_planned(Model::Smooth, u, Leg::default(), plan));
        }
    }
}

/// #1039, the steering step of a held craft (a script's speed 0, or a nebula's floor: its top
/// speed is the held floor, so the turn limit sees speed 0): with a far target and no turn
/// acceleration at that speed it kept its turn rate for ever.
#[test]
fn a_held_craft_does_not_keep_a_turn_it_cannot_steer() {
    for side in [-1.0, 1.0] {
        for omega in [-0.5, 0.5] {
            let mut stats = invented(0.0, 0.0);
            stats.max_speed = crate::speed_override::HELD_SPEED;
            let mut u = craft(stats, NORTH);
            u.omega = f(omega);
            u.target = Some(v((5000.0 * side) as i32, 0, 0));
            settles_within(u, bound(0.0, 0.5), "held craft with a far target", |u| step(Model::Smooth, u, Leg::default()));
        }
    }
}

/// No slowing to turn: facing away, a unit asks for full speed.
#[test]
fn no_slowing_to_turn() {
    let mut u = craft(light_craft(), NORTH);
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
    let mut u = craft(light_craft(), NORTH);
    let (t, _) = fly(&mut u, &[v(0, 0, -60)], 600);
    assert!(t.is_some());
    assert_eq!(u.heading, NORTH);
    assert!(u.pos.z.abs() < f(1.0), "{}", u.pos.z.to_f32());
}

/// Arrival brakes to rest near the point, without snapping onto it.
#[test]
fn arrives_short_of_the_point_at_rest() {
    for stats in [heavy_craft(), light_craft()] {
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
    for stats in [heavy_craft(), light_craft()] {
        let mut u = craft(stats, NORTH);
        let legs =[v(0, 0, 2000), v(2000, 0, 2000), v(2000, 0, 4000)];
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
    for stats in [heavy_craft(), light_craft()] {
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
    for stats in [heavy_craft(), light_craft()] {
        for legs in [
            &[v(0, 0, 2000), v(2000, 0, 2000), v(2000, 0, 4000)][..],
            &[v(0, 0, -3000)][..],
        ] {
            let mut u = craft(stats, NORTH);
            let mut queue: Vec<Vec3> = legs[1..].to_vec();
            u.target = Some(legs[0]);
            for _ in 0..1500 {
                step(Model::Smooth, &mut u, Leg { queued: &queue, a1: None });
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
const SMOOTH_GOLDEN: u64 = 0x315e_7a06_598b_e63a;

/// The native model is the one profiles without a measured model fly.
#[test]
fn models_by_profile() {
    assert_eq!(Model::of(Profile::Native), Model::Native);
    assert_eq!(Model::of(Profile::Sta1Classic), Model::Native);
    assert_eq!(Model::of(Profile::Sta2Classic), Model::Smooth);
    assert_eq!(Model::of(Profile::Fo4), Model::Smooth);
}
