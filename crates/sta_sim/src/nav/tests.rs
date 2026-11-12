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
    let mut u = UnitDef::new(odf, "nebula_obj");
    let text = format!("classLabel = \"nebula_obj\"\nunitName = \"{odf}\"\n{text}");
    u.raw = Arc::new(sta_odf::resolve(odf, &mut |_| Some(text.clone())).unwrap());
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
        players: vec![seat(1)],
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

#[test]
#[allow(clippy::disallowed_methods)] // platform trig only places the test's regions
fn an_enclosed_goal_stops_at_the_nearest_reachable_point() {
    // Twelve fields of 300 on a ring of 900 round (0, 3000): no way in.
    let ring: Vec<Region> = (0..12)
        .map(|k| {
            let a = k as f64 * std::f64::consts::PI / 6.0;
            let (x, z) = (
                (900.0 * a.sin()).round() as i32,
                (3000.0 + 900.0 * a.cos()).round() as i32,
            );
            sphere(RegionKind::AsteroidField, "fx_asteroids", x, z, 300)
        })
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
    // Outside the ring (900 + 300 + body 75), on the near side, and no
    // further out than the planner's inflated edge plus a cell or two.
    assert!(
        (1275.0..1700.0).contains(&d),
        "stopped {d} from the goal at {:?}",
        u.pos.to_f32()
    );
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

const ROUTE_DIGEST: u64 = 0xc1c7_c3e6_8c13_c419;

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
