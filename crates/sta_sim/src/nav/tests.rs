use std::sync::Arc;

use sta_content::{
    ContentDb, MapDef, PathingRules, Region, RegionKind, RegionShape, Rules, UnitDef,
};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use crate::command::{Command, CommandKind, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::ZERO,
        z: Fixed::from_int(z),
    }
}

/// A `nebula_obj` definition from ODF text, as the content loader makes one.
fn nebula(odf: &str, text: &str) -> UnitDef {
    let text = format!("classLabel = \"nebula_obj\"\nunitName = \"{odf}\"\n{text}");
    let stems = [odf.to_owned()];
    let (db, _) = sta_content::odf::load_with(
        Default::default(),
        &Default::default(),
        &stems,
        &mut |_, version| (version == 0).then(|| text.clone()),
    )
    .unwrap();
    let u = db
        .units()
        .next()
        .expect("a nebula_obj with unitName is a unit def")
        .clone();
    assert!(u.nebula_pathing.is_some());
    u
}

/// Fake content with bodies (cruiser 75, moon 300, as SIM-21's tests) and
/// nebulae: `fx_mutara` (`pathing_cost = 400`), `fx_tachyon` (100, open
/// space), `fx_generated` (10000 but `addToPathPlanner = 0`).
fn content(pathing: Option<PathingRules>) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut b = ContentDb::builder()
        .profile(fake.profile())
        .features(fake.features().clone());
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    for u in fake.units() {
        let mut u = u.clone();
        u.radius = Fixed::from_int(match u.odf.as_str() {
            "fx_cruiser" => 100,
            "fx_moon" => 400,
            _ => 0,
        });
        b = b.unit(u);
    }
    b = b
        .unit(nebula(
            "fx_mutara",
            "effectRadius = 400\npathing_cost = 400\n",
        ))
        .unit(nebula(
            "fx_tachyon",
            "effectRadius = 400\npathing_cost = 100\n",
        ))
        .unit(nebula(
            "fx_generated",
            "effectRadius = 400\npathing_cost = 10000\naddToPathPlanner = 0\n",
        ));
    let db = b.build().unwrap();
    Arc::new(match pathing {
        Some(p) => db.with_rules(Rules::default().with_pathing(p)),
        None => db,
    })
}

fn sim_with(regions: Vec<Region>, db: Arc<ContentDb>) -> Sim {
    let mut map = MapDef::empty();
    map.regions = regions;
    let seat = |id| PlayerSetup {
        id: PlayerId(id),
        team: TeamId(id),
        race: "fx".into(),
        control: Control::Idle,
    };
    Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(map),
        content: db,
        players: vec![seat(1), seat(2)],
        options: GameOptions::default(),
    })
}

fn sim(regions: Vec<Region>) -> Sim {
    sim_with(regions, content(None))
}

fn put(sim: &mut Sim, odf: &str, at: Vec3) -> EntityId {
    let key = sim.content().unwrap().key(odf).unwrap();
    sim.spawn_def(key, PlayerId(1), at, (Fixed::ZERO, Fixed::ONE))
        .unwrap()
}

fn go(sim: &mut Sim, id: EntityId, to: Vec3) {
    sim.step(&[Command {
        player: PlayerId(1),
        seq: 0,
        kind: CommandKind::Order(Order::Move {
            units: vec![id],
            to,
            queue: false,
        }),
    }]);
}

fn gap(a: Vec3, b: Vec3) -> f64 {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    (dx * dx + dz * dz).sqrt().to_f32() as f64
}

fn sphere(kind: RegionKind, odf: &str, x: i32, z: i32, r: i32) -> Region {
    Region {
        kind,
        odf: odf.into(),
        shape: RegionShape::Sphere {
            centre: v(x, z),
            radius: Fixed::from_int(r),
        },
    }
}

/// Fly a cruiser from the origin to (0, 3000) for `ticks`; the closest it
/// came to `centre`, and the sim.
fn cross(
    regions: Vec<Region>,
    db: Arc<ContentDb>,
    centre: Vec3,
    ticks: u32,
) -> (f64, Sim, EntityId) {
    let mut sim = sim_with(regions, db);
    let ship = put(&mut sim, "fx_cruiser", v(0, 0));
    go(&mut sim, ship, v(0, 3000));
    let mut closest = f64::MAX;
    for _ in 0..ticks {
        sim.step(&[]);
        closest = closest.min(gap(sim.unit(ship).unwrap().pos, centre));
    }
    (closest, sim, ship)
}

#[test]
fn a_ship_routes_round_a_hazard_nebula_and_arrives() {
    let (closest, sim, ship) = cross(
        vec![sphere(RegionKind::Nebula, "fx_mutara", 0, 1500, 400)],
        content(None),
        v(0, 1500),
        3000,
    );
    assert!(
        closest > 400.0,
        "entered the nebula: {closest} from its centre"
    );
    assert!(
        gap(sim.unit(ship).unwrap().pos, v(0, 3000)) < 1.0,
        "ended at {:?}",
        sim.unit(ship).unwrap().pos
    );
}

#[test]
fn an_unknown_nebula_costs_the_rules_default() {
    // `fx_nebula` is not in the content: `NEBULAE_PATHING_COST` (300) applies.
    let (closest, _, _) = cross(
        vec![sphere(RegionKind::Nebula, "fx_nebula", 0, 1500, 400)],
        content(None),
        v(0, 1500),
        3000,
    );
    assert!(closest > 400.0, "{closest}");
}

#[test]
fn open_space_nebulae_and_unplanned_ones_are_flown_through() {
    for odf in ["fx_tachyon", "fx_generated"] {
        let (closest, _, _) = cross(
            vec![sphere(RegionKind::Nebula, odf, 0, 1500, 400)],
            content(None),
            v(0, 1500),
            3000,
        );
        assert!(closest < 1.0, "{odf}: {closest}");
    }
}

#[test]
fn the_rules_switch_planning_off_and_set_the_costs() {
    let off = PathingRules {
        enabled: false,
        ..Default::default()
    };
    let (closest, _, _) = cross(
        vec![sphere(RegionKind::Nebula, "fx_mutara", 0, 1500, 400)],
        content(Some(off)),
        v(0, 1500),
        3000,
    );
    assert!(closest < 1.0, "{closest}");
    // A default nebula cost of open space: an unknown nebula is no hazard.
    let open = PathingRules {
        nebula_cost: 100,
        ..Default::default()
    };
    let (closest, _, _) = cross(
        vec![sphere(RegionKind::Nebula, "fx_nebula", 0, 1500, 400)],
        content(Some(open)),
        v(0, 1500),
        3000,
    );
    assert!(closest < 1.0, "{closest}");
}

#[test]
fn a_ship_goes_through_the_gap_in_a_barrier() {
    // A wall of blocking fields across z = 1500 from x = -4000 to 4000,
    // with one gap of 800 units centred on x = 1800.
    let mut wall = Vec::new();
    for x in (-4000..=1200)
        .step_by(400)
        .chain((2400..=4000).step_by(400))
    {
        wall.push(sphere(
            RegionKind::AsteroidField,
            "fx_asteroids",
            x,
            1500,
            200,
        ));
    }
    let mut sim = sim(wall.clone());
    let ship = put(&mut sim, "fx_cruiser", v(0, 0));
    go(&mut sim, ship, v(0, 3000));
    let mut worst = f64::MAX;
    for _ in 0..4000 {
        sim.step(&[]);
        let p = sim.unit(ship).unwrap().pos;
        for r in &wall {
            if let RegionShape::Sphere { centre, .. } = r.shape {
                worst = worst.min(gap(p, centre));
            }
        }
    }
    let end = sim.unit(ship).unwrap().pos;
    assert!(gap(end, v(0, 3000)) < 1.0, "ended at {:?}", end.to_f32());
    assert!(worst >= 200.0 + 75.0 - 1.0, "touched a field: {worst}");
}

/// 900 * (sin, cos) of k * 30 degrees, rounded: a ring of field centres.
const RING: [(i32, i32); 12] = [
    (0, 900),
    (450, 779),
    (779, 450),
    (900, 0),
    (779, -450),
    (450, -779),
    (0, -900),
    (-450, -779),
    (-779, -450),
    (-900, 0),
    (-779, 450),
    (-450, 779),
];

#[test]
fn an_enclosed_goal_stops_at_the_nearest_reachable_point() {
    // Twelve fields of 300 on a ring of 900 round (0, 3000): no way in.
    let ring: Vec<Region> = RING
        .iter()
        .map(|&(x, z)| sphere(RegionKind::AsteroidField, "fx_asteroids", x, 3000 + z, 300))
        .collect();
    let mut sim = sim(ring);
    let ship = put(&mut sim, "fx_cruiser", v(0, 0));
    go(&mut sim, ship, v(0, 3000));
    for _ in 0..3000 {
        sim.step(&[]);
    }
    let u = sim.unit(ship).unwrap();
    assert_eq!(u.target, None, "order still open at {:?}", u.pos.to_f32());
    assert_eq!(u.speed, Fixed::ZERO);
    let d = gap(u.pos, v(0, 3000));
    // Outside the ring, perhaps in a notch between two fields, touching
    // none (each 300 + body 75), on the near side.
    assert!(
        (1100.0..1700.0).contains(&d),
        "stopped {d} from the goal at {:?}",
        u.pos.to_f32()
    );
    for &(x, z) in &RING {
        assert!(
            gap(u.pos, v(x, 3000 + z)) >= 375.0 - 1.0,
            "touching the field at {x}, {z}"
        );
    }
    assert!(u.pos.z < Fixed::from_int(3000));
}

#[test]
fn a_leg_with_nothing_on_it_is_untouched() {
    // A nebula and a planet well off the course change nothing: the same
    // motion, tick by tick, as with planning switched off (the digests
    // differ only by the rules in the content hash).
    let run = |pathing: Option<PathingRules>| {
        let mut out = Vec::new();
        let mut sim = sim_with(
            vec![sphere(RegionKind::Nebula, "fx_mutara", 3000, 1500, 400)],
            content(pathing),
        );
        put(&mut sim, "fx_moon", v(-3000, 1500));
        let ship = put(&mut sim, "fx_cruiser", v(0, 0));
        go(&mut sim, ship, v(0, 3000));
        for _ in 0..1200 {
            sim.step(&[]);
            let u = sim.unit(ship).unwrap();
            out.push((u.pos, u.heading, u.speed, u.omega, u.target));
        }
        out
    };
    let off = PathingRules {
        enabled: false,
        ..Default::default()
    };
    assert_eq!(run(None), run(Some(off)));
}

#[test]
fn a_snapshot_mid_route_continues_on_the_same_route() {
    let db = content(None);
    let mut a = sim_with(
        vec![sphere(RegionKind::Nebula, "fx_mutara", 0, 1500, 400)],
        db.clone(),
    );
    let ship = put(&mut a, "fx_cruiser", v(0, 0));
    go(&mut a, ship, v(0, 3000));
    for _ in 0..300 {
        a.step(&[]);
    }
    let mut b = Sim::from_snapshot(&a.snapshot(), Some(db)).unwrap();
    for _ in 0..900 {
        a.step(&[]);
        b.step(&[]);
        assert_eq!(a.digest(), b.digest());
    }
}

/// The route round a nebula pins the planner's arithmetic on every CI target.
#[test]
fn the_route_digest_is_pinned() {
    let (_, sim, _) = cross(
        vec![sphere(RegionKind::Nebula, "fx_mutara", 0, 1500, 400)],
        content(None),
        v(0, 1500),
        1500,
    );
    let (_, again, _) = cross(
        vec![sphere(RegionKind::Nebula, "fx_mutara", 0, 1500, 400)],
        content(None),
        v(0, 1500),
        1500,
    );
    assert_eq!(sim.digest(), again.digest());
    assert_eq!(sim.digest(), ROUTE_DIGEST, "got {:#x}", sim.digest());
}

const ROUTE_DIGEST: u64 = 0x304e_b8fc_2e75_68e4;

#[test]
fn a_group_crossing_a_nebula_keeps_out_of_it() {
    // Thirty cruisers in a line, each ordered straight across a big Mutara
    // nebula to its own point: thirty goals, one grid, cached fields.
    let centre = v(0, 2500);
    let mut sim = sim(vec![sphere(RegionKind::Nebula, "fx_mutara", 0, 2500, 800)]);
    let ships: Vec<_> = (0..30)
        .map(|k| put(&mut sim, "fx_cruiser", v(-1450 + k * 100, 0)))
        .collect();
    let orders: Vec<Command> = ships
        .iter()
        .enumerate()
        .map(|(k, &id)| Command {
            player: PlayerId(1),
            seq: k as u32,
            kind: CommandKind::Order(Order::Move {
                units: vec![id],
                to: v(-1450 + k as i32 * 100, 5000),
                queue: false,
            }),
        })
        .collect();
    sim.step(&orders);
    let mut closest = f64::MAX;
    for _ in 0..1500 {
        sim.step(&[]);
        for &id in &ships {
            closest = closest.min(gap(sim.unit(id).unwrap().pos, centre));
        }
    }
    assert!(closest > 800.0, "a ship entered the nebula: {closest}");
}

#[test]
fn a_goal_in_a_nebulas_safety_halo_is_still_reached_round_it() {
    // (0, 2150) is 650 from the centre: outside the nebula (400), inside
    // the halo planned round (720). The way there must not cut the nebula.
    let centre = v(0, 1500);
    let mut sim = sim(vec![sphere(RegionKind::Nebula, "fx_mutara", 0, 1500, 400)]);
    let ship = put(&mut sim, "fx_cruiser", v(0, 0));
    go(&mut sim, ship, v(0, 2150));
    let mut closest = f64::MAX;
    for _ in 0..3000 {
        sim.step(&[]);
        closest = closest.min(gap(sim.unit(ship).unwrap().pos, centre));
    }
    assert!(closest > 400.0, "entered the nebula: {closest}");
    assert!(
        gap(sim.unit(ship).unwrap().pos, v(0, 2150)) < 1.0,
        "ended at {:?}",
        sim.unit(ship).unwrap().pos.to_f32()
    );
}

#[test]
fn a_gap_sim21_can_fly_is_not_called_unreachable() {
    // Two fields of 400 with 300 units between their edges: room for a body
    // of 75 on either side, so the ship goes through and arrives.
    let mut sim = sim(vec![
        sphere(RegionKind::AsteroidField, "fx_asteroids", -550, 1500, 400),
        sphere(RegionKind::AsteroidField, "fx_asteroids", 550, 1500, 400),
    ]);
    let ship = put(&mut sim, "fx_cruiser", v(0, 0));
    go(&mut sim, ship, v(0, 3000));
    for _ in 0..3000 {
        sim.step(&[]);
    }
    assert!(
        gap(sim.unit(ship).unwrap().pos, v(0, 3000)) < 1.0,
        "ended at {:?}",
        sim.unit(ship).unwrap().pos.to_f32()
    );
}

#[test]
fn a_craft_wedged_in_a_pocket_keeps_its_order() {
    // Four fields of 300 round the ship, 380 from it: it fits (300 + 75),
    // but its passable pocket is tiny. The planner must not end the order.
    let regions = [(380, 0), (-380, 0), (0, 380), (0, -380)]
        .iter()
        .map(|&(x, z)| sphere(RegionKind::AsteroidField, "fx_asteroids", x, z, 300))
        .collect();
    let mut sim = sim(regions);
    let ship = put(&mut sim, "fx_cruiser", v(0, 0));
    go(&mut sim, ship, v(0, 3000));
    for _ in 0..10 {
        sim.step(&[]);
    }
    assert_eq!(sim.unit(ship).unwrap().target, Some(v(0, 3000)));
}

/// The cache is a memo: a sim whose cache is thrown away every tick (a
/// snapshot round trip) runs the same digests as one that keeps it, with
/// two owners, two body sizes and many goals churning it.
#[test]
fn a_cold_cache_every_tick_runs_the_same_digests() {
    let db = content(None);
    let regions = vec![
        sphere(RegionKind::Nebula, "fx_mutara", 0, 2500, 800),
        sphere(RegionKind::AsteroidField, "fx_asteroids", 2500, 2500, 400),
    ];
    let mut warm = sim_with(regions, db.clone());
    put(&mut warm, "fx_moon", v(-2500, 2500));
    let mut orders = Vec::new();
    for k in 0..12 {
        let owner = 1 + (k % 2) as u8;
        let key = warm
            .content()
            .unwrap()
            .key(if k % 3 == 0 {
                "fx_frigate"
            } else {
                "fx_cruiser"
            })
            .unwrap();
        let id = warm
            .spawn_def(
                key,
                PlayerId(owner),
                v(-3000 + k * 500, 0),
                (Fixed::ZERO, Fixed::ONE),
            )
            .unwrap();
        orders.push(Command {
            player: PlayerId(owner),
            seq: k as u32,
            kind: CommandKind::Order(Order::Move {
                units: vec![id],
                to: v(3000 - k * 500, 5000),
                queue: false,
            }),
        });
    }
    warm.step(&orders);
    let mut cold = Sim::from_snapshot(&warm.snapshot(), Some(db.clone())).unwrap();
    for _ in 0..200 {
        warm.step(&[]);
        cold = Sim::from_snapshot(&cold.snapshot(), Some(db.clone())).unwrap();
        cold.step(&[]);
        assert_eq!(warm.digest(), cold.digest(), "tick {}", warm.tick);
    }
}

/// The smooth model's craft at rest next to an impassable nebula: the path's
/// first cells lie within the model's arrival radius, so the aim point is the
/// first cell beyond it, which a straight line may reach across the nebula's
/// corner. The craft must still keep out of the nebula itself.
/// Fleet Operations rules (the smooth model) with a cruiser that has a capital
/// ship's turn limits and `speed` units per second, and an impassable nebula.
fn fo4_content(speed: i32) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut b = ContentDb::builder().profile(sta_content::Profile::Fo4);
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    for u in fake.units() {
        let mut u = u.clone();
        if u.odf == "fx_cruiser" {
            u.radius = Fixed::from_int(100);
            u.movement.max_speed = Fixed::from_int(speed);
            u.movement.combat_speed = Fixed::from_int(speed);
            u.movement.turn_at_rest = Fixed::ONE;
            u.movement.turn_alpha_at_rest = Fixed::ONE;
            u.movement.forward_accel = Fixed::from_f64(0.45);
            u.movement.turn_omega = Fixed::from_f64(0.105);
            u.movement.turn_alpha = Fixed::from_f64(0.25);
            u.movement.too_close_to_turn = Fixed::from_int(120);
            u.movement.path_lead = Fixed::from_int(300);
        }
        b = b.unit(u);
    }
    let b = b.unit(nebula("fx_wall", "effectRadius = 400\npathing_cost = -1\n"));
    Arc::new(b.build().unwrap())
}

#[test]
fn a_craft_at_rest_beside_an_impassable_nebula_does_not_cut_its_corner() {
    let db = fo4_content(135);
    let mut sim = sim_with(vec![sphere(RegionKind::Nebula, "fx_wall", 0, 650, 400)], db);
    let ship = put(&mut sim, "fx_cruiser", v(0, 0));
    go(&mut sim, ship, v(0, 3000));
    let mut closest = f64::MAX;
    for _ in 0..3500 {
        sim.step(&[]);
        closest = closest.min(gap(sim.unit(ship).unwrap().pos, v(0, 650)));
    }
    assert!(closest > 400.0, "entered the nebula: {closest} from its centre");
    assert!(
        gap(sim.unit(ship).unwrap().pos, v(0, 3000)) < 150.0,
        "ended at {:?}",
        sim.unit(ship).unwrap().pos
    );
}

/// A slow craft's way round a long wall of blocking fields takes longer than
/// the stuck-order watch's two minutes standing still, and never gets nearer
/// the goal on the way: it keeps flying, and the order ends by arrival, not by
/// the watch (real maps have walls of up to 36 000 units).
#[test]
fn a_slow_craft_flying_round_a_long_wall_is_not_cut_off() {
    let wall: Vec<Region> = (-10..=10)
        .map(|i| sphere(RegionKind::AsteroidField, "trigger_blocking", i * 400, 500, 400))
        .collect();
    let mut sim = sim_with(wall, fo4_content(30));
    let ship = put(&mut sim, "fx_cruiser", v(0, 0));
    go(&mut sim, ship, v(0, 1500));
    let mut blocked = 0;
    let mut ticks = 0;
    while sim.unit(ship).unwrap().target.is_some() && ticks < 20_000 {
        let out = sim.step(&[]);
        blocked += out.events.iter().filter(|e| matches!(e, crate::SimEvent::OrderBlocked { .. })).count();
        ticks += 1;
    }
    let end = sim.unit(ship).unwrap().pos;
    assert_eq!(blocked, 0, "the order was ended as blocked after {ticks} ticks at {end:?}");
    assert!(gap(end, v(0, 1500)) < 150.0, "ended at {end:?} after {ticks} ticks");
    assert!(ticks > 3600, "the detour should outlast two minutes ({ticks} ticks)");
}
