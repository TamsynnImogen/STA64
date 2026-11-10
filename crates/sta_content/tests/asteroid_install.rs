//! Install-gated (STA64_TEST_INSTALL + STA64_TEST_MOD, empty for none; skipped
//! when unset): the asteroid fields of a real map reach the [`MapDef`] as
//! regions, with the radius their ODF gives.
//!
//! Fleet Operations 4 / Roots' `AsteroidZone` walls its rock sets off with
//! 116 hidden `trigger_blocking` fields (`fieldRadius = 400`; the `_sml`
//! variant has 100). Every map of the install is also checked for the general
//! rule: every `asteroidfield`-class object becomes an `AsteroidField` region.

use std::path::Path;

use sta_content::map::Conversion;
use sta_content::{ContentDb, MapDef, OdfIndex, RegionKind, RegionShape};
use sta_vfs::Vfs;

#[test]
fn asteroid_fields_become_regions() {
    let (Ok(root), Ok(m)) = (std::env::var("STA64_TEST_INSTALL"), std::env::var("STA64_TEST_MOD")) else { return };
    let vfs = Vfs::mount_install(Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let mut total = 0;
    for path in vfs.list("bzn").into_iter().filter(|p| p.to_ascii_lowercase().ends_with(".bzn")) {
        let Some(Ok(data)) = vfs.read(&path) else { continue };
        let Ok(bzn) = sta_bzn::parse(&data) else { continue };
        let fields = bzn.objects.iter().filter(|o| o.odf.to_ascii_lowercase().starts_with("trigger_blocking")).count();
        let Conversion { map, .. } = MapDef::convert_bzn_in(&bzn, &db, &vfs, &index);
        let regions: Vec<_> = map.regions.iter().filter(|r| r.kind == RegionKind::AsteroidField).collect();
        total += regions.len();
        if path.to_ascii_lowercase().ends_with("asteroidzone.bzn") {
            assert_eq!(regions.len(), fields, "{path}: every blocking field is a region");
            assert!(fields >= 100, "{path}: {fields} blocking fields");
            for r in &regions {
                let RegionShape::Sphere { radius, .. } = r.shape else { panic!("{path}: not a sphere") };
                assert!(radius.to_f32() == 400.0 || radius.to_f32() == 100.0, "{path}: radius {radius:?}");
            }
        } else {
            assert!(regions.len() >= fields, "{path}: {} regions for {fields} blocking fields", regions.len());
        }
    }
    eprintln!("{root} + {m}: {total} asteroid field regions over all maps");
}
