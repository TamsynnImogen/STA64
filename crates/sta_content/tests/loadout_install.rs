//! Install-gated (skipped when the install is absent): every playable race's
//! skirmish start units (what `Races::loadout` places: the outpost or station,
//! the construction ship, the freighters, the start fleet) must resolve a def
//! and a model (SOD) through the mod chain, the way the renderer finds one
//! (`sta_assets::find_model`). Prints `race -> unit ODF -> model` per install
//! and fails listing every unit without a model.
//!
//! Installs: the Heroic folders of FO4 (with its "Fleet Ops 4.0" mod), FO, DW
//! and A2, plus `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`) when set. An install
//! whose folder is absent, or that does not mount (GOG's plain A2 folder has
//! no `Data`), is skipped.

use std::path::{Path, PathBuf};

use sta_assets::find_model;
use sta_content::race::skirmish_races;
use sta_content::{ContentDb, OdfIndex, RaceDef, Races};
use sta_vfs::Vfs;

const HEROIC: &str = "/home/cedric/Games/Heroic";

/// One start unit of one race, and the model file it resolves to.
struct Start {
    race: String,
    /// The unit's ODF stem, or `(no start units)` for a race that starts with nothing.
    odf: String,
    model: String,
    /// The model's VFS path, `None` when no layer holds it.
    path: Option<String>,
}

/// The start units of every playable race the skirmish offers. A race whose
/// loadout is empty gets one row of its own, which fails like a missing model.
fn starts(vfs: &Vfs, db: &ContentDb, races: &Races) -> Vec<Start> {
    let mut out = Vec::new();
    for race in skirmish_races(races, db) {
        let loadout = races.loadout(db, &race);
        if loadout.units.is_empty() {
            out.push(Start {
                race: race.clone(),
                odf: "(no start units)".into(),
                model: String::new(),
                path: None,
            });
        }
        for k in loadout.units {
            let Some(u) = db.unit(k) else { continue };
            let path = find_model(vfs, &u.model).map(|(p, _)| p);
            out.push(Start {
                race: race.clone(),
                odf: u.odf.clone(),
                model: u.model.clone(),
                path,
            });
        }
    }
    out
}

fn report(label: &str, rows: &[Start]) {
    eprintln!("== {label}");
    for s in rows {
        let found = s.path.as_deref().unwrap_or("MISSING");
        eprintln!("  {:<16} {:<28} {:<24} {found}", s.race, s.odf, s.model);
    }
}

/// The installs to check: `(label, install root, active mod)`.
fn installs() -> Vec<(String, PathBuf, Option<String>)> {
    let mut v: Vec<(String, PathBuf, Option<String>)> = vec![
        (
            "FO4".into(),
            Path::new(HEROIC).join("Star Trek Armada II Fleet Operations Roots"),
            Some("Fleet Ops 4.0".into()),
        ),
        (
            "Roots".into(),
            Path::new(HEROIC).join("Star Trek Armada II Fleet Operations Roots"),
            None,
        ),
        (
            "STA2 Classic".into(),
            Path::new(HEROIC).join("Star Trek Armada II Fleet Operations Roots"),
            Some("STA2 Classic Mod".into()),
        ),
        (
            "FO".into(),
            Path::new(HEROIC).join("Star Trek Armada II Fleet Operations"),
            None,
        ),
        (
            "DW".into(),
            Path::new(HEROIC).join("Star Trek Armada II Dominion War"),
            None,
        ),
        (
            "A2".into(),
            Path::new(HEROIC).join("Star Trek Armada II (Stock)"),
            None,
        ),
    ];
    if let Ok(root) = std::env::var("STA64_TEST_INSTALL") {
        let m = std::env::var("STA64_TEST_MOD")
            .ok()
            .filter(|m| !m.is_empty());
        v.push(("env".into(), PathBuf::from(root), m));
    }
    v
}

#[test]
fn every_playable_start_unit_has_a_model() {
    let mut missing = Vec::new();
    let mut checked = 0;
    for (label, root, active_mod) in installs() {
        if !root.is_dir() {
            eprintln!("{label}: skipped, {} is absent", root.display());
            continue;
        }
        let vfs = match Vfs::mount_install(&root, active_mod.as_deref()) {
            Ok(vfs) => vfs,
            Err(e) => {
                eprintln!("{label}: skipped, does not mount: {e}");
                continue;
            }
        };
        let (index, _) = OdfIndex::build(&vfs);
        let (db, _) = ContentDb::load(&vfs, &index).unwrap();
        let (races, _) = Races::load(&vfs, &index);
        let rows = starts(&vfs, &db, &races);
        report(&label, &rows);
        checked += rows.len();
        missing.extend(rows.iter().filter(|s| s.path.is_none()).map(|s| {
            format!(
                "{label} {}: {} (model {:?}) has no model",
                s.race, s.odf, s.model
            )
        }));
    }
    if checked == 0 {
        eprintln!("no install mounted: nothing checked");
        return;
    }
    eprintln!(
        "{checked} start units checked, {} without a model",
        missing.len()
    );
    assert!(
        missing.is_empty(),
        "start units without a model:\n  {}",
        missing.join("\n  ")
    );
}

/// The check itself must catch a missing model: the fake content's start units
/// over an empty VFS are all reported, and the same units over a layer that
/// holds their SODs are not.
#[test]
fn a_start_unit_without_a_model_is_reported() {
    let db = sta_content::fake::db();
    let def = RaceDef {
        stem: "fx".into(),
        name: "fx".into(),
        slot: Some(1),
        interface: None,
        interface_sprites: None,
        single_player_music: None,
        multiplayer_music: None,
        standard_units: vec!["fx_yard".into(), "fx_station".into()],
        minimal_units: Vec::new(),
        stock: [4000, 2000, 0, 500],
    };
    let races = Races::from_defs(vec![def]);

    let empty = Vfs::default();
    let bad: Vec<String> = starts(&empty, &db, &races)
        .into_iter()
        .filter(|s| s.path.is_none())
        .map(|s| s.odf)
        .collect();
    assert_eq!(bad, ["fx_yard", "fx_station"]);

    let dir = std::env::temp_dir().join(format!("sta64-con22-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("sod")).unwrap();
    std::fs::write(dir.join("sod/fx_yard.sod"), b"").unwrap();
    std::fs::write(dir.join("sod/fx_station.sod"), b"").unwrap();
    let mut vfs = Vfs::default();
    vfs.push_dir("test".into(), dir.clone()).unwrap();
    let rows = starts(&vfs, &db, &races);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter().all(|s| s.path.is_some()),
        "a model in the layer is not reported"
    );
}

/// The rule per game, read from the data (race ODFs through the mod chain, so
/// the mod's own `federation.odf` wins over Roots'):
///
/// - playable modern races are the referenced race ODFs with a nonnegative
///   `instantactionslot` (including zero); A1 uses its legacy interface fallback.
///   Start validity is checked separately below; it never selects eligibility. Each has a
///   start (DW's `fed`, `dom` and `borg` are real races of their own, with a
///   slot, a race key and start units, not aliases of `federation`/`dominion`);
/// - a start is the race's `standardunits*` that are craft or stations, plus the
///   first `minimalunits` ship when it is the race's own and has a build list.
///   Start units are placed, not paid for, so their cost is no condition;
/// - Fleet Operations 4.0, Federation: `minimalunits1 = "fconst.odf"` (the mod's
///   own `fconst`, race `unitedfederation`) is placed beside the outpost, and
///   a structure on its build list is payable from the `normal*` stock;
/// - a start holds a freighter only where `standardunits*` names one: Roots'
///   `fminer`/`kminer`/... (class `freighter`); A2, FO, FO4 and DW name none.
#[test]
fn start_rule_matches_the_data_per_install() {
    let mut checked = 0;
    for (label, root, active_mod) in installs() {
        let Ok(vfs) = Vfs::mount_install(&root, active_mod.as_deref()) else {
            eprintln!("{label}: skipped");
            continue;
        };
        let (index, _) = OdfIndex::build(&vfs);
        let (db, _) = ContentDb::load(&vfs, &index).unwrap();
        let (races, _) = Races::load(&vfs, &index);
        let offered = skirmish_races(&races, &db);
        let playable: Vec<&str> = races.playable().iter().map(|r| r.name.as_str()).collect();
        let mut expected = Vec::new();
        for name in &playable {
            if !expected.contains(&name.to_string()) {
                expected.push(name.to_string());
            }
        }
        assert_eq!(offered, expected, "{label}: choices must follow the race catalogue");
        for r in &offered {
            assert!(
                playable.contains(&r.as_str()),
                "{label}: {r} offered but not playable"
            );
        }
        for def in races.playable() {
            let l = races.loadout(&db, &def.name);
            let odfs: Vec<String> = l
                .units
                .iter()
                .map(|k| db.unit(*k).unwrap().odf.clone())
                .collect();
            eprintln!("{label} {} ({}) -> {odfs:?}", def.stem, def.name);
            assert!(
                !l.units.is_empty(),
                "{label}: {} starts with nothing",
                def.stem
            );
            // Every start unit is of the race or is a standard unit the data names.
            let has_freighter = l
                .units
                .iter()
                .any(|k| db.unit(*k).unwrap().class.0 == "freighter");
            let names_freighter = def.standard_units.iter().any(|s| {
                db.key(s)
                    .and_then(|k| db.unit(k))
                    .is_some_and(|u| u.class.0 == "freighter")
            });
            assert_eq!(
                has_freighter, names_freighter,
                "{label} {}: freighters come only from standardunits",
                def.stem
            );
            // Something that can build starts.
            assert!(
                l.units
                    .iter()
                    .any(|k| !db.unit(*k).unwrap().build.list.is_empty()),
                "{label} {}: nothing builds",
                def.stem
            );
        }
        if label == "FO4" {
            let l = races.loadout(&db, "unitedfederation");
            let odfs: Vec<String> = l
                .units
                .iter()
                .map(|k| db.unit(*k).unwrap().odf.clone())
                .collect();
            assert!(
                odfs.iter().any(|o| o == "fconst"),
                "FO4 Federation starts with a construction ship: {odfs:?}"
            );
            // fconst is Roots' ODF (its list costs the metal slot, which FO4's stock lacks); the
            // start outpost builds the mod's own construction ships, whose structures cost
            // dilithium and latinum (the Tritanium): one of them is payable from the stock.
            let pays = l
                .units
                .iter()
                .filter_map(|k| db.unit(*k))
                .flat_map(|y| y.build.list.iter())
                .filter_map(|r| db.unit(r.key))
                .filter(|u| u.class.0 == "constructionrig")
                .flat_map(|ship| ship.build.list.iter())
                .filter_map(|r| db.unit(r.key))
                .any(|s| {
                    s.build.cost.iter().any(|c| *c > sta_content::Fixed::ZERO)
                        && s.build
                            .cost
                            .iter()
                            .enumerate()
                            .all(|(i, c)| *c <= l.stock[i])
                });
            assert!(
                pays,
                "no structure of a construction ship the start builds is payable at tick 0"
            );
        }
        if label == "DW" {
            for key in ["fed", "dom", "borg"] {
                let d = races
                    .playable()
                    .into_iter()
                    .find(|r| r.name == key)
                    .unwrap_or_else(|| panic!("DW: {key} not playable"));
                assert!(d.slot.is_some() && !d.standard_units.is_empty());
            }
        }
        checked += 1;
    }
    if checked == 0 {
        eprintln!("no install mounted: nothing checked");
    }
}
