//! Load check for the alpha testbed mod (TOOL-20, docs/ALPHA_TESTBED.md).
//! Read-only, no window.
//!
//! `cargo run -p sta_content --example testbed_check -- <root> <mod> [--profile native] [--strict]`
//!
//! `<root>` holds `Data/Mods/<mod>` (a folder or a `.sta`); no game install
//! is needed for a standalone mod. Prints the detected profile, the resolved
//! switches, the content load warnings, the playable races and their start
//! loadouts, every unit whose model is not found and every ODF sound name
//! (fire, select, acknowledge, attack, ambient) that resolves to nothing, and
//! every built unit and HUD command without a button picture
//! (`sprites/gui_global.spr`). Fails on a model that does not load or whose
//! rendered bounds differ from its SOD (turned or scaled against what the sim
//! uses) and on a GUI sprite whose texture is missing. `--profile` fails when
//! another profile is detected; `--strict` also fails on any warning, missing
//! button picture or missing model.

use std::path::Path;

use sta_content::features::Profile;
use sta_content::profile::FEATURE_NAMES;
use sta_content::race::{self, Races};
use sta_content::{ContentDb, OdfIndex};
use sta_vfs::Vfs;

/// Command button sprites the plain HUD asks for (`ui/hud/model.rs` `button`).
const COMMAND_SPRITES: [&str; 19] = [
    "b_move", "b_attack", "b_stop", "b_build", "b_rally", "b_form", "b_recrew", "b_back", "b_harvest", "b_colonize",
    "b_transport", "b_capture", "b_repair", "b_strade", "b_etrade", "b_decom", "b_cancel", "b_research", "b_evolve",
];

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

    // Each model must load the way the renderer loads it, and its rendered
    // shape must match the SOD the sim takes its size and points from.
    let mut seen = std::collections::BTreeSet::new();
    for u in db.units() {
        if u.model.is_empty() || !seen.insert(u.model.to_ascii_lowercase()) {
            continue;
        }
        let Some((path, fmt)) = sta_assets::find_model(&vfs, &u.model) else { continue };
        let shown = match sta_assets::load_model(&vfs, &path, fmt) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("FAIL model {path}: {e}");
                problems += 1;
                continue;
            }
        };
        let sod = sta_assets::find_sod(&vfs, &u.model)
            .and_then(|p| sta_assets::load_model(&vfs, &p, sta_assets::ModelFormat::Sod).ok());
        let fmt3 = |b: Option<([f32; 3], [f32; 3])>| match b {
            Some((lo, hi)) => format!("x {:.1}..{:.1} y {:.1}..{:.1} z {:.1}..{:.1}", lo[0], hi[0], lo[1], hi[1], lo[2], hi[2]),
            None => "no mesh".into(),
        };
        let (shown_b, sod_b) = (shown.bounds(), sod.as_ref().and_then(|s| s.bounds()));
        println!("model {path}: shown {} | sod {}", fmt3(shown_b), fmt3(sod_b));
        // A rendered model turned or scaled against its SOD draws a ship flying
        // tail first, or bigger or smaller than it collides and fires.
        if let (Some((alo, ahi)), Some((blo, bhi))) = (shown_b, sod_b) {
            let size = (0..3).map(|a| (bhi[a] - blo[a]).max(ahi[a] - alo[a])).fold(1.0f32, f32::max);
            let off = (0..3).map(|a| (alo[a] - blo[a]).abs().max((ahi[a] - bhi[a]).abs())).fold(0.0f32, f32::max);
            if off > 0.05 * size {
                eprintln!("FAIL model {path}: rendered bounds differ from its SOD by {off:.1} (size {size:.1}): turned or scaled?");
                problems += 1;
            }
        }
    }

    // Every build button and command button needs a picture in the GUI sprite
    // tables, else the HUD shows its text label.
    let gui = sta_assets::a2fmt::spr::SpriteTable::load("sprites/gui_global.spr", |name| {
        vfs.read(&format!("sprites/{}", name.trim_start_matches("sprites/"))).and_then(Result::ok)
    });
    let mut no_icon = 0;
    for name in COMMAND_SPRITES {
        if gui.sprite(name).is_none() {
            println!("no button picture: {name}");
            no_icon += 1;
        }
    }
    let built: std::collections::HashSet<String> =
        db.units().flat_map(|u| u.build.list.iter().map(|r| r.odf.to_ascii_lowercase().trim_end_matches(".odf").to_owned())).collect();
    for u in db.units().filter(|u| built.contains(&u.odf.to_ascii_lowercase())) {
        let stem = |s: &str| {
            let b = s.rsplit(['/', '\\']).next().unwrap_or(s);
            b.rsplit_once('.').map_or(b, |(a, _)| a).to_owned()
        };
        if !u.model.is_empty() && gui.sprite(&format!("b_{}", stem(&u.model))).is_none() {
            println!("no button picture: b_{} (unit model)", stem(&u.model));
            no_icon += 1;
        }
    }
    for sprite in &gui.sprites {
        if sta_assets::find_texture(&vfs, &sprite.texture).is_none() {
            eprintln!("FAIL sprite {} names missing texture {}", sprite.name, sprite.texture);
            problems += 1;
        }
    }
    println!("button pictures: {} sprites, {no_icon} missing", gui.sprites.len());

    // Every sound an ODF names (weapon fire, unit select/acknowledge/attack/ambient,
    // explosion: its `fireball` ODF's `explSound`, read since #1224)
    // must resolve to a file or an event, as the client's sound bank finds them.
    let bank = sta_assets::audio::SoundBank::load(&vfs);
    let mut sounds: Vec<(String, String, Option<String>)> = Vec::new();
    for w in db.weapons() {
        if let Some(s) = &w.fire_sound {
            sounds.push((w.odf.clone(), s.clone(), None));
        }
    }
    for u in db.units() {
        let race = (!u.race.is_empty()).then(|| u.race.clone());
        for s in u.sounds.select.iter().chain(&u.sounds.ack_move).chain(&u.sounds.ack_attack).chain(&u.sounds.ambient).chain(&u.sounds.explode) {
            sounds.push((u.odf.clone(), s.clone(), race.clone()));
        }
    }
    let mut silent = 0;
    for (odf, name, race) in &sounds {
        if bank.resolve(&vfs, name, race.as_deref(), 0).is_none() {
            println!("missing sound: {name} ({odf})");
            silent += 1;
        }
    }
    println!("sounds: {} references, {silent} missing", sounds.len());
    missing += silent;

    // Every map's sky: a prefix needs its six faces (`<prefix>0..5`), a `.sod` its model.
    let mut maps: Vec<String> = vfs.list("bzn").into_iter().filter(|p| p.to_ascii_lowercase().ends_with(".bzn")).collect();
    maps.sort();
    for p in &maps {
        let Some(Ok(bytes)) = vfs.read(p) else { continue };
        let Ok(map) = sta_bzn::parse(&bytes) else {
            println!("unreadable map: {p}");
            missing += 1;
            continue;
        };
        let sky = map.background.trim();
        if sky.is_empty() {
            continue;
        }
        let ok = if sky.to_ascii_lowercase().ends_with(".sod") {
            sta_assets::find_model(&vfs, sky).is_some()
        } else {
            (0..6).all(|i| sta_assets::find_texture(&vfs, &format!("{sky}{i}")).is_some())
                || (1..=6).all(|i| sta_assets::find_texture(&vfs, &format!("{sky}.{i}")).is_some())
        };
        if !ok {
            println!("missing sky: {sky} ({p})");
            missing += 1;
        }
    }
    println!("maps: {} checked for sky", maps.len());

    let all: Vec<_> = idx_warn.iter().chain(&warnings).chain(&race_warn).collect();
    for w in &all {
        println!("warning: {}: {}", w.file, w.message);
    }
    println!("{} warnings, {missing} missing models or sounds", all.len());
    if strict && (!all.is_empty() || missing > 0 || no_icon > 0) {
        problems += 1;
    }
    if problems > 0 {
        eprintln!("FAIL ({problems})");
        std::process::exit(1);
    }
    println!("OK");
}
