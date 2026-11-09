//! Install-gated map corpus (TOOL-05, from `maps_report`): every map of an
//! install loads, ticks 300 times and ends on the same digest when built and
//! run a second time.
//!
//! Gate (skipped when none is set), the variables of the CON tests:
//!
//! - `STA64_TEST_INSTALL` + `STA64_TEST_MOD` (empty mod mounts `Data/` alone);
//! - `STA64_TEST_DIR`, a bare game folder.
//!
//! Each map goes through the seams only: `sta_bzn::parse`,
//! `MapDef::convert_bzn_in`, `ContentDb::load`, a [`GameSetup`] and
//! `Sim::from_setup_with_starts`, the rule the game itself uses to place each
//! seat's starting forces on its start position (the race's start objects and
//! stock from its ODF, or the documented default). A map is
//! a failure when its BZN is unreadable, when it panics, does not reach tick
//! 300, or when two fresh builds end on different digests. Armada I's
//! save-game layout (no objects, no mission type) is counted as unreadable,
//! not as a failure, as in `map_install`.
//!
//! Prints a tally per install (`--nocapture`). The two builds of a map run on
//! the same thread, the maps on a few worker threads; `STA64_TEST_JOBS`
//! changes their number (default 4). Use `--release` for the big mods.

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use sta_content::{ContentDb, MapDef, OdfIndex, Races};
use sta_core::{PlayerId, TeamId};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, WorldSource};
use sta_vfs::Vfs;

const TICKS: u64 = 300;
const SEED: u64 = 0x5EED_C0DE;

#[test]
fn every_map_loads_ticks_and_repeats() {
    if let (Ok(root), Ok(m)) = (std::env::var("STA64_TEST_INSTALL"), std::env::var("STA64_TEST_MOD")) {
        // An empty mod mounts the base `Data` alone.
        let vfs = Vfs::mount_install(Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
        check(&format!("{root} + {m}"), &vfs);
    }
    if let Ok(dir) = std::env::var("STA64_TEST_DIR") {
        let mut vfs = Vfs::default();
        vfs.push_dir("root".into(), dir.clone().into()).unwrap();
        check(&dir, &vfs);
    }
}

/// What one map came to.
#[derive(Default)]
struct Outcome {
    ticked: bool,
    repeated: bool,
    /// Entities at tick 0 and at the end.
    units: (usize, usize),
    /// Map objects the sim skipped.
    skipped: usize,
    failure: Option<String>,
}

fn check(label: &str, vfs: &Vfs) {
    let (index, _) = OdfIndex::build(vfs);
    let (db, _) = ContentDb::load(vfs, &index).unwrap();
    let db = Arc::new(db);
    let (races, _) = Races::load(vfs, &index);
    let races = Arc::new(races);

    let mut problems = Vec::new();
    let (mut files, mut unreadable) = (0, 0);
    let mut maps: Vec<(String, Arc<MapDef>)> = Vec::new();
    for path in vfs.list("bzn").into_iter().filter(|p| p.to_ascii_lowercase().ends_with(".bzn")) {
        files += 1;
        let Some(Ok(data)) = vfs.read(&path) else {
            problems.push(format!("{path}: unreadable"));
            continue;
        };
        let bzn = match sta_bzn::parse(&data) {
            Ok(b) => b,
            Err(e) => {
                problems.push(format!("{path}: sta_bzn: {e}"));
                continue;
            }
        };
        if bzn.mission_type.is_empty() && bzn.objects.is_empty() {
            // Armada I's save-game layout: not one this reader knows.
            unreadable += 1;
            continue;
        }
        let mut map = MapDef::convert_bzn_in(&bzn, &db, vfs, &index).map;
        map.name = path.rsplit('/').next().unwrap_or(&path).trim_end_matches(".bzn").to_owned();
        maps.push((path, Arc::new(map)));
    }

    let next = AtomicUsize::new(0);
    let jobs = std::env::var("STA64_TEST_JOBS").ok().and_then(|j| j.parse().ok()).unwrap_or(4usize).max(1);
    let mut results: Vec<(usize, Outcome)> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..jobs)
            .map(|_| {
                s.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some((_, map)) = maps.get(i) else { break };
                        done.push((i, run_map(map, &db, &races)));
                    }
                    done
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().unwrap()).collect()
    });
    results.sort_by_key(|r| r.0);

    let (mut ticked, mut repeated, mut empty) = (0, 0, 0);
    let (mut skipped, mut start_units) = (0, 0);
    for (i, o) in &results {
        let path = &maps[*i].0;
        ticked += usize::from(o.ticked);
        repeated += usize::from(o.repeated);
        empty += usize::from(o.units.0 == 0);
        skipped += o.skipped;
        start_units += o.units.0;
        if let Some(f) = &o.failure {
            problems.push(format!("{path}: {f}"));
        }
    }
    eprintln!("== {label}");
    eprintln!("{files} bzn files: {} loaded, {unreadable} in a layout the reader does not know", maps.len());
    eprintln!("{} maps loaded, {ticked} ticked {TICKS}, {repeated} repeated with the same digest", maps.len());
    eprintln!("{start_units} units at tick 0 in all, {skipped} map objects skipped by the sim, {empty} maps empty at tick 0");
    let mut reasons: BTreeMap<&str, usize> = BTreeMap::new();
    for p in &problems {
        *reasons.entry(p.split_once(": ").map_or(p.as_str(), |(_, r)| r)).or_default() += 1;
    }
    for p in &problems {
        eprintln!("PROBLEM {p}");
    }
    for (r, n) in &reasons {
        eprintln!("  {n:4}  {r}");
    }
    assert!(problems.is_empty(), "{label}: {} problem(s), see PROBLEM lines", problems.len());
    assert!(!maps.is_empty(), "{label}: no map loaded");
}

/// Build the map twice, tick each 300 times, compare the digests.
fn run_map(map: &Arc<MapDef>, db: &Arc<ContentDb>, races: &Races) -> Outcome {
    let mut out = Outcome::default();
    let setup = setup_for(map, db, races);
    let run = || {
        catch_unwind(AssertUnwindSafe(|| {
            let (mut sim, skipped) = build(&setup, races);
            let at_start = sim.view().units.len();
            let mut tick = 0;
            for _ in 0..TICKS {
                tick = sim.step(&[]).view.tick;
            }
            (tick, at_start, sim.view().units.len(), skipped, sim.digest())
        }))
    };
    let first = match run() {
        Ok(r) => r,
        Err(p) => {
            out.failure = Some(format!("panicked: {}", panic_text(&p)));
            return out;
        }
    };
    out.units = (first.1, first.2);
    out.skipped = first.3;
    if first.0 != TICKS {
        out.failure = Some(format!("stopped at tick {}", first.0));
        return out;
    }
    out.ticked = true;
    match run() {
        Ok(second) if second.4 == first.4 => out.repeated = true,
        Ok(second) => out.failure = Some(format!("digest {:016x} then {:016x}", first.4, second.4)),
        Err(p) => out.failure = Some(format!("panicked on the second run: {}", panic_text(&p))),
    }
    out
}

fn panic_text(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned())).unwrap_or_default()
}

/// One seat per start position, or per owner a mission's objects name, and at
/// least two; each seat its own team and, in turn, one of the content's races.
fn setup_for(map: &Arc<MapDef>, db: &Arc<ContentDb>, races: &Races) -> GameSetup {
    let seats = map
        .starts
        .iter()
        .map(|s| s.slot as usize + 1)
        .chain(map.objects.iter().filter_map(|o| o.owner).map(|o| o as usize + 1))
        .max()
        .unwrap_or(0)
        .clamp(2, 16);
    let races = sta_content::race::skirmish_races(races, db);
    let players = (0..seats)
        .map(|i| PlayerSetup {
            id: PlayerId(i as u8),
            team: TeamId(i as u8),
            race: races.get(i % races.len().max(1)).cloned().unwrap_or_default(),
            control: Control::Idle,
        })
        .collect();
    GameSetup { seed: SEED, map: Arc::clone(map), content: Arc::clone(db), players, options: GameOptions::default() }
}

/// The skirmish start rule (`Sim::from_setup_with_starts`): each seat's race
/// loadout on its start position. Returns the sim and the objects it skipped.
fn build(setup: &GameSetup, races: &Races) -> (Sim, usize) {
    let (sim, warnings) = Sim::from_setup_with_starts(setup, races);
    (sim, warnings.len())
}
