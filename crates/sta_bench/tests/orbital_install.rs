//! Install-gated (SIM-62): every orbital station ODF (class `p_mining`) of the
//! install snaps to a planet's orbit slot on a real map and is refused in open
//! space; a planet takes one station. What the original does is read from the
//! executable (static): see `sta_sim::production::orbit`.
//!
//! Gate: `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`, empty mounts `Data/` alone;
//! `STA64_TEST_MAP` names a map, default: the first map with planets that have
//! a slot). Skipped when unset. Installs without orbital stations (Armada I,
//! Fleet Operations, Roots) pass vacuously after checking that no planet
//! offers a slot.

use std::path::Path;
use std::sync::Arc;

use sta_content::{ContentDb, MapDef, OdfIndex};
use sta_core::{Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::production::orbit;
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};
use sta_vfs::Vfs;

fn dist(a: Vec3, b: Vec3) -> f64 {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    f64::from((dx * dx + dz * dz).sqrt().to_f32())
}

#[test]
fn every_orbital_station_snaps_to_a_planet_and_is_refused_in_open_space() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
    let modname = std::env::var("STA64_TEST_MOD").unwrap_or_default();
    let vfs = Vfs::mount_install(Path::new(&root), Some(modname.as_str()).filter(|m| !m.is_empty())).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let db = Arc::new(db);
    let stations: Vec<_> = db.units().filter(|u| orbit::is_orbital(u)).map(|u| u.key).collect();
    let rig_of = |what| db.units().find(|u| u.class.0 == "constructionrig" && u.build.list.iter().any(|r| r.key == what)).map(|u| u.key);
    eprintln!("{root} {modname}: {} orbital stations: {:?}", stations.len(), stations.iter().map(|k| db.unit(*k).unwrap().odf.clone()).collect::<Vec<_>>());
    for u in db.units() {
        if u.class.0 == "p_mining" {
            assert!(orbit::is_orbital(u), "{}: class p_mining is an orbital station", u.odf);
        } else {
            assert!(!orbit::is_orbital(u), "{}", u.odf);
        }
    }

    let wanted = std::env::var("STA64_TEST_MAP").ok();
    let mut maps = vfs.list("bzn");
    maps.retain(|p| p.to_ascii_lowercase().ends_with(".bzn") && wanted.as_ref().is_none_or(|w| p.to_ascii_lowercase().ends_with(&format!("{}.bzn", w.to_ascii_lowercase()))));
    maps.sort();
    let new_sim = |map: &Arc<MapDef>| {
        let players = vec![
            PlayerSetup { id: PlayerId(1), team: TeamId(1), race: String::new(), control: Control::Human },
            PlayerSetup { id: PlayerId(2), team: TeamId(2), race: String::new(), control: Control::Idle },
        ];
        let mut sim = Sim::from_setup(&GameSetup { seed: 1, map: map.clone(), content: db.clone(), players, options: GameOptions::default() });
        for slot in 0..4u8 {
            let e = Edit::Cargo(PlayerId(1), slot, Fixed::from_int(100_000));
            sim.step(&[Command { player: PlayerId::BENCH, seq: u32::from(slot), kind: CommandKind::Edit(e) }]);
        }
        sim
    };
    let mut checked = 0;
    for path in maps.iter().take(40) {
        let Some(Ok(bytes)) = vfs.read(path) else { continue };
        let Ok(bzn) = sta_bzn::parse(&bytes) else { continue };
        let map = Arc::new(MapDef::convert_bzn_in(&bzn, &db, &vfs, &index).map);
        let sim = new_sim(&map);
        let (planets, orbiters) = orbit::from_view(&db, &sim.view());
        if planets.is_empty() {
            continue;
        }
        if stations.is_empty() {
            panic!("{path}: planets offer a slot but the install has no orbital station");
        }
        let free: Vec<_> = planets.iter().filter(|p| !orbiters.iter().any(|o| orbit::pick(&planets, *o).is_some_and(|q| q.id == p.id))).copied().collect();
        let Some(p) = free.first().copied() else { continue };
        for &what in &stations {
            let name = db.unit(what).unwrap().odf.clone();
            // The cursor over the planet, then at its reach: the slot is on the orbit.
            for cursor in [p.pos, Vec3 { x: p.pos.x + p.radius, ..p.pos }, Vec3 { z: p.pos.z - p.radius - Fixed::from_int(orbit::REACH - 1), ..p.pos }] {
                let (picked, slot) = (orbit::pick(&planets, cursor), orbit::snap_view(&db, &sim.view(), what, cursor));
                let Some(Ok(slot)) = slot else { panic!("{path} {name}: {cursor:?} picks {picked:?} but {slot:?}") };
                let ring = (p.radius + Fixed::from_int(orbit::ALTITUDE)).to_f32() as f64;
                assert!((dist(slot.at, p.pos) - ring).abs() < 0.05, "{path} {name}: on the orbit");
            }
            // Open space: a point past every planet's reach.
            let far = Vec3 { x: p.pos.x + p.radius + Fixed::from_int(orbit::REACH + 400), ..p.pos };
            if orbit::pick(&planets, far).is_none() {
                assert_eq!(orbit::snap_view(&db, &sim.view(), what, far), Some(Err(orbit::OrbitError::NoPlanet)), "{path} {name}");
            }
            // The sim, with a rig that lists the station.
            let Some(rig) = rig_of(what) else { continue };
            let mut sim = new_sim(&map);
            let h = (Fixed::ONE, Fixed::ZERO);
            let at = Vec3 { x: p.pos.x, y: p.pos.y, z: p.pos.z - p.radius - Fixed::from_int(300) };
            let id = sim.spawn_def(rig, PlayerId(1), at, h).unwrap();
            let order = |at| Command { player: PlayerId(1), seq: 5, kind: CommandKind::Order(Order::Build { builder: id, what, at: Some(at) }) };
            let ok = |out: &sta_sim::TickOutput| !out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. }));
            if !ok(&sim.step(&[order(p.pos)])) {
                eprintln!("{path} {name}: the sim rejects at {p:?} (tech or cost); the rule is checked above");
                continue;
            }
            assert!(!ok(&sim.step(&[order(far)])), "{path} {name}: refused in open space");
            checked += 1;
        }
        if checked > 0 {
            break;
        }
    }
    if !stations.is_empty() {
        assert!(checked > 0, "no station was ordered through the sim on any map");
    }
}
