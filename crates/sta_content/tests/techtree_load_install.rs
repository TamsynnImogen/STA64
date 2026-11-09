//! Install-gated: `ContentDb::load` finds and loads the skirmish tech tree
//! (`tech1`) of a real install, and the NotListed rule (SIM-09 blocks what the
//! tree does not list) does not block a real game's first build.
//!
//! For each race with a start station (a builder the tree lets a player with
//! nothing own, as the sim does when the map places the station), prints the tree, and its build list with only that
//! station owned. Requires at least one such station with an available option.
//!
//! - STA64_TEST_INSTALL (+ STA64_TEST_MOD): a Fleet Operations / Roots install.
//! - STA64_TEST_DIR: a plain game folder (A1 / A2 retail) mounted as one layer;
//!   STA64_TEST_TT_DIRS also names plain folders, path-separated.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sta_content::{Availability, ContentDb, OdfIndex, Owned};
use sta_vfs::Vfs;

fn check(label: &str, vfs: &Vfs) {
    let (index, _) = OdfIndex::build(vfs);
    let (db, warnings) = ContentDb::load(vfs, &index).unwrap();
    let tree = db.tech_tree().unwrap_or_else(|| panic!("{label}: no tech tree loaded; warnings: {:?}", warnings.iter().filter(|w| w.file == "tech1").collect::<Vec<_>>()));
    eprintln!(
        "{label}: tree {} from {:?}, {} entries, {} capacity lists, {} load warnings in all, {} defs",
        tree.name,
        tree.files(),
        tree.len(),
        tree.capacity_lists().count(),
        warnings.len(),
        db.len()
    );
    let mut races: BTreeSet<&str> = db.units().map(|u| u.race.as_str()).filter(|r| !r.is_empty()).collect();
    races.insert("");
    let mut playable = 0;
    let mut dead_races: Vec<String> = Vec::new();
    for race in races {
        // Start stations: they build something, and the tree needs nothing
        // owned for one (a station is placed by the map, not built).
        let stations: Vec<_> = db
            .units()
            .filter(|u| u.race == race && !u.build.list.is_empty())
            .filter(|u| tree.availability(&u.odf, &Owned::new(race)) == Availability::Available)
            .collect();
        // Many stations are upgrades of one another; print the one with the
        // most available options, and how many there are.
        let rows: Vec<_> = stations
            .iter()
            .map(|st| {
                let owned = Owned::new(race).with(&st.odf, 1);
                (st.odf.as_str(), tree.build_options(&db, &st.odf, &owned))
            })
            .collect();
        let avail = |o: &[sta_content::BuildOption]| o.iter().filter(|o| o.availability.is_available()).count();
        playable += rows.iter().filter(|(_, o)| avail(o) > 0).count();
        if !race.is_empty() && !rows.is_empty() && rows.iter().all(|(_, o)| avail(o) == 0) {
            dead_races.push(race.to_owned());
        }
        // Build options of any start station that the tree does not list at
        // all: the sim blocks these, whatever is owned.
        let unlisted: BTreeSet<&str> =
            rows.iter().flat_map(|(_, o)| o.iter().filter(|o| o.availability == Availability::NotListed).map(|o| o.def.odf.as_str())).collect();
        if !unlisted.is_empty() {
            eprintln!("  {race:>16}: not listed in the tree: {unlisted:?}");
        }
        if let Some((odf, opts)) = rows.iter().max_by_key(|(_, o)| avail(o)) {
            let first: Vec<&str> = opts.iter().filter(|o| o.availability.is_available()).map(|o| o.def.odf.as_str()).take(4).collect();
            let not_listed = opts.iter().filter(|o| o.availability == Availability::NotListed).count();
            eprintln!(
                "  {race:>16}: {} start stations; {odf}: {} options, {} available, {not_listed} not listed; e.g. {first:?}",
                rows.len(),
                opts.len(),
                avail(opts)
            );
        }
    }
    assert!(playable > 0, "{label}: no start station has an available build option");
    // Every race with a start base can build something from it.
    assert!(dead_races.is_empty(), "{label}: races whose start bases have no available build option: {dead_races:?}");
}

/// The NotListed rule (`TechTree::availability_for_builder`): counts what it
/// frees, checks no editor page is offered and, on Fleet Ops 4.0, that the
/// avatar-form main bases and `kli_field_yardP4` build.
fn not_listed_rule(label: &str, vfs: &Vfs, fo4: bool) {
    let (index, _) = OdfIndex::build(vfs);
    let (db, _) = ContentDb::load(vfs, &index).unwrap();
    let tree = db.tech_tree().expect("tech tree");
    // A player owning one of everything: the strictest test that an editor
    // page is never offered (nothing is Locked then).
    let everything = |race: &str| db.units().fold(Owned::new(race), |o, u| o.with(&u.odf, 1));
    let (mut before, mut after) = (BTreeSet::new(), BTreeSet::new());
    let mut freed = BTreeSet::new();
    for b in db.units() {
        let st = everything(&b.race);
        for item in &b.build.list {
            let pair = (b.odf.as_str(), item.odf.as_str());
            if tree.availability(&item.odf, &st) == Availability::NotListed {
                before.insert(pair);
                if tree.availability_for_builder(&db, &item.odf, &b.odf, &st) == Availability::NotListed {
                    after.insert(pair);
                } else {
                    freed.insert(item.odf.as_str());
                    if std::env::var_os("STA64_TEST_VERBOSE").is_some() {
                        let o = db.key(&item.odf).and_then(|k| db.unit(k)).and_then(|u| u.raw.str("originalclass").map(str::to_owned));
                        let slots: Vec<String> = b.raw.keys().filter(|k| k.to_ascii_lowercase().starts_with("builditem") && b.raw.raw(k).is_some_and(|v| v.trim_matches('"').eq_ignore_ascii_case(&item.odf))).flat_map(|k| { let n = &k[9..]; ["availability", "provided", "code"].iter().filter_map(|f| b.raw.raw(&format!("builditem{n}{f}")).map(|v| format!("{f}={v}"))).collect::<Vec<_>>() }).collect();
                        eprintln!("DBG freed {} <- {} originalclass={o:?} slot={slots:?}", item.odf, b.odf);
                    }
                }
            }
            let offered = tree.availability_for_builder(&db, &item.odf, &b.odf, &st).is_available();
            assert!(
                !offered || !(item.odf.to_ascii_lowercase().starts_with("edit_") || b.odf.to_ascii_lowercase().starts_with("edit_")),
                "{label}: editor page offered: {} from {}",
                item.odf,
                b.odf
            );
        }
    }
    // No install defines editor pages as units (the loader never sees them),
    // so the `edit_` guard is belt and braces; say so if one ever appears.
    let editors = db.units().filter(|u| u.odf.to_ascii_lowercase().starts_with("edit_")).count();
    eprintln!("{label}: {editors} edit_ defs in the db");
    let items = |s: &BTreeSet<(&str, &str)>| s.iter().map(|p| p.1).collect::<BTreeSet<_>>().len();
    eprintln!(
        "{label}: NotListed (builder, item) slots blocked: before {} ({} distinct items), after {} ({} distinct); freed items: {freed:?}",
        before.len(),
        items(&before),
        after.len(),
        items(&after)
    );
    if !fo4 {
        return;
    }
    // Each Z base builds from its construction ship with only the ship and
    // the start base (its Y twin) owned.
    for z in ["fed_outpostZ", "rom_starbaseZ", "bor_assi_matrixZ", "dom_cloning_facZ", "kli_kahless_stationZ"] {
        let ships: Vec<_> = db.units().filter(|u| u.odf.to_ascii_lowercase().ends_with("zs") && u.build.list.iter().any(|d| d.odf.eq_ignore_ascii_case(z))).collect();
        assert!(!ships.is_empty(), "{label}: no *ZS ship builds {z}");
        let y = format!("{}Y", &z[..z.len() - 1]);
        let ok = ships.iter().any(|ship| {
            let owned = Owned::new(&ship.race).with(&ship.odf, 1).with(&y, 1);
            tree.availability_for_builder(&db, z, &ship.odf, &owned).is_available()
        });
        assert!(ok, "{label}: {z} is not available from {:?}", ships.iter().map(|s| &s.odf).collect::<Vec<_>>());
        // Without the rule the tree alone omits it.
        assert_eq!(tree.availability(z, &Owned::new(&ships[0].race)), Availability::NotListed, "{label}: {z} is listed after all");
    }
    let yard = db.units().find(|u| u.build.list.iter().any(|d| d.odf.eq_ignore_ascii_case("kli_field_yardP4")));
    let yard = yard.expect("some builder lists kli_field_yardP4");
    eprintln!("{label}: kli_field_yardP4 is built by {}", yard.odf);
    let owned = Owned::new(&yard.race).with(&yard.odf, 1);
    assert!(tree.availability_for_builder(&db, "kli_field_yardP4", &yard.odf, &owned).is_available(), "{label}: kli_field_yardP4");
}

#[test]
fn the_installed_tech_tree_loads_and_the_start_station_can_build() {
    if let Ok(root) = std::env::var("STA64_TEST_INSTALL") {
        let m = std::env::var("STA64_TEST_MOD").ok();
        let vfs = Vfs::mount_install(Path::new(&root), m.as_deref()).unwrap();
        check(m.as_deref().unwrap_or("Data"), &vfs);
        not_listed_rule(m.as_deref().unwrap_or("Data"), &vfs, m.as_deref() == Some("Fleet Ops 4.0"));
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("STA64_TEST_TT_DIRS").map(|d| std::env::split_paths(&d).collect()).unwrap_or_default();
    dirs.extend(std::env::var_os("STA64_TEST_DIR").map(PathBuf::from));
    for dir in dirs {
        let mut vfs = Vfs::default();
        vfs.push_dir("root".into(), dir.clone()).unwrap();
        check(&dir.display().to_string(), &vfs);
        not_listed_rule(&dir.display().to_string(), &vfs, false);
    }
}
