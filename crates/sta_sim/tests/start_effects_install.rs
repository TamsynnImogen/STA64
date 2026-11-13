//! SIM-28, install-gated: the start helpers and effects every playable race
//! places run their lifecycle (docs: `start_effects.rs`).
//!
//! `STA64_TEST_INSTALL` names a game root; `STA64_TEST_MOD` the mod (empty: `Data/`
//! alone; unset: base, "Fleet Ops 4.0" and "STA2 Classic Mod" where they mount).
//! For each playable race, seat 0 (human) and seat 1 (AI) start on
//! `fake::two_bases()` and the sim runs 640 s. Then, for every seat:
//! - every `standardUnits` entry that resolves to a unit raised no
//!   "unresolved or unsupported" warning; what the sim does not run of one's
//!   lifecycle is a warning naming its key (printed);
//! - no entity with an expiry (`autoExpireTime` / `lifeTimer`) remains, and
//!   no start pod whose instant weapon fires for its owner remains;
//! - a race whose list holds a pod that spawns ships (`8472start`) has them.

use std::path::PathBuf;
use std::sync::Arc;

use sta_content::race::skirmish_races;
use sta_content::{ContentDb, OdfIndex, Races};
use sta_core::{PlayerId, TICK_HZ, TeamId};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, WorldSource};
use sta_vfs::Vfs;

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
fn installed_start_helpers_run_their_lifecycle() {
    let mut checked = 0;
    let mut failures: Vec<String> = Vec::new();
    for (label, root, active_mod) in installs() {
        let vfs = match Vfs::mount_install(&root, active_mod.as_deref()) {
            Ok(vfs) => vfs,
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
        for race in skirmish_races(&races, &db) {
            let seat = |id: u8, control: Control| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: race.clone(), control };
            let setup = GameSetup {
                seed: 7,
                map: Arc::new(sta_content::fake::two_bases()),
                content: db.clone(),
                players: vec![seat(0, Control::Human), seat(1, Control::Ai("test".into()))],
                options: GameOptions::default(),
            };
            let (mut sim, warnings) = Sim::from_setup_with_starts(&setup, &races);
            let def = races.get_playable(&race).or_else(|| races.get(&race)).unwrap();
            for odf in &def.standard_units {
                let resolves = db.key(odf).is_some_and(|k| db.unit(k).is_some());
                let unresolved = warnings.iter().any(|w| w.message.contains(&format!("entry for {odf}.odf is unresolved")));
                if resolves == unresolved {
                    failures.push(format!("{label} {race}: {odf} resolves={resolves} but unresolved-warning={unresolved}"));
                }
            }
            for w in warnings.iter().filter(|w| w.message.contains("standardUnits entry") && !w.message.contains("unresolved")) {
                eprintln!("{label} {race}: unsupported: {}", w.message);
            }
            for _ in 0..640 * TICK_HZ {
                sim.step(&[]);
            }
            for u in sim.view().units.iter() {
                let Some(d) = db.unit(u.def) else { continue };
                if d.hidden && d.life.is_some() {
                    failures.push(format!("{label} {race}: {} (owner {}) outlived its expiry", d.odf, u.owner.0));
                }
                let kind_idx = if u.owner == PlayerId(1) { 1 } else { 0 };
                let fires = d.hardpoints.iter().filter_map(|hp| db.weapon(hp.weapon.key)).filter_map(|w| w.replace.as_ref())
                    .any(|r| r.instant[kind_idx] && r.required_officers.is_none());
                if d.hidden && fires {
                    failures.push(format!("{label} {race}: start pod {} (owner {}) was never replaced", d.odf, u.owner.0));
                }
            }
            if def.standard_units.iter().any(|o| o.eq_ignore_ascii_case("8472start")) {
                let ships = |p: u8| sim.view().units.iter().filter(|u| u.owner == PlayerId(p))
                    .filter(|u| db.unit(u.def).is_some_and(|d| ["8472corvette", "8472active", "8472passive"].contains(&d.odf.as_str()))).count();
                for p in [0u8, 1] {
                    if ships(p) < 4 {
                        failures.push(format!("{label} {race}: seat {p} has {} of the four 8472start ships", ships(p)));
                    }
                }
            }
            checked += 1;
        }
    }
    if checked == 0 {
        eprintln!("STA64_TEST_INSTALL unset or no playable race: nothing checked");
        return;
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
