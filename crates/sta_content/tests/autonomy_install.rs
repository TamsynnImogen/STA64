//! Install-gated (STA64_TEST_INSTALL; skipped when unset): the default craft
//! autonomy read from each mod's `RTS_CFG.h` (CON-46). Read from the data:
//! Fleet Ops 4.0 sets movement 1 and special weapon 2, Classic Enhanced 1
//! and 1, plain Classic sets neither (the engine defaults 1 and 0 apply).

use std::path::Path;

use sta_content::{ContentDb, OdfIndex};
use sta_vfs::Vfs;

#[test]
fn installed_headers_give_the_documented_defaults() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
    // Roots itself (no mod) and Fleet Ops 4.0 set 1 and 2.
    let mut checked = 0;
    for (m, want) in [
        (None, (1, 2)),
        (Some("Fleet Ops 4.0"), (1, 2)),
        (Some("STA2 Classic Mod Enhanced Gameplay"), (1, 1)),
        (Some("STA2 Classic Mod"), (1, 0)),
    ] {
        let Ok(vfs) = Vfs::mount_install(Path::new(&root), m) else { continue };
        checked += 1;
        let m = m.unwrap_or("Roots");
        let (index, _) = OdfIndex::build(&vfs);
        let (db, _) = ContentDb::load(&vfs, &index).unwrap();
        let a = db.rules().autonomy();
        eprintln!("{m}: movement {} special weapon {}", a.movement, a.special_weapon);
        assert_eq!((a.movement, a.special_weapon), want, "{m}");
    }
    assert!(checked > 0, "no mod of the install could be mounted");
}
