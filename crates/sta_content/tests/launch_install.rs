//! Install-gated (STA64_TEST_INSTALL + STA64_TEST_MOD, STA64_TEST_DIR; skipped
//! when unset, like `odf_load_install`): every builder (a unit with a build
//! list) resolves a launch point, either its `buildHardpoint` node or the
//! stated fallback (builder centre, facing ahead). Prints the tallies; run it
//! against FO4, STA2 Classic, FO, DW (STA64_TEST_INSTALL/MOD) and A1, A2
//! (STA64_TEST_DIR, a plain folder). An empty STA64_TEST_MOD mounts the base
//! game alone (FO, DW).

use std::path::Path;

use sta_content::{ContentDb, OdfIndex};
use sta_vfs::Vfs;

fn check(vfs: &Vfs, what: &str) {
    let (index, _) = OdfIndex::build(vfs);
    let (db, _) = ContentDb::load(vfs, &index).unwrap();
    let (mut builders, mut node, mut fallback) = (0, 0, 0);
    let mut bad = Vec::new();
    for u in db.units().filter(|u| !u.build.list.is_empty()) {
        builders += 1;
        let Some(l) = &u.launch else {
            bad.push(format!("{}: no launch point", u.odf));
            continue;
        };
        match &l.node {
            Some(_) => node += 1,
            None => fallback += 1,
        }
        let len =
            |v: (sta_content::Fixed, sta_content::Fixed)| (v.0 * v.0 + v.1 * v.1).sqrt().to_f32();
        if (len(l.dir) - 1.0).abs() > 0.01 {
            bad.push(format!("{}: direction is not a unit vector", u.odf));
        }
        if l.clear < sta_content::Fixed::from_int(1) || len(l.offset) > l.clear.to_f32() + 0.01 {
            bad.push(format!(
                "{}: clear radius {} does not enclose the hardpoint",
                u.odf,
                l.clear.to_f32()
            ));
        }
    }
    eprintln!(
        "{what}: {builders} builders, {node} at a hardpoint node, {fallback} on the fallback"
    );
    assert!(bad.is_empty(), "{what}: {bad:?}");
    assert!(builders > 0, "{what}: no builders found");
}

#[test]
fn every_builder_resolves_a_launch_point() {
    if let (Ok(root), Ok(m)) = (
        std::env::var("STA64_TEST_INSTALL"),
        std::env::var("STA64_TEST_MOD"),
    ) {
        let vfs = Vfs::mount_install(Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
        check(&vfs, &format!("install {m}"));
    }
    if let Ok(dir) = std::env::var("STA64_TEST_DIR") {
        let mut vfs = Vfs::default();
        vfs.push_dir("root".into(), dir.clone().into()).unwrap();
        check(&vfs, &format!("dir {dir}"));
    }
}
