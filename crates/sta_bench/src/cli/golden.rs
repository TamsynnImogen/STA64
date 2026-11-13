//! `sta-bench golden` (TOOL-12, D5): the content identity and the sim digests
//! of an install's skirmish starts, as one machine-readable report that can be
//! recorded and checked later.
//!
//! ```text
//! sta-bench golden --install <root> [--mod <name>] [map...] [--ticks N] [--seed S]
//!                  [--json <file|->] [--record <file>] [--against <file>]
//! ```
//!
//! Each map (default: every readable `bzn/*.bzn`) is converted on the install's
//! content and started as a skirmish (`Sim::from_setup_with_starts`: one seat
//! per start position, at least two, the content's playable races in turn,
//! idle seats), then stepped `--ticks` times (default 300) twice; the two runs
//! must end on the same digest. The report names the profile and the content
//! hash, and the digests at tick 0 and at the end. `--record` writes it,
//! `--against` compares a fresh run with a recorded one and exits 1 on any
//! difference. Recorded goldens derive from the install: keep them outside the
//! repository (docs/TESTBENCH.md, "CI"); nothing here pins an install-derived
//! value.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sta_content::{ContentDb, MapDef, OdfIndex, Races};
use sta_core::{PlayerId, TeamId};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, WorldSource};
use sta_vfs::Vfs;

use super::{Fail, Opts, Outcome};

const DEFAULT_TICKS: u64 = 300;
const DEFAULT_SEED: u64 = 0x5EED_C0DE;

/// The report: what `--json` prints and `--record` writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Golden {
    /// The active mod, if any (the install path is machine specific and not recorded).
    pub mod_name: Option<String>,
    /// Profile name (`fo3`, `fo4`, `sta2classic`, ...), D4.
    pub profile: String,
    /// `ContentDb::hash`, 16 hex digits.
    pub content_hash: String,
    pub seed: u64,
    pub ticks: u64,
    pub maps: Vec<MapGolden>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapGolden {
    pub map: String,
    pub seats: usize,
    pub units_start: usize,
    pub units_end: usize,
    /// Map objects the sim skipped at start.
    pub skipped: usize,
    pub digest_start: String,
    pub digest_end: String,
    /// The second run ended on the same digest.
    pub repeatable: bool,
    /// A panic or other failure, if any.
    pub error: Option<String>,
}

impl Golden {
    /// The report as JSON (strict: double-quoted keys, no trailing commas), one map per line.
    pub fn to_json(&self) -> String {
        use crate::run::json::string as q;
        let opt = |o: &Option<String>| o.as_deref().map_or("null".to_owned(), q);
        let maps: Vec<String> = self
            .maps
            .iter()
            .map(|m| {
                format!(
                    "    {{\"map\": {}, \"seats\": {}, \"units_start\": {}, \"units_end\": {}, \"skipped\": {}, \"digest_start\": {}, \"digest_end\": {}, \"repeatable\": {}, \"error\": {}}}",
                    q(&m.map),
                    m.seats,
                    m.units_start,
                    m.units_end,
                    m.skipped,
                    q(&m.digest_start),
                    q(&m.digest_end),
                    m.repeatable,
                    opt(&m.error)
                )
            })
            .collect();
        format!(
            "{{\n  \"mod_name\": {},\n  \"profile\": {},\n  \"content_hash\": {},\n  \"seed\": {},\n  \"ticks\": {},\n  \"maps\": [\n{}\n  ]\n}}\n",
            opt(&self.mod_name),
            q(&self.profile),
            q(&self.content_hash),
            self.seed,
            self.ticks,
            maps.join(",\n")
        )
    }
}

impl MapGolden {
    fn failed(map: &str, error: String) -> MapGolden {
        MapGolden { map: map.to_owned(), seats: 0, units_start: 0, units_end: 0, skipped: 0, digest_start: String::new(), digest_end: String::new(), repeatable: false, error: Some(error) }
    }
}

fn hex(v: u64) -> String {
    format!("{v:016x}")
}

fn setup_for(map: &Arc<MapDef>, db: &Arc<ContentDb>, races: &Races, seed: u64) -> GameSetup {
    let seats = map
        .starts
        .iter()
        .map(|s| s.slot as usize + 1)
        .chain(map.objects.iter().filter_map(|o| o.owner).map(|o| o as usize + 1))
        .max()
        .unwrap_or(0)
        .clamp(2, 16);
    let names = sta_content::race::skirmish_races(races, db);
    let players = (0..seats)
        .map(|i| PlayerSetup { id: PlayerId(i as u8), team: TeamId(i as u8), race: names.get(i % names.len().max(1)).cloned().unwrap_or_default(), control: Control::Idle })
        .collect();
    GameSetup { seed, map: Arc::clone(map), content: Arc::clone(db), players, options: GameOptions::default() }
}

/// One run: (seats, units at start, units at the end, skipped, digest at start, digest at end).
fn run_once(setup: &GameSetup, races: &Races, ticks: u64) -> (usize, usize, usize, usize, u64, u64) {
    let (mut sim, warnings) = Sim::from_setup_with_starts(setup, races);
    let start = (sim.view().units.len(), sim.digest());
    for _ in 0..ticks {
        sim.step(&[]);
    }
    (setup.players.len(), start.0, sim.view().units.len(), warnings.len(), start.1, sim.digest())
}

fn one_map(name: &str, map: &Arc<MapDef>, db: &Arc<ContentDb>, races: &Races, seed: u64, ticks: u64) -> MapGolden {
    let go = || catch_unwind(AssertUnwindSafe(|| run_once(&setup_for(map, db, races, seed), races, ticks)));
    let panic_text = |p: Box<dyn std::any::Any + Send>| {
        p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned())).unwrap_or_else(|| "panic".into())
    };
    let first = match go() {
        Ok(r) => r,
        Err(p) => return MapGolden::failed(name, format!("panicked: {}", panic_text(p))),
    };
    let (repeatable, error) = match go() {
        Ok(second) => (second.5 == first.5 && second.4 == first.4, None),
        Err(p) => (false, Some(format!("panicked on the second run: {}", panic_text(p)))),
    };
    MapGolden {
        map: name.to_owned(),
        seats: first.0,
        units_start: first.1,
        units_end: first.2,
        skipped: first.3,
        digest_start: hex(first.4),
        digest_end: hex(first.5),
        repeatable,
        error,
    }
}

/// Run the install's maps and build the report.
pub fn collect(vfs: &Vfs, mod_name: Option<&str>, only: &[String], ticks: u64, seed: u64, jobs: usize) -> Result<Golden, String> {
    let (index, _) = OdfIndex::build(vfs);
    let (db, _) = ContentDb::load(vfs, &index).map_err(|e| format!("content: {e}"))?;
    let db = Arc::new(db);
    let (races, _) = Races::load(vfs, &index);
    let races = Arc::new(races);

    let mut maps: Vec<(String, Arc<MapDef>)> = Vec::new();
    let mut failed: Vec<MapGolden> = Vec::new();
    let mut paths: Vec<String> = vfs.list("bzn").into_iter().filter(|p| p.to_ascii_lowercase().ends_with(".bzn")).collect();
    paths.sort();
    for path in paths {
        let name = path.rsplit('/').next().unwrap_or(&path).trim_end_matches(".bzn").trim_end_matches(".BZN").to_owned();
        if !only.is_empty() && !only.iter().any(|m| m.eq_ignore_ascii_case(&name)) {
            continue;
        }
        let Some(Ok(bytes)) = vfs.read(&path) else {
            failed.push(MapGolden::failed(&name, "unreadable".into()));
            continue;
        };
        let bzn = match sta_bzn::parse(&bytes) {
            Ok(b) => b,
            Err(e) => {
                failed.push(MapGolden::failed(&name, format!("sta_bzn: {e}")));
                continue;
            }
        };
        if bzn.mission_type.is_empty() && bzn.objects.is_empty() {
            continue; // a layout this reader does not know (Armada I saved games)
        }
        let mut map = MapDef::convert_bzn_in(&bzn, &db, vfs, &index).map;
        map.name = name.clone();
        maps.push((name, Arc::new(map)));
    }
    for want in only {
        if !maps.iter().any(|m| m.0.eq_ignore_ascii_case(want)) && !failed.iter().any(|f| f.map.eq_ignore_ascii_case(want)) {
            return Err(format!("no map named `{want}` in the install"));
        }
    }

    let next = AtomicUsize::new(0);
    let mut done: Vec<(usize, MapGolden)> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..jobs.max(1))
            .map(|_| {
                s.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some((name, map)) = maps.get(i) else { break };
                        out.push((i, one_map(name, map, &db, &races, seed, ticks)));
                    }
                    out
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().expect("a golden worker panicked outside a map run")).collect()
    });
    done.sort_by_key(|d| d.0);
    let mut all: Vec<MapGolden> = failed;
    all.extend(done.into_iter().map(|d| d.1));
    all.sort_by_key(|a| a.map.to_ascii_lowercase());
    Ok(Golden { mod_name: mod_name.map(str::to_owned), profile: db.profile().name().to_owned(), content_hash: hex(db.hash()), seed, ticks, maps: all })
}

/// What differs between a recorded report and a fresh one (empty: they agree).
pub fn differences(recorded: &Golden, now: &Golden) -> Vec<String> {
    let mut out = Vec::new();
    for (what, a, b) in [
        ("profile", recorded.profile.clone(), now.profile.clone()),
        ("content_hash", recorded.content_hash.clone(), now.content_hash.clone()),
        ("seed", recorded.seed.to_string(), now.seed.to_string()),
        ("ticks", recorded.ticks.to_string(), now.ticks.to_string()),
        ("mod", format!("{:?}", recorded.mod_name), format!("{:?}", now.mod_name)),
    ] {
        if a != b {
            out.push(format!("{what}: recorded {a}, now {b}"));
        }
    }
    for r in &recorded.maps {
        match now.maps.iter().find(|m| m.map.eq_ignore_ascii_case(&r.map)) {
            None => out.push(format!("{}: recorded, not run now", r.map)),
            Some(m) if m != r => out.push(format!(
                "{}: recorded {} -> {} ({} units), now {} -> {} ({} units){}",
                r.map,
                r.digest_start,
                r.digest_end,
                r.units_end,
                m.digest_start,
                m.digest_end,
                m.units_end,
                m.error.as_deref().map_or(String::new(), |e| format!(" ({e})"))
            )),
            Some(_) => {}
        }
    }
    for m in &now.maps {
        if !recorded.maps.iter().any(|r| r.map.eq_ignore_ascii_case(&m.map)) {
            out.push(format!("{}: run now, not recorded", m.map));
        }
    }
    out
}

fn summary(g: &Golden) -> String {
    let bad = g.maps.iter().filter(|m| m.error.is_some() || !m.repeatable).count();
    format!("profile {}, content {}, {} maps x {} ticks (seed {:#x}), {} failed or not repeatable", g.profile, g.content_hash, g.maps.len(), g.ticks, g.seed, bad)
}

pub(crate) fn golden(o: &Opts) -> Outcome {
    let Some(root) = &o.install else {
        return Err(Fail::Usage("golden: --install <root> [--mod <name>] [map...]".into()));
    };
    let vfs = Vfs::mount_install(root, o.mod_name.as_deref()).map_err(|e| Fail::Error(format!("mounting {}: {e}", root.display())))?;
    let ticks = o.ticks.unwrap_or(DEFAULT_TICKS);
    let seed = o.seed.unwrap_or(DEFAULT_SEED);
    let now = collect(&vfs, o.mod_name.as_deref(), &o.rest, ticks, seed, o.jobs.unwrap_or(2)).map_err(Fail::Error)?;
    let json = now.to_json();
    let to_stdout = o.json.as_deref().is_some_and(|p| p.as_os_str() == "-");
    match &o.json {
        Some(p) if p.as_os_str() == "-" => print!("{json}"),
        Some(p) => std::fs::write(p, &json).map_err(|e| Fail::Error(format!("{}: {e}", p.display())))?,
        None => {}
    }
    let say = |s: String| {
        if to_stdout {
            eprintln!("{s}");
        } else {
            println!("{s}");
        }
    };
    say(summary(&now));
    let mut code = 0;
    for m in now.maps.iter().filter(|m| m.error.is_some() || !m.repeatable) {
        say(format!("  FAIL {}: {}", m.map, m.error.as_deref().unwrap_or("the second run ended on another digest")));
        code = 1;
    }
    if let Some(p) = &o.against {
        let text = std::fs::read_to_string(p).map_err(|e| Fail::Error(format!("{}: {e}", p.display())))?;
        let recorded: Golden = json5::from_str(&text).map_err(|e| Fail::Error(format!("{}: {e}", p.display())))?;
        let diff = differences(&recorded, &now);
        for d in &diff {
            say(format!("  DIFF {d}"));
        }
        say(if diff.is_empty() { format!("matches {}", p.display()) } else { format!("{} difference(s) from {}", diff.len(), p.display()) });
        if !diff.is_empty() {
            code = 1;
        }
    }
    if let Some(p) = &o.record {
        std::fs::write(p, &json).map_err(|e| Fail::Error(format!("{}: {e}", p.display())))?;
        say(format!("recorded {}", p.display()));
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(digest: &str) -> Golden {
        Golden {
            mod_name: None,
            profile: "fo3".into(),
            content_hash: "00".into(),
            seed: 1,
            ticks: 2,
            maps: vec![MapGolden { map: "A".into(), seats: 2, units_start: 3, units_end: 3, skipped: 0, digest_start: "1".into(), digest_end: digest.into(), repeatable: true, error: None }],
        }
    }

    #[test]
    fn a_report_round_trips_and_differences_name_the_map() {
        let a = g("aa");
        let back: Golden = json5::from_str(&a.to_json()).unwrap();
        assert_eq!(back, a);
        assert!(a.to_json().contains("\"profile\": \"fo3\"") && !a.to_json().contains(",\n  ]"), "strict JSON");
        assert!(differences(&a, &back).is_empty());
        let d = differences(&a, &g("bb"));
        assert_eq!(d.len(), 1);
        assert!(d[0].starts_with("A: recorded 1 -> aa"), "{d:?}");
        let mut other = g("aa");
        other.profile = "sta2classic".into();
        other.maps.clear();
        let d = differences(&a, &other);
        assert!(d.iter().any(|x| x.starts_with("profile:")) && d.iter().any(|x| x == "A: recorded, not run now"), "{d:?}");
    }

    /// A map of the fake content runs twice on one digest: the machinery works without an install.
    #[test]
    fn the_fake_content_map_repeats() {
        let db = Arc::new(sta_content::fake::db());
        let map = Arc::new(MapDef::empty());
        let m = one_map("fake", &map, &db, &Races::default(), 7, 20);
        assert!(m.repeatable && m.error.is_none(), "{m:?}");
        assert_eq!(m.digest_start.len(), 16);
    }
}
