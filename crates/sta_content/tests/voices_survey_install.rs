//! Which races can speak in an install, from its own data: for each playable race, the units
//! of that race that name a select / acknowledge event (`eventSelect`, `eventAcknowledge`,
//! `eventAttack`), whether the mounted `events.dat` knows the event, whether it has a line
//! tagged for the race, and whether the file resolves in the mounted layers.
//!
//! Install-gated: `STA64_TEST_INSTALLS` is a `:`-separated list of install roots; every mod
//! folder under `Data/Mods` is mounted as well as the base. Skips when unset.

use std::collections::BTreeSet;
use std::path::Path;

use sta_assets::audio::SoundBank;
use sta_content::{ContentDb, OdfIndex, Races};
use sta_vfs::Vfs;

fn norm(s: &str) -> String {
    let s = s.to_ascii_lowercase();
    let s = s.strip_prefix("united").unwrap_or(&s);
    s.strip_suffix("empire").unwrap_or(s).to_owned()
}

#[test]
fn every_playable_race_has_voice_files_that_resolve() {
    let Ok(roots) = std::env::var("STA64_TEST_INSTALLS") else { return };
    let mut failures = Vec::new();
    for root in roots.split(':').filter(|r| !r.is_empty()) {
        let root = Path::new(root);
        let data = if root.join("Data").is_dir() { root.join("Data") } else { root.to_path_buf() };
        let mut mods: Vec<Option<String>> = vec![None];
        if let Ok(rd) = std::fs::read_dir(data.join("Mods")) {
            let mut names: Vec<String> = rd.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            mods.extend(names.into_iter().map(Some));
        }
        for m in mods {
            let Ok(vfs) = Vfs::mount_install(root, m.as_deref()) else { continue };
            let label = format!("{} / {}", root.file_name().unwrap().to_string_lossy(), m.as_deref().unwrap_or("(base)"));
            let (index, _) = OdfIndex::build(&vfs);
            let Ok((db, _)) = ContentDb::load(&vfs, &index) else { continue };
            let (races, _) = Races::load(&vfs, &index);
            let bank = SoundBank::load(&vfs);
            // A resolver still mounted on the base game (what the match holds after the front
            // end's mod switch): the same units, looked up in the base events and files.
            let base = Vfs::mount_install(root, None).ok().filter(|_| m.is_some());
            let stale = base.as_ref().map(|v| (SoundBank::load(v), v));
            for r in sta_content::race::skirmish_races(&races, &db) {
                let (mut units, mut with_key, mut known, mut tagged, mut resolved) = (0, 0, 0, 0, 0);
                let mut files = BTreeSet::new();
                for d in db.units().filter(|d| norm(&d.race) == norm(&r)) {
                    units += 1;
                    let names: Vec<&String> = d.sounds.select.iter().chain(&d.sounds.ack_move).chain(&d.sounds.ack_attack).collect();
                    if names.is_empty() {
                        continue;
                    }
                    with_key += 1;
                    for n in names {
                        let Some(ev) = bank.events.get(n) else { continue };
                        known += 1;
                        if ev.sounds.iter().any(|s| s.race.as_deref().is_some_and(|t| norm(t) == norm(&r))) {
                            tagged += 1;
                        }
                        if let Some(res) = bank.resolve(&vfs, n, Some(&r), 0) {
                            resolved += 1;
                            files.insert(res.path);
                        }
                    }
                }
                if let Some((sb, sv)) = &stale {
                    let mut n = 0;
                    let mut with = 0;
                    for d in db.units().filter(|d| norm(&d.race) == norm(&r)) {
                        for nm in d.sounds.select.iter().chain(&d.sounds.ack_move).chain(&d.sounds.ack_attack) {
                            with += 1;
                            n += usize::from(sb.resolve(sv, nm, Some(&r), 0).is_some());
                        }
                    }
                    eprintln!("| STALE base mount, {label} | {r} | {with} event refs | {n} resolve |");
                }
                eprintln!("| {label} | {r} | {units} units | {with_key} with voice keys | {known} event refs known | {tagged} tagged for race | {resolved} resolve | {} files |", files.len());
                if with_key > 0 && resolved == 0 {
                    failures.push(format!("{label}: {r} has {with_key} voiced units but nothing resolves"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
