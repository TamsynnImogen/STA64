//! Unit voices belong to the unit's own race (the "Federation ships have Borg voices"
//! regression), checked against the installs' own data: a start loadout's units are mostly
//! the chosen race's (`dominant_race`), a ship's `eventSelect` event has sounds tagged for
//! that race in `events.dat`, and no two races share a voice file. Install-gated:
//! `STA64_TEST_INSTALL` is a Roots install (base Roots, Fleet Ops 4.0 and Classic are checked).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use sta_assets::audio::SoundBank;
use sta_content::{ContentDb, OdfIndex, Races, dominant_race};
use sta_vfs::Vfs;

/// `federation` and `unitedfederation` name one race (as `events.dat` spells them).
fn norm(s: &str) -> String {
    let s = s.to_ascii_lowercase();
    let s = s.strip_prefix("united").unwrap_or(&s);
    s.strip_suffix("empire").unwrap_or(s).to_owned()
}

#[test]
fn dominant_race_ignores_shared_helpers_and_empty_races() {
    assert_eq!(dominant_race(["borg", "species8472", "unitedfederation", "unitedfederation", "", "unitedfederation"]), Some("unitedfederation"));
    assert_eq!(dominant_race(["borg", "klingon"]), Some("borg"), "a tie goes to the first");
    assert_eq!(dominant_race(["", " "]), None);
    assert_eq!(dominant_race(["Borg", "borg", "x"]), Some("Borg"));
}

#[test]
fn every_playable_race_speaks_with_its_own_voices() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
    for m in [None, Some("Fleet Ops 4.0"), Some("STA2 Classic Mod")] {
        let vfs = match Vfs::mount_install(Path::new(&root), m) {
            Ok(v) => v,
            Err(_) => continue, // not installed here
        };
        let label = m.unwrap_or("base Roots");
        let (index, _) = OdfIndex::build(&vfs);
        let (db, _) = ContentDb::load(&vfs, &index).unwrap();
        let (races, _) = Races::load(&vfs, &index);
        let bank = SoundBank::load(&vfs);
        let mut owned: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for r in races.playable() {
            // The player's race, as its start units say it.
            let loadout = races.loadout(&db, &r.name);
            let unit_races: Vec<&str> = loadout.units.iter().filter_map(|k| db.unit(*k)).map(|d| d.race.as_str()).collect();
            let dominant = dominant_race(unit_races.iter().copied()).unwrap_or_default();
            assert!(
                norm(dominant) == norm(&r.name),
                "{label}: race {} starts with {unit_races:?}, whose dominant race is {dominant:?}",
                r.name
            );
            // Its ships' select events: the resolver picks a sound tagged for this race.
            let events: BTreeSet<&str> = db.units().filter(|d| norm(&d.race) == norm(&r.name)).flat_map(|d| d.sounds.select.iter().map(String::as_str)).collect();
            let mut files = BTreeSet::new();
            let mut checked = 0;
            for ev in events {
                let Some(event) = bank.events.get(ev) else { continue };
                let own: Vec<&str> = event.sounds.iter().filter(|s| s.race.as_deref().is_some_and(|t| norm(t) == norm(&r.name))).map(|s| s.file.as_str()).collect();
                if own.is_empty() {
                    continue; // the event has no sound of this race: it falls back to the shared ones
                }
                for variant in 0..own.len().max(1) {
                    let pick = bank.events.pick(ev, Some(dominant), variant).unwrap();
                    assert!(own.contains(&pick.file.as_str()), "{label}: {} {ev} variant {variant} resolved {:?}, not one of its own {own:?}", r.name, pick.file);
                }
                files.extend(own.iter().map(|f| f.to_ascii_lowercase()));
                checked += 1;
            }
            eprintln!("{label}: {} {} select events with own sounds, {} files", r.name, checked, files.len());
            owned.insert(r.name.clone(), files);
        }
        // Where two races' data for one event are disjoint, they resolve to different files.
        // (A few stock events list one file for two races, e.g. Classic's `bursen.wav`; that is
        // the data's own choice and is reported, not failed.)
        let events: BTreeSet<&str> = bank.events.iter().map(|e| e.name.as_str()).collect();
        let (mut compared, mut shared_by_data) = (0, 0);
        for ev in events {
            let sounds = &bank.events.get(ev).unwrap().sounds;
            for (i, a) in races.playable().iter().enumerate() {
                for b in &races.playable()[i + 1..] {
                    if norm(&a.name) == norm(&b.name) {
                        continue;
                    }
                    let tagged = |r: &str| sounds.iter().filter(|s| s.race.as_deref().is_some_and(|t| norm(t) == norm(r))).map(|s| s.file.to_ascii_lowercase()).collect::<BTreeSet<_>>();
                    let (fa, fb) = (tagged(&a.name), tagged(&b.name));
                    if fa.is_empty() || fb.is_empty() {
                        continue;
                    }
                    if !fa.is_disjoint(&fb) {
                        shared_by_data += 1;
                        continue;
                    }
                    let pa = bank.events.pick(ev, Some(&a.name), 0).unwrap();
                    let pb = bank.events.pick(ev, Some(&b.name), 0).unwrap();
                    assert_ne!(pa.file.to_ascii_lowercase(), pb.file.to_ascii_lowercase(), "{label}: {} and {} resolve {ev} to one file", a.name, b.name);
                    compared += 1;
                }
            }
        }
        eprintln!("{label}: {compared} event/race pairs resolve to different files, {shared_by_data} share a file in the data itself");
        assert!(compared > 0);
        assert!(owned.values().any(|f| !f.is_empty()), "{label}: no race has its own select sounds");
    }
}
