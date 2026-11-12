//! Install-gated check of SIM-23's planner on real data: the install's
//! `RTS_CFG.h` PATHING block is read, and a ship ordered straight across the
//! first isolated map nebula whose ODF `pathing_cost` exceeds open space flies round
//! it and arrives.
//!
//! Gate: `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`, empty mounts `Data/`
//! alone; `STA64_TEST_MAP` names one map instead of searching). Skipped when
//! unset.

use std::path::Path;
use std::sync::Arc;

use sta_content::{ContentDb, MapDef, OdfIndex, RegionKind, RegionShape};
use sta_core::{Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Order};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
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
    let (dx, dz) = ((a.x - b.x).to_f32() as f64, (a.z - b.z).to_f32() as f64);
    (dx * dx + dz * dz).sqrt()
}

#[test]
fn a_ship_routes_round_an_installed_hazard_nebula() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
        return;
    };
    let modname = std::env::var("STA64_TEST_MOD").unwrap_or_default();
    let only = std::env::var("STA64_TEST_MAP").ok();
    let vfs = Vfs::mount_install(
        Path::new(&root),
        Some(modname.as_str()).filter(|m| !m.is_empty()),
    )
    .unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let rules = db.rules().pathing();
    eprintln!("pathing rules: {rules:?}");
    assert!(rules.enabled, "USE_PATH_PLANNING is off in this install");
    let db = Arc::new(db);
    let hazard = |odf: &str| {
        db.key(odf)
            .and_then(|k| db.unit(k))
            .and_then(|d| d.raw.i64("pathing_cost"))
            .unwrap_or(rules.nebula_cost)
    };

    // The first map (by path) with a sphere nebula dearer than open space.
    let mut found = None;
    let mut paths = vfs.list("bzn");
    paths.sort();
    for path in paths {
        let lower = path.to_ascii_lowercase();
        if !lower.ends_with(".bzn")
            || only
                .as_ref()
                .is_some_and(|m| !lower.ends_with(&format!("{}.bzn", m.to_ascii_lowercase())))
        {
            continue;
        }
        let Some(Ok(bytes)) = vfs.read(&path) else {
            continue;
        };
        let Ok(bzn) = sta_bzn::parse(&bytes) else {
            continue;
        };
        let map = MapDef::convert_bzn_in(&bzn, &db, &vfs, &index).map;
        // An isolated one: inside a nebula field (Armada II's Borg missions
        // are carpets of overlapping Mutara nebulae) the cheapest way across
        // is through, and the planner rightly takes it.
        let centres: Vec<Vec3> = map
            .regions
            .iter()
            .filter_map(|r| match r.shape {
                RegionShape::Sphere { centre, .. } => Some(centre),
                RegionShape::Box { min, max } => Some(at(
                    (min.x + max.x) / Fixed::from_int(2),
                    (min.z + max.z) / Fixed::from_int(2),
                )),
                _ => None,
            })
            .collect();
        let nebula = map.regions.iter().find_map(|r| match r.shape {
            RegionShape::Sphere { centre, radius }
                if r.kind == RegionKind::Nebula
                    && hazard(&r.odf) > rules.default_cost
                    && centres
                        .iter()
                        .filter(|c| dist(**c, centre) < 4000.0)
                        .count()
                        == 1 =>
            {
                Some((r.odf.clone(), centre, radius))
            }
            _ => None,
        });
        if let Some(n) = nebula {
            found = Some((path, map, n));
            break;
        }
    }
    let Some((path, map, (odf, centre, radius))) = found else {
        eprintln!("no map with a hazard nebula; nothing to check");
        return;
    };

    eprintln!(
        "{path}: {odf} (pathing_cost {}) radius {} at {:?}",
        hazard(&odf),
        radius.to_f32(),
        centre.to_f32()
    );
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
    let reach = radius * rules.nebula_safety + Fixed::from_int(800);
    let (from, to) = (
        at(centre.x - reach, centre.z),
        at(centre.x + reach, centre.z),
    );
    let id = sim
        .spawn_def(ship, PlayerId(1), from, (Fixed::ONE, Fixed::ZERO))
        .unwrap();
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
    let mut closest = f64::MAX;
    let mut done = false;
    for _ in 0..30_000 {
        sim.step(&[]);
        let u = sim.unit(id).unwrap();
        closest = closest.min(dist(u.pos, centre));
        if u.target.is_none() {
            done = true;
            break;
        }
    }
    let end = sim.unit(id).unwrap().pos;
    let r = radius.to_f32() as f64;
    eprintln!(
        "closest {closest:.1} (radius {r:.1}); ended {:.1} from the goal",
        dist(end, to)
    );
    assert!(done, "order never ended; at {:?}", end.to_f32());
    assert!(
        closest >= r,
        "entered the nebula: {closest} from its centre, radius {r}"
    );
}
