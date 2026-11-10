use std::sync::Arc;

use sta_content::{ContentDb, MapDef, Region, RegionKind, RegionShape};
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

/// Fake content with bodies: cruiser circle 100 (body 75), moon 400 (body
/// 300, a planet-class resource with no node), station 200 (body 150).
fn content() -> Arc<ContentDb> {
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
            "fx_station" => 200,
            _ => 0,
        });
        b = b.unit(u);
    }
    Arc::new(b.build().unwrap())
}

fn sim(regions: Vec<Region>) -> Sim {
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
        content: content(),
        players: vec![seat(1), seat(2)],
        options: GameOptions::default(),
    })
}

fn put(sim: &mut Sim, odf: &str, owner: u8, at: Vec3) -> EntityId {
    let key = sim.content().unwrap().key(odf).unwrap();
    sim.spawn_def(key, PlayerId(owner), at, (Fixed::ZERO, Fixed::ONE))
        .unwrap()
}

fn go(sim: &mut Sim, owner: u8, ids: &[EntityId], to: Vec3) {
    let kind = CommandKind::Order(Order::Move {
        units: ids.to_vec(),
        to,
        queue: false,
    });
    sim.step(&[Command {
        player: PlayerId(owner),
        seq: 0,
        kind,
    }]);
}

fn gap(a: Vec3, b: Vec3) -> f64 {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    (dx * dx + dz * dz).sqrt().to_f32() as f64
}

/// Overlap allowed between two bodies (units): a tenth of the radii's sum.
const TOLERANCE: f64 = 15.0;

fn crossing(owner_b: u8) -> f64 {
    let mut sim = sim(Vec::new());
    let zs = [-200, 0, 200];
    let a: Vec<_> = zs
        .iter()
        .map(|&z| put(&mut sim, "fx_cruiser", 1, v(-1500, z)))
        .collect();
    let b: Vec<_> = zs
        .iter()
        .map(|&x| put(&mut sim, "fx_cruiser", owner_b, v(x, -1500)))
        .collect();
    for (i, id) in a.iter().enumerate() {
        go(&mut sim, 1, &[*id], v(1500, zs[i]));
    }
    for (i, id) in b.iter().enumerate() {
        go(&mut sim, owner_b, &[*id], v(zs[i], 1500));
    }
    let all: Vec<_> = a.iter().chain(&b).copied().collect();
    let mut worst = 0.0f64;
    for _ in 0..1800 {
        sim.step(&[]);
        for (i, x) in all.iter().enumerate() {
            for y in &all[i + 1..] {
                if let (Some(p), Some(q)) = (sim.unit(*x), sim.unit(*y)) {
                    worst = worst.max(150.0 - gap(p.pos, q.pos));
                }
            }
        }
    }
    // Everyone made it to its own point.
    for (i, id) in a.iter().enumerate() {
        let at = sim.unit(*id).unwrap().pos;
        assert!(gap(at, v(1500, zs[i])) < 10.0, "{at:?}");
    }
    worst
}

#[test]
fn groups_crossing_do_not_overlap_beyond_tolerance() {
    let own = crossing(1);
    let enemy = crossing(2);
    assert!(own <= TOLERANCE, "own groups overlapped by {own}");
    assert!(enemy <= TOLERANCE, "enemy groups overlapped by {enemy}");
}

#[test]
fn a_ship_ordered_through_a_planet_flies_round_it() {
    let mut sim = sim(Vec::new());
    let planet = put(&mut sim, "fx_moon", 0, v(0, 0));
    let ship = put(&mut sim, "fx_cruiser", 1, v(0, -1500));
    go(&mut sim, 1, &[ship], v(0, 1500));
    let mut closest = f64::MAX;
    for _ in 0..2400 {
        sim.step(&[]);
        closest = closest.min(gap(
            sim.unit(ship).unwrap().pos,
            sim.unit(planet).unwrap().pos,
        ));
    }
    assert!(closest >= 300.0 + 75.0 - TOLERANCE, "came within {closest}");
    assert!(
        gap(sim.unit(ship).unwrap().pos, v(0, 1500)) < 1.0,
        "ended at {:?}",
        sim.unit(ship).unwrap().pos
    );
}

#[test]
fn a_ship_ordered_through_an_asteroid_field_flies_round_it() {
    let shape = RegionShape::Sphere {
        centre: v(0, 1500),
        radius: Fixed::from_int(300),
    };
    let field = Region {
        kind: RegionKind::AsteroidField,
        odf: "fx_asteroids".into(),
        shape,
    };
    let mut sim = sim(vec![field]);
    let ship = put(&mut sim, "fx_cruiser", 1, v(0, 0));
    go(&mut sim, 1, &[ship], v(0, 3000));
    let mut closest = f64::MAX;
    for _ in 0..2400 {
        sim.step(&[]);
        closest = closest.min(gap(sim.unit(ship).unwrap().pos, v(0, 1500)));
    }
    assert!(closest >= 300.0 + 75.0 - TOLERANCE, "came within {closest}");
    assert!(gap(sim.unit(ship).unwrap().pos, v(0, 3000)) < 1.0);
}

#[test]
fn a_destination_inside_a_planet_ends_at_its_edge() {
    let mut sim = sim(Vec::new());
    put(&mut sim, "fx_moon", 0, v(0, 0));
    let ship = put(&mut sim, "fx_cruiser", 1, v(-1500, 0));
    go(&mut sim, 1, &[ship], v(50, 0));
    for _ in 0..1200 {
        sim.step(&[]);
    }
    let d = gap(sim.unit(ship).unwrap().pos, v(0, 0));
    assert!((370.0..=400.0).contains(&d), "stopped {d} from the centre");
    assert_eq!(sim.unit(ship).unwrap().speed, Fixed::ZERO);
}

#[test]
fn own_stations_are_not_solid_but_enemy_ones_are() {
    for (station_owner, solid) in [(1u8, false), (2u8, true)] {
        let mut sim = sim(Vec::new());
        let s = put(&mut sim, "fx_station", station_owner, v(0, 0));
        let ship = put(&mut sim, "fx_cruiser", 1, v(0, -1000));
        go(&mut sim, 1, &[ship], v(0, 1000));
        let mut closest = f64::MAX;
        for _ in 0..1200 {
            sim.step(&[]);
            closest = closest.min(gap(sim.unit(ship).unwrap().pos, v(0, 0)));
        }
        assert_eq!(sim.unit(s).unwrap().pos, v(0, 0), "stations never move");
        assert_eq!(closest > 100.0, solid, "closest {closest}");
    }
}
