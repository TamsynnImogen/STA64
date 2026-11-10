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
