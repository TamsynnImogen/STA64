//! Install-gated check of SIM-21's steering on a real map: a ship ordered
//! straight through the biggest asteroid field of `AsteroidZone` (Fleet
//! Operations 4) arrives on the far side instead of stalling at the field.
//!
//! Gate: `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`, empty mounts `Data/`
//! alone; `STA64_TEST_MAP` names another map). Skipped when unset.

use std::path::Path;
use std::sync::Arc;

use sta_content::{ContentDb, MapDef, OdfIndex, RegionKind, RegionShape};
use sta_core::{Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Order};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::world::EntityKind;
use sta_sim::{Sim, WorldSource};
use sta_vfs::Vfs;

fn at(x: Fixed, z: Fixed) -> Vec3 {
    Vec3 {
        x,
        y: Fixed::ZERO,
        z,
    }
}

fn dist(a: Vec3, b: Vec3) -> f64 {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    (dx * dx + dz * dz).sqrt().to_f32() as f64
}

#[test]
fn a_ship_crosses_a_map_with_asteroid_fields() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
        return;
    };
    let modname = std::env::var("STA64_TEST_MOD").unwrap_or_default();
    let name = std::env::var("STA64_TEST_MAP").unwrap_or_else(|_| "AsteroidZone".into());
    let vfs = Vfs::mount_install(
        Path::new(&root),
        Some(modname.as_str()).filter(|m| !m.is_empty()),
    )
    .unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let db = Arc::new(db);
    let path = vfs.list("bzn").into_iter().find(|p| {
        p.to_ascii_lowercase()
            .ends_with(&format!("{}.bzn", name.to_ascii_lowercase()))
    });
    let Some(path) = path else { return };
    let bzn = sta_bzn::parse(&vfs.read(&path).unwrap().unwrap()).unwrap();
    let map = MapDef::convert_bzn_in(&bzn, &db, &vfs, &index).map;

    let (centre, radius) = map
        .regions
        .iter()
        .filter(|r| r.kind == RegionKind::AsteroidField)
        .filter_map(|r| match r.shape {
            RegionShape::Sphere { centre, radius } => Some((centre, radius)),
            _ => None,
        })
        .max_by_key(|f| f.1)
        .expect("the map has an asteroid field");
    let ship = db
        .units()
        .filter(|u| {
            u.class.0 == "craft" && u.movement.max_speed > Fixed::ZERO && u.hull > Fixed::ZERO
        })
        .map(|u| u.key)
        .next()
        .expect("a craft");

    let seat = PlayerSetup {
        id: PlayerId(1),
        team: TeamId(1),
        race: String::new(),
        control: Control::Idle,
    };
    let setup = GameSetup {
        seed: 1,
        map: Arc::new(map),
        content: db.clone(),
        players: vec![seat],
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let reach = radius + Fixed::from_int(800);
    let (from, to) = (
        at(centre.x - reach, centre.z),
        at(centre.x + reach, centre.z),
    );
    let id = sim
        .spawn_def(ship, PlayerId(1), from, (Fixed::ONE, Fixed::ZERO))
        .unwrap();
    assert_eq!(sim.unit(id).unwrap().kind, EntityKind::Craft);
    let kind = CommandKind::Order(Order::Move {
        units: vec![id],
        to,
        queue: false,
    });
    sim.step(&[Command {
        player: PlayerId(1),
        seq: 0,
        kind,
    }]);
    let (mut closest, mut arrived) = (f64::MAX, false);
    for _ in 0..30_000 {
        sim.step(&[]);
        let p = sim.unit(id).unwrap().pos;
        closest = closest.min(dist(p, centre));
        if dist(p, to) < 50.0 {
            arrived = true;
            break;
        }
    }
    let field = radius.to_f32() as f64;
    assert!(
        closest >= field - 1.0,
        "entered the field: {closest} from its centre, radius {field}"
    );
    assert!(
        arrived,
        "stopped at {:?}, goal {to:?}",
        sim.unit(id).unwrap().pos
    );
}
