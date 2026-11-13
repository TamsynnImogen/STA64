//! Install-gated: a freighter ordered to mine each kind of resource moon a real map places
//! gathers and delivers it (dilithium and tritanium alike).
//!
//! Gate: `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`, empty mounts `Data/` alone;
//! `STA64_TEST_MAP` names the map, default `Adapt`). Skipped when unset.

use std::path::Path;
use std::sync::Arc;

use sta_content::{ContentDb, MapDef, OdfIndex};
use sta_core::{Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Order};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::world::EntityKind;
use sta_sim::{Sim, SimEvent, WorldSource};
use sta_vfs::Vfs;

#[test]
fn a_freighter_mines_every_moon_kind_a_map_places() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
    let modname = std::env::var("STA64_TEST_MOD").unwrap_or_default();
    let name = std::env::var("STA64_TEST_MAP").unwrap_or_else(|_| "Adapt".into());
    let vfs = Vfs::mount_install(Path::new(&root), Some(modname.as_str()).filter(|m| !m.is_empty())).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let db = Arc::new(db);
    let Some(path) = vfs.list("bzn").into_iter().find(|p| p.to_ascii_lowercase().ends_with(&format!("{}.bzn", name.to_ascii_lowercase()))) else { return };
    let bzn = sta_bzn::parse(&vfs.read(&path).unwrap().unwrap()).unwrap();
    let map = Arc::new(MapDef::convert_bzn_in(&bzn, &db, &vfs, &index).map);
    let new_sim = || {
        let players = vec![
            PlayerSetup { id: PlayerId(1), team: TeamId(1), race: String::new(), control: Control::Human },
            PlayerSetup { id: PlayerId(2), team: TeamId(2), race: String::new(), control: Control::Idle },
        ];
        Sim::from_setup(&GameSetup { seed: 1, map: map.clone(), content: db.clone(), players, options: GameOptions::default() })
    };
    {
        let sim = new_sim();
        let v = sim.view();
        for u in &v.units {
            let Some(d) = db.unit(u.def) else { continue };
            if d.resource.is_some() || d.odf.starts_with("hook_") || d.odf.starts_with("trigger_") {
                let near: Vec<String> = v.units.iter().filter(|o| o.id != u.id && { let (dx, dz) = (o.pos.x - u.pos.x, o.pos.z - u.pos.z); (dx * dx + dz * dz) < Fixed::from_int(2500) }).filter_map(|o| db.unit(o.def).map(|d| d.odf.clone())).collect();
                println!("UNIT {:?} {} owner {:?} selectable {} near {:?}", u.id, d.odf, u.owner, u.flags.contains(sta_sim::view::UnitFlags::SELECTABLE), near);
            }
        }
    }
    // One moon of each resource definition the map placed.
    let mut moons: Vec<(String, Vec3, sta_sim::EntityId)> = Vec::new();
    for u in &new_sim().view().units {
        let Some(d) = db.unit(u.def) else { continue };
        if d.resource.is_some() && !moons.iter().any(|m| m.0 == d.odf) {
            println!("moon def {} class {} resource {:?} kind {:?}", d.odf, d.class.0, d.resource, sta_sim::world::EntityKind::of_def(d));
            moons.push((d.odf.clone(), u.pos, u.id));
        }
    }
    assert!(!moons.is_empty(), "the map places resource moons");
    // A freighter and a station of one race.
    let miner = db.units().find(|u| u.miner && u.movement.max_speed > Fixed::ZERO && u.race != "norace" && u.odf.ends_with("miner")).or_else(|| db.units().find(|u| u.miner && u.movement.max_speed > Fixed::ZERO)).expect("a miner").key;
    let station = db.units().find(|u| u.race == db.unit(miner).unwrap().race && EntityKind::of_def(u) == EntityKind::Station && u.movement.max_speed == Fixed::ZERO).expect("a station of that race").key;
    let mut failed = Vec::new();
    for (odf, pos, node) in &moons {
        let node = *node;
        let mut sim = new_sim();
        let at = |dx: i32| Vec3 { x: pos.x + Fixed::from_int(dx), y: pos.y, z: pos.z };
        let h = (Fixed::ONE, Fixed::ZERO);
        let f = sim.spawn_def(miner, PlayerId(1), at(300), h).unwrap();
        sim.spawn_def(station, PlayerId(1), at(700), h).unwrap();
        let out = sim.step(&[Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Mine { units: vec![f], node }) }]);
        let mut rejected = out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. }));
        let mut delivered = false;
        for _ in 0..20_000 {
            for e in sim.step(&[]).events {
                rejected |= matches!(e, SimEvent::Rejected { .. });
                delivered |= matches!(e, SimEvent::Delivered { player, .. } if player == PlayerId(1));
            }
            if delivered { break; }
        }
        println!("{odf}: node {node:?} rejected {rejected} delivered {delivered} freighter at {:?}", sim.unit(f).map(|u| u.pos));
        if rejected || !delivered { failed.push(odf.clone()); }
    }
    assert!(failed.is_empty(), "no delivery from: {failed:?}");
}
