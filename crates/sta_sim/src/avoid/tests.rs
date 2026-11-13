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

// A craft confined to the map (Fleet Operations' `forceOntoMap`) that rests
// against one of the border asteroid fields used to be held still with its
// order kept: the field counted as "on the leg" although the leg led away from
// it, and when no checked detour existed `steer_around` froze the craft, so
// every later order was accepted and never moved. Authored layout, no game data.

fn fo4_content() -> Arc<ContentDb> {
    let mut craft = sta_content::UnitDef::new("craft", "craft");
    craft.hull = Fixed::from_int(100);
    craft.radius = Fixed::from_int(100);
    craft.movement = sta_content::MoveStats::from_f32(135.0, 2.0, 2.0, 2.0, 1.0);
    // A capital ship's turning limits, as a Fleet Operations hull has them.
    craft.movement.max_speed = Fixed::from_int(135);
    craft.movement.combat_speed = Fixed::from_int(135);
    craft.movement.turn_at_rest = Fixed::ONE;
    craft.movement.turn_alpha_at_rest = Fixed::ONE;
    craft.movement.forward_accel = Fixed::from_f64(0.45);
    craft.movement.turn_omega = Fixed::from_f64(0.105);
    craft.movement.turn_alpha = Fixed::from_f64(0.25);
    craft.movement.too_close_to_turn = Fixed::from_int(120);
    craft.movement.path_lead = Fixed::from_int(300);
    craft.movement.turn_control_angle = Fixed::from_int(20);
    craft.movement.forward_control_distance = Fixed::from_int(70);
    Arc::new(
        ContentDb::builder()
            .profile(sta_content::Profile::Fo4)
            .unit(craft)
            .build()
            .unwrap(),
    )
}

fn field(x: i32, z: i32) -> Region {
    Region {
        kind: RegionKind::AsteroidField,
        odf: "trigger_blocking".into(),
        shape: RegionShape::Sphere {
            centre: v(x, z),
            radius: Fixed::from_int(400),
        },
    }
}

/// A ring of overlapping fields round the east edge of a 6200-unit map.
fn confined_sim(extra: Vec<Region>) -> Sim {
    let mut map = MapDef::empty();
    map.bounds = (v(100, 100), v(6300, 6300));
    map.bounds.0.y = Fixed::from_int(-10);
    map.bounds.1.y = Fixed::from_int(10);
    map.regions = vec![field(4590, 2541), field(4530, 2009), field(4049, 1756)];
    map.regions.extend(extra);
    Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(map),
        content: fo4_content(),
        players: vec![PlayerSetup {
            id: PlayerId(1),
            team: TeamId(1),
            race: "fx".into(),
            control: Control::Idle,
        }],
        options: GameOptions::default(),
    })
}

#[test]
fn a_confined_craft_resting_on_a_border_field_leaves_it() {
    let mut sim = confined_sim(Vec::new());
    // Resting at contact distance (400 + 75) from the first field.
    let id = put(&mut sim, "craft", 1, v(4159, 2354));
    assert_eq!(sim.force_onto_map(id), Some(true));
    let start = sim.unit(id).unwrap().pos;
    go(&mut sim, 1, &[id], v(3799, 2921));
    for _ in 0..600 {
        sim.step(&[]);
    }
    let end = sim.unit(id).unwrap();
    assert!(
        gap(start, end.pos) > 100.0 || end.target.is_none(),
        "frozen at {:?} with order {:?}",
        end.pos,
        end.target
    );
    assert!(gap(end.pos, v(3799, 2921)) < 60.0, "ended at {:?}", end.pos);
}

/// Whatever a player orders across and into a field of obstacles, the order
/// ends (arrival, or the sim ending it) within a bound: a craft never keeps a
/// move order it makes no progress on. Own deterministic generator, so the
/// layout and orders are the same on every run.
#[test]
fn random_orders_across_a_field_of_obstacles_always_end() {
    let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = |n: i32| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 33) as i32 % n
    };
    // A ring of fields round the map (16 at 2900 from its middle) and some
    // scattered ones, with passable gaps between all of them.
    let regions: Vec<Region> = [(6100, 3200), (5879, 4310), (5251, 5251), (4310, 5879), (3200, 6100), (2090, 5879), (1149, 5251), (521, 4310), (300, 3200), (521, 2090), (1149, 1149), (2090, 521), (3200, 300), (4310, 521), (5251, 1149), (5879, 2090), (2000, 2000), (4400, 4300), (1800, 4300), (4300, 1700)]
        .into_iter()
        .map(|(x, z)| field(x, z))
        .collect();
    for confined in [true, false] {
        let mut sim = confined_sim(Vec::new());
        sim.regions = regions.clone();
        let id = put(&mut sim, "craft", 1, v(3200, 3200));
        if !confined {
            sim.step(&[Command {
                player: PlayerId::SCRIPT,
                seq: 0,
                kind: CommandKind::Edit(crate::command::Edit::ForceOntoMap { id, enabled: false }),
            }]);
        }
        for order in 0..40 {
            // Targets anywhere in the map, inside fields and just past their
            // edges as well (a destination the craft cannot settle on ends by
            // the stuck-order watch).
            let to = v(150 + next(6000), 150 + next(6000));
            go(&mut sim, 1, &[id], to);
            let mut ticks = 0;
            while sim.unit(id).unwrap().target.is_some() && ticks < 4500 {
                sim.step(&[]);
                ticks += 1;
                            }
            let u = sim.unit(id).unwrap();
            assert!(
                u.target.is_none(),
                "confined={confined} order {order} to {:?} still alive at {:?} after {ticks} ticks",
                to,
                u.pos
            );
        }
    }
}
