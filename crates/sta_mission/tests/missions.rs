//! Install-gated headless run of every installed mission on its own map
//! (LUA-04). Read-only: nothing is written to the install.
//!
//! Gate (skipped when none is set), the variables of the CON tests:
//!
//! - `STA64_TEST_INSTALL` + `STA64_TEST_MOD` (empty mod mounts `Data/` alone);
//! - `STA64_TEST_DIR`, a bare game folder.
//!
//! Every map of the install is parsed and converted, its mission is resolved
//! the way the game does (`sta_mission::resolve`: label, file name, description),
//! and when a Lua script is found the map is built through the same seams as
//! `sta_bench`'s corpus (`GameSetup`, `Sim::from_setup_with_starts`) and the
//! script runs 300 ticks as the seat-0 controller of an otherwise idle game.
//! Scripts that no map names (helpers, tutorials) are listed separately.
//!
//! Outcome per mission script:
//!
//! - **run**: loaded, 300 ticks, no error, no stub touched;
//! - **stubbed**: as run, but the script touched API the runtime still stubs
//!   (the names are tallied in the report, most-used first);
//! - **failed**: a load or callback error, a panic, or the sim did not reach
//!   tick 300. These are the runtime's open gaps, listed as `PROBLEM` lines
//!   (LUA-03 and LUA-05 work), so they do not fail the test by default;
//!   `STA64_MISSIONS_STRICT=1` makes any failure fail it. A panic, or a script
//!   stopped by the LUA-09 limits (instruction budget, memory), always fails:
//!   the limits must stay above what any installed mission needs.
//!
//! Run with `--nocapture` to see the report.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::Arc;

use sta_content::{ContentDb, MapDef, OdfIndex, Races};
use sta_core::{PlayerId, TeamId};
use sta_mission::api::MissionConfig;
use sta_mission::runtime::MissionRuntime;
use sta_mission::{Labels, Resolution};
use sta_sim::command::Command;
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, WorldSource};
use sta_vfs::Vfs;

const TICKS: u64 = 300;
const SEED: u64 = 0x5EED_C0DE;

#[test]
fn every_installed_mission_runs_headless() {
    if let (Ok(root), Ok(m)) = (std::env::var("STA64_TEST_INSTALL"), std::env::var("STA64_TEST_MOD")) {
        let vfs = Vfs::mount_install(Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
        check(&format!("{root} + {m}"), Arc::new(vfs));
    }
    if let Ok(dir) = std::env::var("STA64_TEST_DIR") {
        let mut vfs = Vfs::default();
        vfs.push_dir("root".into(), dir.clone().into()).unwrap();
        check(&dir, Arc::new(vfs));
    }
}

#[derive(Default)]
struct Outcome {
    errors: Vec<String>,
    stubs: Vec<(String, u32)>,
    callbacks: usize,
    /// Most VM instructions one host entry used (the LUA-09 budget is
    /// `sandbox::INSTRUCTION_BUDGET` per entry).
    peak: u64,
}

fn check(label: &str, vfs_arc: Arc<Vfs>) {
    let vfs: &Vfs = &vfs_arc;
    let (index, _) = OdfIndex::build(vfs);
    let (db, _) = ContentDb::load(vfs, &index).unwrap();
    let db = Arc::new(db);
    let (races, _) = Races::load(vfs, &index);
    let labels = Labels::load(vfs);

    let (mut files, mut unreadable, mut legacy, mut none) = (0, 0, 0, 0);
    let mut legacy_names = BTreeSet::new();
    let mut run = Vec::new();
    let mut stubbed = Vec::new();
    let mut failed = Vec::new();
    let mut seen_scripts: BTreeSet<String> = BTreeSet::new();
    let mut stub_tally: BTreeMap<String, u32> = BTreeMap::new();

    for path in vfs.list("bzn").into_iter().filter(|p| p.to_ascii_lowercase().ends_with(".bzn")) {
        files += 1;
        let Some(Ok(data)) = vfs.read(&path) else {
            unreadable += 1;
            continue;
        };
        let Ok(bzn) = sta_bzn::parse(&data) else {
            unreadable += 1;
            continue;
        };
        if bzn.mission_type.is_empty() && bzn.objects.is_empty() {
            unreadable += 1;
            continue;
        }
        let script = match sta_mission::resolve(vfs, &labels, &path, &bzn) {
            Resolution::Lua { path, .. } => path,
            Resolution::LegacyDll { files } => {
                legacy += 1;
                legacy_names.extend(files);
                continue;
            }
            Resolution::None => {
                none += 1;
                continue;
            }
        };
        // Several maps may share a script; each map is its own run.
        seen_scripts.insert(script.to_ascii_lowercase());
        let mut map = MapDef::convert_bzn_in(&bzn, &db, vfs, &index).map;
        let stem = path.rsplit('/').next().unwrap_or(&path).trim_end_matches(".bzn").to_owned();
        map.name = stem.clone();
        let name = format!("{stem} -> {script}");
        let out = catch_unwind(AssertUnwindSafe(|| run_one(&Arc::new(map), &db, &races, &vfs_arc, &script)))
            .unwrap_or_else(|p| Outcome { errors: vec![format!("panicked: {}", panic_text(&p))], ..Default::default() });
        for (k, n) in &out.stubs {
            *stub_tally.entry(k.clone()).or_default() += n;
        }
        if !out.errors.is_empty() {
            failed.push((name, out));
        } else if !out.stubs.is_empty() {
            stubbed.push((name, out));
        } else {
            run.push((name, out));
        }
    }

    let all_scripts: BTreeSet<String> = vfs
        .list("missions")
        .into_iter()
        .filter(|p| p.to_ascii_lowercase().ends_with(".lua"))
        .map(|p| p.to_ascii_lowercase())
        .collect();
    let unbound: Vec<&String> = all_scripts.difference(&seen_scripts).collect();

    eprintln!("== {label}");
    eprintln!("{files} bzn files: {unreadable} in a layout the reader does not know");
    eprintln!(
        "maps by mission: {} Lua, {legacy} legacy DLL only ({} distinct files), {none} none",
        run.len() + stubbed.len() + failed.len(),
        legacy_names.len()
    );
    eprintln!(
        "scripts run: {}  stubbed: {}  failed: {}  (each = one map's mission, {TICKS} ticks)",
        run.len(),
        stubbed.len(),
        failed.len()
    );
    let peak = run.iter().chain(&stubbed).chain(&failed).map(|(_, o)| o.peak).max().unwrap_or(0);
    eprintln!(
        "peak instructions in any one entry: {peak} of {} allowed",
        sta_mission::sandbox::INSTRUCTION_BUDGET
    );
    eprintln!("mission scripts in missions/: {}, run by some map: {}, not named by any map: {}", all_scripts.len(), seen_scripts.len(), unbound.len());
    for (n, o) in &run {
        eprintln!("  RUN     {n} ({} callbacks)", o.callbacks);
    }
    for (n, o) in &stubbed {
        let top: Vec<String> = o.stubs.iter().take(4).map(|(k, c)| format!("{k}x{c}")).collect();
        eprintln!("  STUBBED {n}: {} stub name(s), e.g. {}", o.stubs.len(), top.join(", "));
    }
    for (n, o) in &failed {
        eprintln!("PROBLEM {n}: {}", o.errors.first().cloned().unwrap_or_default());
    }
    let mut tally: Vec<_> = stub_tally.into_iter().collect();
    tally.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    eprintln!("stub names touched, most-used first (top 25 of {}):", tally.len());
    for (k, n) in tally.iter().take(25) {
        eprintln!("  {n:6}  {k}");
    }
    let strict = std::env::var_os("STA64_MISSIONS_STRICT").is_some();
    let hard: Vec<&str> = failed
        .iter()
        .filter(|(_, o)| {
            o.errors.iter().any(|e| e.starts_with("panicked") || e.contains("instruction budget") || e.contains("memory error"))
        })
        .map(|(n, _)| n.as_str())
        .collect();
    assert!(hard.is_empty(), "{label}: panic or sandbox limit hit in {hard:?}");
    assert!(!strict || failed.is_empty(), "{label}: {} mission(s) failed, see PROBLEM lines", failed.len());
}

fn run_one(map: &Arc<MapDef>, db: &Arc<ContentDb>, races: &Races, vfs: &Arc<Vfs>, script: &str) -> Outcome {
    let setup = setup_for(map, db, races);
    let (mut sim, _) = Sim::from_setup_with_starts(&setup, races);
    let cfg = MissionConfig {
        seat: Some(PlayerId(0)),
        content: Some(db.clone()),
        labels: sim.labels.iter().map(|(id, l)| (id, l.clone())).collect(),
        view: Some((*sim.view()).clone()),
        ..Default::default()
    };
    let mut rt = MissionRuntime::from_vfs_with(vfs.clone(), script, SEED, cfg);
    let mut events = Vec::new();
    let mut tick = 0;
    for _ in 0..TICKS {
        let mut commands: Vec<Command> = Vec::new();
        rt.think(&sim.view(), &events, &mut commands);
        let out = sim.step(&commands);
        tick = out.view.tick;
        events = out.events;
    }
    let mut errors: Vec<String> = rt.errors().to_vec();
    if tick != TICKS {
        errors.push(format!("stopped at tick {tick}"));
    }
    Outcome { errors, stubs: rt.unsupported(), callbacks: rt.active_callbacks(), peak: rt.instruction_peak() }
}

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

fn panic_text(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_default()
}
