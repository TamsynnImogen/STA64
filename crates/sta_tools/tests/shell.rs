//! EDT-01: the tool shell's core (`--features shell`). A synthetic install for
//! the save round trip; the installed game, when `STA64_TEST_INSTALL` names
//! one, for listing by root and a SOD preview.
#![cfg(feature = "shell")]

use std::fs;
use std::path::PathBuf;

use sta_tools::shell::{Workspace, mods, preview_triangles};

fn scratch(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("sta_tools-shell-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

/// `Data/odf/ships/alpha.odf` and a mod `m` overriding it with `Data/Mods/m/odf/ships/alpha.odf`.
fn install() -> PathBuf {
    let root = scratch("install");
    let data = root.join("Data");
    fs::create_dir_all(data.join("odf/ships")).unwrap();
    fs::write(data.join("odf/ships/alpha.odf"), "classLabel = \"craft\"\r\nbasename = \"alphahull\"\r\n").unwrap();
    fs::write(data.join("odf/ships/beta.odf"), "classLabel = \"craft\"\r\n").unwrap();
    let m = data.join("Mods/m");
    fs::create_dir_all(m.join("odf/ships")).unwrap();
    fs::write(m.join("info.ini"), "[Mod]\nName = m\n").unwrap();
    fs::write(m.join("odf/ships/alpha.odf"), "classLabel = \"craft\"\r\nbasename = \"modhull\"\r\n").unwrap();
    root
}

#[test]
fn an_install_and_a_mod_list_their_odfs_by_root_and_a_save_round_trips_with_a_backup() {
    let root = install();
    assert_eq!(mods(&root), ["m"]);
    let mut ws = Workspace::open(&root, Some("m")).unwrap();
    let roots = ws.roots();
    let names: Vec<_> = roots.iter().map(|r| (r.name.as_str(), r.odfs.iter().map(|o| o.0.as_str()).collect::<Vec<_>>())).collect();
    assert_eq!(roots.len(), 2, "{names:?}");
    assert_eq!(names[0].1, ["alpha"], "the mod's alpha wins: {names:?}");
    assert_eq!(names[1].1, ["beta"], "{names:?}");
    assert_eq!(ws.model_name("alpha").as_deref(), Some("modhull"));
    assert_eq!(ws.model_name("beta").as_deref(), Some("beta"), "no basename: the stem");

    let before = ws.read_odf("alpha").unwrap();
    let edited = before.replace("modhull", "newhull");
    let backup = ws.save("alpha", &edited).unwrap();
    assert_eq!(fs::read_to_string(&backup).unwrap(), before, "the previous bytes, CRLF kept");
    assert_eq!(ws.read_odf("alpha").unwrap(), edited, "the reopened index reads the new text");
    assert_eq!(ws.model_name("alpha").as_deref(), Some("newhull"));
    assert!(fs::read_to_string(root.join("Data/odf/ships/alpha.odf")).unwrap().contains("alphahull"), "the base file is untouched");
    assert!(ws.read_odf("gamma").is_err());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_installed_game_lists_by_root_and_previews_a_model() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
        eprintln!("SKIP: set STA64_TEST_INSTALL to an Armada II family install");
        return;
    };
    let m = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
    let ws = Workspace::open(PathBuf::from(&root).as_path(), m.as_deref()).unwrap();
    let roots = ws.roots();
    let listed: usize = roots.iter().map(|r| r.odfs.len()).sum();
    assert_eq!(listed, ws.odf_count(), "every ODF sits in exactly one root");
    assert!(listed > 0);
    // The first ODF whose model the install has gives preview triangles.
    let shown = roots.iter().flat_map(|r| r.odfs.iter()).find_map(|(stem, _)| ws.model_name(stem).and_then(|n| ws.load_sod(&n)).map(|m| (stem.clone(), m)));
    let (stem, model) = shown.expect("some ODF names a SOD the install has");
    let (positions, indices) = preview_triangles(&model);
    eprintln!("{stem}: {} positions, {} triangles", positions.len(), indices.len() / 3);
    assert!(!indices.is_empty() && indices.len() % 3 == 0);
    assert!(indices.iter().all(|&i| (i as usize) < positions.len()));
}
