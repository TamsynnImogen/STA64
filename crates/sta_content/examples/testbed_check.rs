//! Load check for the alpha testbed mod (TOOL-20, docs/ALPHA_TESTBED.md).
//! Read-only, no window.
//!
//! `cargo run -p sta_content --example testbed_check -- <root> <mod> [--profile native] [--strict]`
//!
//! `<root>` holds `Data/Mods/<mod>` (a folder or a `.sta`); no game install
//! is needed for a standalone mod. Prints the detected profile, the resolved
//! switches, the content load warnings, the playable races and their start
//! loadouts, and every unit whose model is not found. `--profile` fails when
//! another profile is detected; `--strict` also fails on any warning or
//! missing model.

use std::path::Path;

use sta_content::features::Profile;
use sta_content::profile::FEATURE_NAMES;
use sta_content::race::{self, Races};
use sta_content::{ContentDb, OdfIndex};
use sta_vfs::Vfs;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(root), Some(m)) = (args.first(), args.get(1)) else {
        eprintln!("usage: testbed_check <root> <mod> [--profile <name>] [--strict]");
        std::process::exit(2);
    };
    let strict = args.iter().any(|a| a == "--strict");
    let want = args.iter().position(|a| a == "--profile").and_then(|i| args.get(i + 1));
    let mut problems = 0usize;

    let vfs = match Vfs::mount_install(Path::new(root), Some(m)) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("FAIL mount {m}: {e}");
            std::process::exit(1);
        }
    };
    println!("layers:");
    for l in vfs.layers() {
        println!("  {}", l.root.display());
    }

    let det = Profile::detect_with_source(&vfs);
    println!("profile: {} (step {:?}: {})", det.profile.name(), det.step, det.detail);
    if let Some(w) = want
        && Profile::from_name(w) != Some(det.profile)
    {
        eprintln!("FAIL profile: wanted {w}");
        problems += 1;
    }

    let (index, idx_warn) = OdfIndex::build(&vfs);
    let (db, warnings) = match ContentDb::load(&vfs, &index) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("FAIL content: {e}");
            std::process::exit(1);
        }
    };
    let on: Vec<&str> = FEATURE_NAMES.iter().copied().filter(|n| db.features().get(n) == Some(true)).collect();
    println!("switches on ({}): {}", on.len(), on.join(" "));
    println!("units: {}, weapons: {}", db.units().count(), db.weapons().count());

    let (races, race_warn) = Races::load(&vfs, &index);
    let playable = race::skirmish_races(&races, &db);
    println!("playable races: {}", playable.join(", "));
    if playable.is_empty() {
        eprintln!("FAIL no playable race");
        problems += 1;
    }
    for r in &playable {
        let l = races.loadout(&db, r);
        let names: Vec<&str> = l.units.iter().filter_map(|k| db.name(*k)).collect();
        println!("  {r}: {} ({})", names.join(", "), l.source);
        if l.units.is_empty() {
            eprintln!("FAIL race {r} starts with nothing");
            problems += 1;
        }
    }

    let mut missing = 0;
    for u in db.units() {
        if !u.model.is_empty() && sta_assets::find_model(&vfs, &u.model).is_none() {
            println!("missing model: {} (unit of race {:?})", u.model, u.race);
            missing += 1;
        }
    }

    let all: Vec<_> = idx_warn.iter().chain(&warnings).chain(&race_warn).collect();
    for w in &all {
        println!("warning: {}: {}", w.file, w.message);
    }
    println!("{} warnings, {missing} missing models", all.len());
    if strict && (!all.is_empty() || missing > 0) {
        problems += 1;
    }
    if problems > 0 {
        eprintln!("FAIL ({problems})");
        std::process::exit(1);
    }
    println!("OK");
}
