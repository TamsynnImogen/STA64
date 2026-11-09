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
}

#[test]
fn the_installed_tech_tree_loads_and_the_start_station_can_build() {
    if let Ok(root) = std::env::var("STA64_TEST_INSTALL") {
        let m = std::env::var("STA64_TEST_MOD").ok();
        let vfs = Vfs::mount_install(Path::new(&root), m.as_deref()).unwrap();
        check(m.as_deref().unwrap_or("Data"), &vfs);
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("STA64_TEST_TT_DIRS").map(|d| std::env::split_paths(&d).collect()).unwrap_or_default();
    dirs.extend(std::env::var_os("STA64_TEST_DIR").map(PathBuf::from));
    for dir in dirs {
        let mut vfs = Vfs::default();
        vfs.push_dir("root".into(), dir.clone()).unwrap();
        check(&dir.display().to_string(), &vfs);
    }
}
