//! SIM-67: craft separation under the Armada II family model (3D, shared by
//! speed) and the Native model it leaves alone. Authored numbers.

use std::sync::Arc;

use sta_content::{ContentDb, MoveStats, Profile, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use crate::command::{Command, CommandKind, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}

fn p3(x: i32, y: i32, z: i32) -> Vec3 {
    Vec3 { x: n(x), y: n(y), z: n(z) }
}

/// Body radius of the test craft; two of them make a radius sum of 40.
pub(crate) const RADIUS: i32 = 20;
const SUM: f64 = 40.0;

fn db(profile: Profile, radius: i32) -> Arc<ContentDb> {
    let mut ship = UnitDef::new("st_ship", "craft");
    ship.race = "st".into();
    ship.movement = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 1.0);
    ship.movement.combat_speed = n(70);
    ship.radius = n(radius);
    ship.hull = n(100_000);
    Arc::new(
        ContentDb::builder()
            .profile(profile)
            .features(profile.features())
            .unit(ship)
            .build()
            .expect("consistent"),
    )
}

pub(crate) fn sim(profile: Profile) -> Sim {
    sim_r(profile, RADIUS)
}

fn sim_r(profile: Profile, radius: i32) -> Sim {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(1), race: "st".into(), control: Control::Idle };
    Sim::from_setup(&GameSetup {
        seed: 9,
        map: Arc::new(sta_content::MapDef::empty()),
        content: db(profile, radius),
        players: vec![seat(1), seat(2)],
        options: GameOptions::default(),
    })
}

pub(crate) fn put(sim: &mut Sim, at: Vec3, heading: (i32, i32)) -> EntityId {
    let key = sim.content().unwrap().key("st_ship").unwrap();
    sim.spawn_def(key, PlayerId(1), at, (n(heading.0), n(heading.1))).unwrap()
}

pub(crate) fn go(sim: &mut Sim, ids: &[EntityId], to: Vec3) {
    let kind = CommandKind::Order(Order::Move { units: ids.to_vec(), to, queue: false, mode: None });
    sim.step(&[Command { player: PlayerId(1), seq: 0, kind }]);
}

fn dist3(a: Vec3, b: Vec3) -> f64 {
    let (x, y, z) = (a.x - b.x, a.y - b.y, a.z - b.z);
    fl((x * x + y * y + z * z).sqrt())
}

fn fl(x: Fixed) -> f64 {
    x.to_f32() as f64
}

/// What one craft did over a run: its greatest height offset (signed), its
/// greatest sideways offset from its straight route, and the closest 3D
/// approach to a given craft.
#[derive(Default, Debug)]
struct Peaks {
    y: f64,
    side: f64,
}

/// `route`: a point on the craft's straight route and its unit direction.
fn note(pk: &mut Peaks, at: Vec3, route: ((f64, f64), (f64, f64))) {
    let y = fl(at.y);
    if y.abs() > pk.y.abs() {
        pk.y = y;
    }
    let ((x0, z0), (dx, dz)) = route;
    let side = (fl(at.x) - x0) * -dz + (fl(at.z) - z0) * dx;
    if side.abs() > pk.side.abs() {
        pk.side = side;
    }
}

struct Run {
    a: Peaks,
    b: Peaks,
    closest: f64,
    parked_moved: f64,
    end_a: Vec3,
    end_b: Vec3,
}

/// Two craft flying their routes (the second may be absent) past a parked
/// craft, for `ticks` ticks.
fn fly(
    profile: Profile,
    a: (Vec3, Vec3, (i32, i32)),
    b: Option<(Vec3, Vec3, (i32, i32))>,
    parked: Option<Vec3>,
    ticks: u32,
) -> Run {
    let mut sim = sim(profile);
    let ida = put(&mut sim, a.0, a.2);
    let idb = b.map(|b| put(&mut sim, b.0, b.2));
    let idp = parked.map(|p| put(&mut sim, p, (1, 0)));
    go(&mut sim, &[ida], a.1);
    if let (Some(id), Some(b)) = (idb, b) {
        go(&mut sim, &[id], b.1);
    }
    let route = |s: Vec3, e: Vec3| {
        let (dx, dz) = (e.x - s.x, e.z - s.z);
        let l = fl((dx * dx + dz * dz).sqrt());
        ((fl(s.x), fl(s.z)), (fl(dx) / l, fl(dz) / l))
    };
    let (ra, rb) = (route(a.0, a.1), b.map_or(((0.0, 0.0), (1.0, 0.0)), |b| route(b.0, b.1)));
    let mut run = Run { a: Peaks::default(), b: Peaks::default(), closest: 1e9, parked_moved: 0.0, end_a: a.0, end_b: a.0 };
    for _ in 0..ticks {
        sim.step(&[]);
        let pa = sim.unit(ida).unwrap().pos;
        note(&mut run.a, pa, ra);
        if let Some(id) = idb {
            let pb = sim.unit(id).unwrap().pos;
            note(&mut run.b, pb, rb);
            run.closest = run.closest.min(dist3(pa, pb));
        }
        if let (Some(id), Some(p)) = (idp, parked) {
            let pp = sim.unit(id).unwrap().pos;
            run.parked_moved = run.parked_moved.max(dist3(pp, p));
            run.closest = run.closest.min(dist3(pa, pp));
        }
    }
    run.end_a = sim.unit(ida).unwrap().pos;
    if let Some(id) = idb {
        run.end_b = sim.unit(id).unwrap().pos;
    }
    run
}

fn head_on(profile: Profile) -> Run {
    // Two craft head on over a parked one midway; the second starts a little
    // further out, so the first meets the parked craft first.
    fly(
        profile,
        (p3(100, 0, 300), p3(1100, 0, 300), (1, 0)),
        Some((p3(1270, 0, 300), p3(100, 0, 300), (-1, 0))),
        Some(p3(640, 0, 300)),
        600,
    )
}

#[test]
fn head_on_over_a_parked_craft_moves_only_the_movers_and_only_vertically() {
    let r = head_on(Profile::Sta2Classic);
    // The parked craft is never moved (not even by a fraction).
    assert_eq!(r.parked_moved, 0.0);
    // On the degenerate line the push is vertical: no sideways detour, and one
    // craft goes over, the other under, by about the radius sum.
    assert!(r.a.side.abs() < 0.5 && r.b.side.abs() < 0.5, "{:?} {:?}", r.a, r.b);
    assert!((0.8 * SUM..=1.25 * SUM).contains(&r.a.y.abs()), "{:?}", r.a);
    assert!((0.8 * SUM..=1.25 * SUM).contains(&r.b.y.abs()), "{:?}", r.b);
    // Never much closer (3D) than the radius sum: the three bodies stack, so
    // the tolerance is wider than for a pair (15 % of the sum).
    assert!(r.closest >= 0.85 * SUM, "closest {}", r.closest);
    // Both made it across to their goals.
    assert!(gap_xz(r.end_a, p3(1100, 0, 300)) < 5.0, "{:?}", r.end_a);
    assert!(gap_xz(r.end_b, p3(100, 0, 300)) < 5.0, "{:?}", r.end_b);
}

fn gap_xz(a: Vec3, b: Vec3) -> f64 {
    let (x, z) = (a.x - b.x, a.z - b.z);
    fl((x * x + z * z).sqrt())
}

#[test]
fn a_glancing_pass_leaves_the_parked_craft_alone() {
    // Lateral offset 12, well inside the radius sum.
    let r = fly(
        Profile::Sta2Classic,
        (p3(0, 0, 500), p3(1000, 0, 500), (1, 0)),
        None,
        Some(p3(500, 0, 512)),
        400,
    );
    assert_eq!(r.parked_moved, 0.0);
    assert!(r.closest >= SUM - 2.0, "closest {}", r.closest);
    // A mix of sideways (away from the parked craft) and vertical.
    assert!(r.a.side.abs() > 0.1 * SUM && r.a.y.abs() > 0.05 * SUM, "{:?}", r.a);
    assert!(r.a.side < 0.0, "pushed away from the parked craft: {:?}", r.a);
}

#[test]
fn a_crossing_at_thirty_degrees_splits_the_push_between_both_craft() {
    // Both reach the crossing point (500, 300) together.
    let r = fly(
        Profile::Sta2Classic,
        (p3(0, 0, 300), p3(1000, 0, 300), (1, 0)),
        Some((p3(67, 0, 50), p3(933, 0, 550), (1, 0))),
        None,
        600,
    );
    assert!(r.closest >= SUM - 2.0, "closest {}", r.closest);
    // Fitted band (bench marks): vertical 0.15 to 0.6 of the radius sum for
    // each craft, opposite signs, sideways at most 0.6 of it.
    for p in [&r.a, &r.b] {
        assert!((0.15 * SUM..=0.6 * SUM).contains(&p.y.abs()), "{p:?}");
        assert!(p.side.abs() <= 0.6 * SUM, "{p:?}");
    }
    assert!(r.a.y * r.b.y < 0.0, "{:?} {:?}", r.a, r.b);
}

#[test]
fn the_native_model_keeps_the_xz_half_and_half_rule() {
    let r = fly(
        Profile::Native,
        (p3(0, 0, 500), p3(1000, 0, 500), (1, 0)),
        None,
        Some(p3(500, 0, 512)),
        400,
    );
    // Height is ignored and a parked craft is pushed.
    assert_eq!(r.a.y, 0.0);
    assert!(r.parked_moved > 1.0, "{}", r.parked_moved);
}

fn digest_of(profile: Profile) -> u64 {
    let mut sim = sim(profile);
    let a = put(&mut sim, p3(0, 0, 300), (1, 0));
    let b = put(&mut sim, p3(67, 0, 50), (1, 0));
    go(&mut sim, &[a], p3(1000, 0, 300));
    go(&mut sim, &[b], p3(933, 0, 550));
    for _ in 0..360 {
        sim.step(&[]);
    }
    let digest = sim.digest();
    let mut loaded = Sim::from_snapshot(&sim.snapshot(), sim.content().cloned()).unwrap();
    for _ in 0..60 {
        sim.step(&[]);
        loaded.step(&[]);
    }
    assert_eq!(loaded.digest(), sim.digest());
    digest
}

/// Pinned whole-state digests of two bodied craft crossing; they move only
/// when the separation rule or the movement under it changes.
#[test]
fn pinned_digest_of_two_craft_crossing() {
    let (a2, native) = (digest_of(Profile::Sta2Classic), digest_of(Profile::Native));
    assert_eq!(a2, PINNED_A2, "{a2:#x}");
    assert_eq!(native, PINNED_NATIVE, "{native:#x}");
}

const PINNED_A2: u64 = 0x908f_3722_88fe_cf78;
const PINNED_NATIVE: u64 = 0x7bf3_4c37_09cc_fc90;

#[test]
fn a_craft_with_a_goal_descends_on_the_straight_line_and_arrives_with_the_horizontal_arrival() {
    let mut sim = sim(Profile::Sta2Classic);
    let a = put(&mut sim, p3(0, 20, 0), (1, 0));
    go(&mut sim, &[a], p3(1000, 0, 0));
    let (mut checked, mut halfway) = (0, false);
    for _ in 0..900 {
        sim.step(&[]);
        let at = sim.unit(a).unwrap().pos;
        let left = 1000.0 - fl(at.x);
        // Height left over distance left stays the original slope, 20 / 1000.
        if left > 150.0 && fl(at.x) > 20.0 {
            assert!((fl(at.y) / left - 0.02).abs() < 0.002, "x {} y {}", fl(at.x), fl(at.y));
            checked += 1;
        }
        // Half way along it is still about half the height (the old climb had
        // spent all of it within a fraction of a second).
        if (fl(at.x) - 500.0).abs() < 3.0 {
            assert!((8.0..=12.0).contains(&fl(at.y)), "y {} half way", fl(at.y));
            halfway = true;
        }
    }
    assert!(checked > 100);
    assert!(halfway, "never passed the half way mark");
    let end = sim.unit(a).unwrap();
    assert!(gap_xz(end.pos, p3(1000, 0, 0)) < 5.0);
    assert!(end.pos.y >= Fixed::ZERO && fl(end.pos.y) < 2.0, "ended at height {}", fl(end.pos.y));
}

/// A pin of rule that did not change (the old climb held it too).
#[test]
fn a_craft_with_no_goal_holds_its_height() {
    let mut sim = sim(Profile::Sta2Classic);
    let a = put(&mut sim, p3(0, 20, 0), (1, 0));
    for _ in 0..600 {
        sim.step(&[]);
    }
    assert_eq!(sim.unit(a).unwrap().pos, p3(0, 20, 0));
}

/// Fake content with bodies, under the Armada II profile: cruiser circle
/// 100 (body 75), station 200 (body 150).
fn station_content() -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut b = ContentDb::builder().profile(Profile::Sta2Classic).features(fake.features().clone());
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    for u in fake.units() {
        let mut u = u.clone();
        u.radius = n(match u.odf.as_str() {
            "fx_cruiser" => 100,
            "fx_station" => 200,
            _ => 0,
        });
        b = b.unit(u);
    }
    Arc::new(b.build().unwrap())
}

#[test]
fn an_enemy_station_stays_a_solid_craft_are_pushed_out_of_and_it_never_moves() {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(sta_content::MapDef::empty()),
        content: station_content(),
        players: vec![seat(1), seat(2)],
        options: GameOptions::default(),
    });
    let k = |s: &str, sim: &Sim| sim.content().unwrap().key(s).unwrap();
    let (ks, kc) = (k("fx_station", &sim), k("fx_cruiser", &sim));
    let station = sim.spawn_def(ks, PlayerId(2), p3(0, 0, 0), (Fixed::ZERO, Fixed::ONE)).unwrap();
    let ship = sim.spawn_def(kc, PlayerId(1), p3(0, 0, -1000), (Fixed::ZERO, Fixed::ONE)).unwrap();
    go(&mut sim, &[ship], p3(0, 0, 1000));
    let mut closest = f64::MAX;
    for _ in 0..1200 {
        sim.step(&[]);
        closest = closest.min(gap_xz(sim.unit(ship).unwrap().pos, p3(0, 0, 0)));
    }
    // Solid rule unchanged: three quarters of each radius, craft pushed fully out.
    assert!(closest >= 150.0 + 75.0 - 15.0, "came within {closest}");
    assert_eq!(sim.unit(station).unwrap().pos, p3(0, 0, 0));
}

fn body(id: u32, at: Vec3, movable: bool) -> crate::spatial::Body {
    crate::spatial::Body { id: EntityId(id), pos: at, radius: n(20), movable }
}

fn shifts(bodies: &[crate::spatial::Body], speeds: &[(u32, i32)]) -> Vec<(EntityId, crate::spatial::Shift3)> {
    let grid = crate::spatial::Grid::build(bodies, 7);
    let speed = |id: EntityId| n(speeds.iter().find(|s| s.0 == id.0).map_or(0, |s| s.1));
    crate::spatial::separate_3d(&grid, &speed, Fixed::from_f64(0.42))
}

#[test]
fn an_immovable_body_never_moves_and_the_other_takes_the_whole_push() {
    // A rig (immovable) and a craft overlapping by 10 along X, both at rest.
    let out = shifts(&[body(1, p3(0, 0, 0), false), body(2, p3(30, 0, 0), true)], &[]);
    assert_eq!(out.len(), 1);
    let (id, (x, _, _)) = out[0];
    assert_eq!(id, EntityId(2));
    assert!((fl(x) - 10.0).abs() < 0.01, "{}", fl(x));
}

#[test]
fn at_equal_heights_the_lower_id_goes_up_and_the_higher_id_down() {
    // Two movers at the same height, same speed, overlapping along X.
    let out = shifts(&[body(1, p3(0, 0, 0), true), body(2, p3(30, 0, 0), true)], &[(1, 50), (2, 50)]);
    let (a, b) = (out[0].1, out[1].1);
    assert!(a.1 > Fixed::ZERO && b.1 < Fixed::ZERO, "{a:?} {b:?}");
    assert!((fl(a.1 + b.1)).abs() < 0.01, "no net lift: {a:?} {b:?}");
    // Pushed apart along the line, half each.
    assert!(a.0 < Fixed::ZERO && b.0 > Fixed::ZERO);
}

#[test]
fn only_a_craft_at_speed_zero_is_never_moved() {
    // The craft at rest is not moved: the other one takes the whole push.
    let out = shifts(&[body(1, p3(0, 0, 0), true), body(2, p3(30, 0, 0), true)], &[(1, 0), (2, 5)]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].0, EntityId(2));
}

#[test]
fn the_armada_one_classic_profile_keeps_the_xz_rule() {
    let r = fly(
        Profile::Sta1Classic,
        (p3(0, 0, 500), p3(1000, 0, 500), (1, 0)),
        None,
        Some(p3(500, 0, 512)),
        400,
    );
    assert_eq!(r.a.y, 0.0);
    assert!(r.parked_moved > 1.0, "{}", r.parked_moved);
}

/// Known difference: formation slots are 60 apart (movement.rs), so craft
/// whose radii sum to more than that overlap their own slots and are pushed
/// apart; they end a few units off their slots and a few units above and below
/// the plane (the residual of the height rule, see `climb_line`). The group
/// still arrives.
#[test]
fn a_group_move_of_large_craft_ends_close_to_its_slots_and_the_plane() {
    let mut sim = sim_r(Profile::Sta2Classic, 35);
    let ids: Vec<_> = (0..3).map(|i| put(&mut sim, p3(0, 0, i * 200), (1, 0))).collect();
    let kind = CommandKind::Order(Order::Formation { units: ids.clone(), kind: 1 });
    sim.step(&[Command { player: PlayerId(1), seq: 0, kind }]);
    let kind = CommandKind::Order(Order::Move { units: ids.clone(), to: p3(1000, 0, 200), queue: false, mode: None });
    sim.step(&[Command { player: PlayerId(1), seq: 1, kind }]);
    for _ in 0..900 {
        sim.step(&[]);
    }
    for (i, id) in ids.iter().enumerate() {
        let at = sim.unit(*id).unwrap().pos;
        let slot = p3(1000, 0, 200 - (i as i32 - 1) * 60);
        assert!(gap_xz(at, slot) < 15.0, "craft {i} ended {} from its slot", gap_xz(at, slot));
        assert!(fl(at.y).abs() < 10.0, "craft {i} ended at height {}", fl(at.y));
    }
}
