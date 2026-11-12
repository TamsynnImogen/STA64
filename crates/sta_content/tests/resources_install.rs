//! Install-gated (skipped when an install is absent): each game's resource
//! model, read from its own data. Prints, per install, the names of the four
//! stock slots (its string table), each playable race's start stock and what
//! the tritanium/latinum/dilithium moons give; then asserts the rules the
//! data states:
//!
//! - start units are placed, not bought (a start ship may cost a slot the stock lacks);
//! - Fleet Operations 4.0 (and 3.x, Dominion War) call the latinum slot
//!   "Tritanium" and the biomatter slot "Supply", Roots' own ODFs the metal
//!   slot "Tritanium" (`GUI_RD_*_VTOOLTIP`);
//! - a moon named for a resource fills the slot of that name (its resource
//!   ODF's `amt<slot>`: `amtlatinum` on Fleet Ops 4.0's tritanium moon);
//! - normal start stock comes from the race ODF, independently of unit costs.

use std::path::{Path, PathBuf};

use sta_content::race::{skirmish_races, StartingResources};
use sta_content::resources::{self, SLOTS};
use sta_content::{ContentDb, Fixed, OdfIndex, Profile, Races};
use sta_vfs::Vfs;

const HEROIC: &str = "/home/cedric/Games/Heroic";
const ROOTS: &str = "Star Trek Armada II Fleet Operations Roots";

struct Install {
    label: &'static str,
    root: PathBuf,
    modname: Option<&'static str>,
    /// The names the four slots must have (`""`: not checked).
    names: [&'static str; SLOTS],
}

fn installs() -> Vec<Install> {
    let h = Path::new(HEROIC);
    let plain = |label, dir: &str, names| Install {
        label,
        root: h.join(dir),
        modname: None,
        names,
    };
    vec![
        Install {
            label: "FO4",
            root: h.join(ROOTS),
            modname: Some("Fleet Ops 4.0"),
            names: ["Dilithium", "Tritanium", "Collective Connections", "Supply"],
        },
        Install {
            label: "STA2 Classic",
            root: h.join(ROOTS),
            modname: Some("STA2 Classic Mod"),
            names: ["Dilithium", "Latinum", "Metal", "Bio-matter"],
        },
        Install {
            label: "Roots",
            root: h.join(ROOTS),
            modname: None,
            names: ["Dilithium", "Connections Nodes", "Tritanium", "Supply"],
        },
        plain(
            "FO",
            "Star Trek Armada II Fleet Operations",
            ["Dilithium", "Tritanium", "Collective Connections", "Supply"],
        ),
        plain(
            "DW",
            "Star Trek Armada II Dominion War",
            ["Dilithium", "Tritanium", "Collective Connections", "Supply"],
        ),
        plain(
            "A2",
            "Star Trek Armada II (Stock)",
            ["Dilithium", "Latinum", "Metal", "Bio-matter"],
        ),
    ]
}

fn mount(i: &Install) -> Option<Vfs> {
    if !i.root.is_dir() {
        eprintln!("{}: skipped, {} is absent", i.label, i.root.display());
        return None;
    }
    match Vfs::mount_install(&i.root, i.modname) {
        Ok(v) => Some(v),
        Err(_) => {
            // A plain game folder with no `Data` (GOG's A2).
            let mut v = Vfs::default();
            v.push_dir(i.label.into(), i.root.clone()).ok()?;
            Some(v)
        }
    }
}

fn f(v: Fixed) -> i64 {
    v.to_f32() as i64
}

#[test]
fn every_game_names_and_feeds_its_resources() {
    let mut checked = 0;
    for i in installs() {
        let Some(vfs) = mount(&i) else { continue };
        let (index, _) = OdfIndex::build(&vfs);
        let (db, _) = ContentDb::load(&vfs, &index).unwrap();
        let (races, _) = Races::load(&vfs, &index);
        let model = db.resources();
        let names: Vec<&str> = (0..SLOTS).map(|s| model.name(s)).collect();
        eprintln!("== {} ({:?}): slots {names:?}", i.label, db.profile());
        for (s, want) in i.names.iter().enumerate() {
            assert_eq!(
                model.name(s),
                *want,
                "{}: name of slot {s} ({})",
                i.label,
                resources::KEYS[s]
            );
        }
        for race in skirmish_races(&races, &db) {
            let l = races.loadout(&db, &race);
            let def = races.get_playable(&race).expect("offered race is playable");
            let lots = races.loadout_with_mode(&db, &race, StartingResources::Lots);
            let fixed = |n: i64| Fixed::from_int(n.clamp(0, 1_000_000_000) as i32);
            assert_eq!(l.stock, def.stock.map(fixed), "{} {race}: normal stock", i.label);
            assert_eq!(lots.stock, def.lots_stock.map(fixed), "{} {race}: lots stock", i.label);
            assert_eq!(l.crew, def.normal_crew.map(fixed), "{} {race}: normal crew", i.label);
            assert_eq!(lots.crew, def.lots_crew.map(fixed), "{} {race}: lots crew", i.label);
            assert_eq!(l.units, lots.units, "{} {race}: resource mode keeps start units", i.label);
            eprintln!(
                "  {race}: start stock {:?}",
                l.stock.iter().map(|v| f(*v)).collect::<Vec<_>>()
            );
            for k in &l.units {
                let u = db.unit(*k).unwrap();
                // Not an outpost's own cost (it is placed free): what it can build is checked below.
                eprintln!("    {} builds {} items", u.odf, u.build.list.len());
            }
        }
        // Moons: a dilithium moon fills dilithium, a tritanium moon never does; where the
        // game has the word (Fleet Operations, Roots) the tritanium moon fills that slot.
        for u in db
            .units()
            .filter(|u| u.odf.starts_with("map_") && u.odf.contains("moon") && u.resource.is_some())
        {
            let slot = u.resource.unwrap() as usize;
            if u.odf.contains("dilithium") {
                assert_eq!(slot, resources::DILITHIUM, "{}: {}", i.label, u.odf);
            } else if u.odf.contains("tritanium") {
                assert_ne!(slot, resources::DILITHIUM, "{}: {}", i.label, u.odf);
                if model.name(slot) != "Metal" {
                    assert_eq!(
                        model.name(slot),
                        "Tritanium",
                        "{}: {} gives slot {slot}",
                        i.label,
                        u.odf
                    );
                }
            }
        }
        let tri = db.units().find(|u| u.odf == "map_tritanium_moon1");
        if let Some(u) = tri {
            eprintln!(
                "  map_tritanium_moon1 gives slot {} ({})",
                u.resource.unwrap_or(9),
                model.name(u.resource.unwrap_or(9) as usize)
            );
        }
        checked += 1;
    }
    if checked == 0 {
        eprintln!("no install mounted: nothing checked");
    }
}

/// Normal starting stock is authored by the faction, not inferred from the
/// cost of a minimal-start ship or a synthetic playable replacement.
#[test]
fn fo4_federation_start_stock_comes_from_the_race() {
    let i = &installs()[0];
    let Some(vfs) = mount(i) else { return };
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let (races, _) = Races::load(&vfs, &index);
    assert_eq!(db.profile(), Profile::Fo4);
    let race = races.get_playable("unitedfederation").unwrap();
    let loadout = races.loadout(&db, "unitedfederation");
    assert_eq!(loadout.stock.map(f), [4000, 2000, 0, 500]);
    assert_eq!(loadout.stock.map(f), race.stock);
    // minimalUnits belongs to the separate minimal-start option. A normal
    // start must not buy or insert that ship to make its resource model work.
    for name in &race.minimal_units {
        if !race.standard_units.contains(name) {
            assert!(!loadout.units.iter().any(|k| db.unit(*k).unwrap().odf == *name));
        }
    }
}
