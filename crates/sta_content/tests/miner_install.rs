//! Install-gated (STA64_TEST_INSTALL + STA64_TEST_MOD, STA64_TEST_DIR; skipped
//! when unset, like `launch_install`): which units mine is decided by the
//! `miner` key where the ODF has one, else by class `freighter`. Prints every
//! unit where that differs from the old rule (a freighter always mined) and
//! checks `UnitDef::mines` against the raw key.

use std::path::Path;

use sta_content::{ContentDb, OdfIndex};
use sta_vfs::Vfs;

fn check(vfs: &Vfs, what: &str) {
    let (index, _) = OdfIndex::build(vfs);
    let (db, _) = ContentDb::load(vfs, &index).unwrap();
    let (mut miners, mut keyed, mut changed) = (0, 0, Vec::new());
    for u in db.units() {
        let key = u.raw.f32("miner");
        keyed += key.is_some() as usize;
        miners += u.mines() as usize;
        if let Some(k) = key {
            assert_eq!(u.mines(), k != 0.0, "{what}: {} miner = {k}", u.odf);
        }
        let old = u.class.0 == "freighter" || key == Some(1.0);
        if old != u.mines() {
            changed.push(format!("{} (class {}, miner {key:?})", u.odf, u.class.0));
        }
    }
    eprintln!(
        "{what}: {miners} miners, {keyed} units with a miner key, {} differ from the old rule: {changed:?}",
        changed.len()
    );
}

#[test]
fn the_miner_key_decides() {
    if let (Ok(root), Ok(m)) = (
        std::env::var("STA64_TEST_INSTALL"),
        std::env::var("STA64_TEST_MOD"),
    ) {
        let vfs = Vfs::mount_install(Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty()))
            .unwrap();
        check(&vfs, &format!("install {root} / {m}"));
    }
    if let Ok(dir) = std::env::var("STA64_TEST_DIR") {
        let mut vfs = Vfs::default();
        vfs.push_dir("root".into(), dir.clone().into()).unwrap();
        check(&vfs, &format!("dir {dir}"));
    }
}
