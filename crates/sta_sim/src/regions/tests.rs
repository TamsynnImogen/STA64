use std::sync::Arc;

use sta_content::{MapDef, RegionShape};
use sta_core::{PlayerId, TeamId, Vec3};

use super::*;
use crate::WorldSource;
use crate::command::{Command, CommandKind, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

fn field(kind: RegionKind, shape: RegionShape) -> Region {
    Region { kind, odf: "fx_asteroids".into(), shape }
}

/// A cruiser at the origin ordered 3000 units east, past a field on the way
/// (offset a little in Z so the ship can slide round, as a real order never
/// lines up exactly). Returns the closest it got to `centre` and where it
/// ended.
fn fly(regions: Vec<Region>, centre: Vec3) -> (Fixed, Vec3) {
    fly_with(regions, centre, sta_content::fake::db())
}

fn fly_with(regions: Vec<Region>, centre: Vec3, db: sta_content::ContentDb) -> (Fixed, Vec3) {
    let mut map = MapDef::empty();
    map.regions = regions;
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let db = Arc::new(db);
    let cruiser = db.key("fx_cruiser").unwrap();
    let setup = GameSetup { seed: 3, map: Arc::new(map), content: db, players: vec![seat(1)], options: GameOptions::default() };
    let mut sim = Sim::from_setup(&setup);
    let id = sim.spawn_def(cruiser, PlayerId(1), v(0, 0), (Fixed::ONE, Fixed::ZERO)).unwrap();
    let go = Command {
        player: PlayerId(1),
        seq: 0,
        kind: CommandKind::Order(Order::Move { units: vec![id], to: v(3000, 0), queue: false }),
    };
    sim.step(&[go]);
    let mut closest = Fixed::from_int(1_000_000);
    for _ in 0..4000 {
        sim.step(&[]);
        let p = sim.unit(id).unwrap().pos;
        let (dx, dz) = (p.x - centre.x, p.z - centre.z);
        closest = closest.min((dx * dx + dz * dz).sqrt());
    }
    (closest, sim.unit(id).unwrap().pos)
}

#[test]
fn units_do_not_enter_an_asteroid_field() {
    let centre = v(1500, 20);
    let r = Fixed::from_int(400);
    let (closest, end) = fly(vec![field(RegionKind::AsteroidField, RegionShape::Sphere { centre, radius: r })], centre);
    assert!(closest >= r - Fixed::from_f64(0.01), "got within {closest:?} of the centre");
    assert!(end.x > Fixed::from_int(2900), "ended at {end:?}");
}

#[test]
fn the_same_flight_without_a_field_crosses_it() {
    let centre = v(1500, 20);
    let (closest, end) = fly(vec![], centre);
    assert!(closest < Fixed::from_int(100));
    assert!(end.x > Fixed::from_int(2990));
}

#[test]
fn nebulae_are_not_obstacles() {
    // Not solids: nothing pushes a unit out. SIM-23's planner routes round
    // them by cost, so it is switched off here (`USE_PATH_PLANNING = 0`).
    let centre = v(1500, 20);
    let off = sta_content::PathingRules { enabled: false, ..Default::default() };
    let db = sta_content::fake::db().with_rules(sta_content::Rules::default().with_pathing(off));
    let (closest, _) = fly_with(vec![field(RegionKind::Nebula, RegionShape::Sphere { centre, radius: Fixed::from_int(400) })], centre, db);
    assert!(closest < Fixed::from_int(100));
}

#[test]
fn box_fields_block_too() {
    let (min, max) = (v(1200, -300), v(1800, 300));
    let mut map_end = fly(vec![field(RegionKind::AsteroidField, RegionShape::Box { min, max })], v(1500, 0)).1;
    map_end.z = Fixed::ZERO;
    assert!(!(min.x < map_end.x && map_end.x < max.x), "ended inside the box: {map_end:?}");
}
