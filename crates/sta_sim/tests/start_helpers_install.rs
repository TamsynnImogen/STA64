//! Install-gated: a skirmish start on an installed game shows the player its
//! starting units and none of the engine's `hidden = 1` start helpers (Roots'
//! "AI Pod", "Species 8472 Starting Pod", "Federation Faction";
//! docs/START_HELPERS.md).
//!
//! `STA64_TEST_INSTALL` names a Fleet Operations Roots root. With
//! `STA64_TEST_MOD` unset the test runs base Roots, "Fleet Ops 4.0" and
//! "STA2 Classic Mod" (each mod when it mounts); with it set, that mod alone (empty:
//! `Data/` alone). Without `STA64_TEST_INSTALL` the test checks nothing.
//!
//! For each playable race: seat 0 (human) and seat 1 (AI) play it on
//! `fake::two_bases()`, the start settles for five seconds, and then
//! - every authored `standardUnits` entry that resolves to a unit is placed,
//!   in order, for both seats (scripts and later lifecycle rules see them);
//! - every placed unit whose ODF has `hidden = 1` is neither selectable nor
//!   in any player's visible set;
//! - every other placed unit is selectable and visible to its owner.

use std::path::PathBuf;
use std::sync::Arc;

use sta_content::race::skirmish_races;
use sta_content::{ContentDb, OdfIndex, Races};
use sta_core::{PlayerId, TeamId};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::UnitFlags;
use sta_sim::world::EntityKind;
use sta_sim::{Sim, WorldSource};
use sta_vfs::Vfs;

fn hidden(db: &ContentDb, key: sta_core::ContentKey) -> bool {
    db.unit(key).is_some_and(EntityKind::hidden)
}

fn installs() -> Vec<(String, PathBuf, Option<String>)> {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return Vec::new() };
    let root = PathBuf::from(root);
    match std::env::var("STA64_TEST_MOD") {
        Ok(m) => vec![(format!("mod {m:?}"), root, Some(m).filter(|m| !m.is_empty()))],
        Err(_) => [None, Some("Fleet Ops 4.0"), Some("STA2 Classic Mod")]
            .into_iter()
            .map(|m| (m.unwrap_or("base").to_owned(), root.clone(), m.map(str::to_owned)))
            .collect(),
    }
}

#[test]
fn installed_starts_show_their_units_and_hide_their_helpers() {
    let mut checked = 0;
    let mut failures: Vec<String> = Vec::new();
    for (label, root, active_mod) in installs() {
        let vfs = match Vfs::mount_install(&root, active_mod.as_deref()) {
            Ok(vfs) => vfs,
            // A mod of the default set this install lacks; a named one must mount.
            Err(e) if std::env::var("STA64_TEST_MOD").is_err() && active_mod.is_some() => {
                eprintln!("{label}: skipped, does not mount: {e}");
                continue;
            }
            Err(e) => panic!("{label}: {e}"),
        };
        let (index, _) = OdfIndex::build(&vfs);
        let (db, _) = ContentDb::load(&vfs, &index).unwrap();
        let db = Arc::new(db);
        let (races, _) = Races::load(&vfs, &index);
        let mut helpers = 0;
        for race in skirmish_races(&races, &db) {
            let seat = |id: u8, control: Control| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: race.clone(), control };
            let setup = GameSetup {
                seed: 7,
                map: Arc::new(sta_content::fake::two_bases()),
                content: db.clone(),
                players: vec![seat(0, Control::Human), seat(1, Control::Ai("test".into()))],
                options: GameOptions::default(),
            };
            let (mut sim, _) = Sim::from_setup_with_starts(&setup, &races);
            // What is placed is the authored list at tick 0; the helpers then run
            // their lifecycle (SIM-28, tests/start_effects_install.rs).
            let placed_view = sim.view();
            for _ in 0..5 * sta_core::TICK_HZ {
                sim.step(&[]);
            }
            let view = sim.view();
            let expected: Vec<_> = races.loadout(&db, &race).units;
            for p in [PlayerId(0), PlayerId(1)] {
                let placed: Vec<_> = placed_view.units.iter().filter(|u| u.owner == p).collect();
                let defs: Vec<_> = placed.iter().map(|u| u.def).collect();
                if defs != expected {
                    let n = |ks: &[sta_core::ContentKey]| ks.iter().map(|&k| db.name(k).unwrap_or("?").to_owned()).collect::<Vec<_>>();
                    failures.push(format!("{label} {race} seat {}: placed {:?}, authored {:?}", p.0, n(&defs), n(&expected)));
                }
                let mine = view.visible_to(p);
                // Units the lifecycle has since removed (expired pods) are not checked again.
                for u in placed.iter().filter(|u| view.units.iter().any(|now| now.id == u.id)) {
                    let name = db.name(u.def).unwrap_or("?");
                    let selectable = u.flags.contains(UnitFlags::SELECTABLE);
                    let seen = mine.iter().any(|v| v.id == u.id);
                    if hidden(&db, u.def) {
                        helpers += 1;
                        let by_anyone = view.visibility.iter().any(|(_, ids)| ids.contains(&u.id));
                        if selectable || by_anyone {
                            failures.push(format!("{label} {race} seat {}: hidden {name} selectable={selectable} visible={by_anyone}", p.0));
                        }
                    } else if !selectable || !seen {
                        failures.push(format!("{label} {race} seat {}: start unit {name} selectable={selectable} visible={seen}", p.0));
                    }
                }
            }
            checked += 1;
        }
        eprintln!("{label}: {helpers} hidden start helpers placed and hidden");
    }
    if checked == 0 {
        eprintln!("STA64_TEST_INSTALL unset or no playable race: nothing checked");
        return;
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
