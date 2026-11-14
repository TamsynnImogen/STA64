//! Install-gated (STA64_TEST_INSTALL + STA64_TEST_MOD, empty for none; skipped
//! when unset, and on installs without the stock Armada II map objects): the
//! random map generator (CON-49) names ODFs the install has, and its map
//! converts with the install's sizes.

use std::path::Path;

use sta_content::map::Conversion;
use sta_content::random_map::{self, ContentClasses};
use sta_content::{ContentDb, MapDef, OdfIndex, RegionKind};
use sta_vfs::Vfs;

#[test]
fn random_maps_convert_against_the_install() {
    let (Ok(root), Ok(m)) = (std::env::var("STA64_TEST_INSTALL"), std::env::var("STA64_TEST_MOD")) else { return };
    let vfs = Vfs::mount_install(Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let mut lookup = |name: &str| sta_odf::resolve_layered(&name.to_ascii_lowercase(), &mut |n, v| index.load(&vfs, n, v)).ok().map(std::sync::Arc::new);
    if lookup(random_map::WORMHOLE).is_none() || lookup("mnebula6").is_none() {
        eprintln!("{root} + {m}: not a stock Armada II object set; skipped");
        return;
    }
    for seed in [1, 2, 3, 1000, 123_456] {
        let bzn = random_map::generate(seed, 4, &mut ContentClasses { db: &db, odfs: &mut lookup });
        for o in &bzn.objects {
            assert!(lookup(&o.odf).is_some(), "seed {seed}: the install has no {}.odf", o.odf);
        }
        // The planets have a size, so their clearance is real.
        let planet = db.key(random_map::START_PLANET).and_then(|k| db.unit(k)).expect("the start planet is a unit");
        assert!(planet.radius.to_f32() > 0.0);
        let Conversion { map, counts, warnings } = MapDef::convert_bzn_in(&bzn, &db, &vfs, &index);
        assert!(map.starts.len() >= 4, "seed {seed}: {} starts", map.starts.len());
        assert_eq!(counts.total(), bzn.objects.len());
        assert_eq!(counts.lights, 2, "seed {seed}");
        // One planet, moon and latinum nebula per start at the least.
        assert!(counts.objects >= 3 * map.starts.len(), "seed {seed}: {counts:?}");
        let nebulae = map.regions.iter().filter(|r| r.kind == RegionKind::Nebula).count();
        let fields = map.regions.iter().filter(|r| r.kind == RegionKind::AsteroidField).count();
        assert_eq!(nebulae, bzn.objects.iter().filter(|o| o.odf.starts_with("mnebula")).count());
        assert_eq!(fields, bzn.objects.iter().filter(|o| o.odf == random_map::ASTEROID_FIELD).count());
        // Only wormholes and black holes have no place in a MapDef yet.
        let dropped = bzn.objects.iter().filter(|o| [random_map::WORMHOLE, random_map::BLACK_HOLE].contains(&o.odf.as_str())).count();
        assert!(counts.skipped + counts.props <= dropped, "seed {seed}: {counts:?}, {dropped} holes; {warnings:?}");
        eprintln!("seed {seed}: {:.0} across, {} starts, {counts:?}", bzn.size[0], map.starts.len());
    }
}
