//! Install-gated (STA64_TEST_INSTALL + STA64_TEST_MOD, empty for none, and/or
//! STA64_TEST_DIR; skipped when unset): convert every `bzn/*.bzn` of the install to a
//! [`MapDef`] against the content database loaded from the same install.
//!
//! Required of every map the BZN reader parses:
//!
//! - every object of `sta_bzn` is accounted for (`Counts::total` equals the
//!   BZN's object count, and the map holds exactly what the counts say);
//! - a multiplayer map (`mission_type` `mp-*`) has its announced start
//!   locations, one per seat, all inside the bounds.
//!
//! Prints the tallies (`--nocapture`). Run it against each install and mod.

use std::collections::BTreeMap;
use std::path::Path;

use sta_content::map::Conversion;
use sta_content::{ContentDb, MapDef, OdfIndex};
use sta_vfs::Vfs;

#[test]
fn every_map_converts_with_matching_counts_and_starts() {
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

fn check(label: &str, vfs: &Vfs) {
    let (index, _) = OdfIndex::build(vfs);
    let (db, _) = ContentDb::load(vfs, &index).unwrap();
    let mut t = Tally::default();
    let mut problems = Vec::new();
    for path in vfs.list("bzn").into_iter().filter(|p| p.to_ascii_lowercase().ends_with(".bzn")) {
        t.files += 1;
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
            t.unreadable += 1;
            continue;
        }
        t.parsed += 1;
        let Conversion { map, warnings, counts } = MapDef::convert_bzn_in(&bzn, &db, vfs, &index);
        t.objects += bzn.objects.len();
        t.placed += counts.objects;
        t.regions += counts.regions;
        t.planets += counts.planets;
        t.lights += counts.lights;
        t.props += counts.props;
        t.skipped += counts.skipped;
        t.starts += map.starts.len();
        t.warnings += warnings.len();
        for w in &warnings {
            // The reasons, with the counts and map names taken out.
            let why = w.message.split_once(": ").map_or(w.message.as_str(), |(_, r)| r);
            let why = why.split_once("object(s): ").map_or(why, |(_, r)| r);
            *t.reasons.entry(why.to_owned()).or_default() += 1;
            if why.contains("no such ODF") || why.contains("not a unit definition") {
                *t.unknown.entry(w.file.clone()).or_default() += 1;
            }
        }
        if counts.total() != bzn.objects.len() {
            problems.push(format!("{path}: {} objects in the BZN, {} accounted for", bzn.objects.len(), counts.total()));
        }
        if (map.objects.len(), map.regions.len(), map.scenery.planets.len(), map.scenery.lights.len())
            != (counts.objects, counts.regions, counts.planets, counts.lights)
        {
            problems.push(format!("{path}: the map does not hold what the counts say"));
        }
        if bzn.mission_type.to_ascii_lowercase().starts_with("mp") {
            t.mp += 1;
            let (lo, hi) = map.bounds;
            let inside = map.starts.iter().all(|s| lo.x <= s.pos.x && s.pos.x <= hi.x && lo.z <= s.pos.z && s.pos.z <= hi.z);
            let seats: Vec<u8> = map.starts.iter().map(|s| s.slot).collect();
            if map.starts.is_empty() {
                problems.push(format!("{path}: multiplayer map without start positions"));
            } else {
                t.mp_with_starts += 1;
            }
            if !inside {
                problems.push(format!("{path}: a start is outside the bounds"));
            }
            if seats.windows(2).any(|w| w[0] >= w[1]) {
                problems.push(format!("{path}: start slots {seats:?} are not ascending"));
            }
            if map.starts.len() != bzn.start_count.max(0) as usize {
                t.mp_short += 1;
            }
        }
    }
    eprintln!("== {label}");
    eprintln!("{} bzn files: {} converted, {} in a layout the reader does not know", t.files, t.parsed, t.unreadable);
    eprintln!(
        "{} objects in the BZNs: {} placed, {} regions, {} planets, {} lights, {} props not carried, {} skipped",
        t.objects, t.placed, t.regions, t.planets, t.lights, t.props, t.skipped
    );
    eprintln!("{} multiplayer maps, {} with starts ({} with fewer than announced); {} starts in all", t.mp, t.mp_with_starts, t.mp_short, t.starts);
    eprintln!("{} warnings; by reason:", t.warnings);
    for (r, n) in &t.reasons {
        eprintln!("  {n:5}  {r}");
    }
    let mut unknown: Vec<_> = t.unknown.iter().collect();
    unknown.sort_by_key(|e| std::cmp::Reverse(*e.1));
    eprintln!("ODFs not placed: {}", unknown.iter().take(12).map(|(k, n)| format!("{k} x{n}")).collect::<Vec<_>>().join(", "));
    for p in problems.iter().take(20) {
        eprintln!("PROBLEM {p}");
    }
    // Armada I's maps are not in the layout `sta_bzn` reads: nothing to check there.
    assert!(t.parsed > 0 || t.unreadable == t.files, "no map converted");
    assert!(problems.is_empty(), "{} problems, first: {:?}", problems.len(), problems.first());
}

#[derive(Default)]
struct Tally {
    files: usize,
    parsed: usize,
    unreadable: usize,
    objects: usize,
    placed: usize,
    regions: usize,
    planets: usize,
    lights: usize,
    props: usize,
    skipped: usize,
    starts: usize,
    mp: usize,
    mp_with_starts: usize,
    mp_short: usize,
    warnings: usize,
    reasons: BTreeMap<String, usize>,
    unknown: BTreeMap<String, usize>,
}
