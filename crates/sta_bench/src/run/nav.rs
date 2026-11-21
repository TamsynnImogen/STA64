//! Maps and content for the pathfinding scenes (SIM-23): `nav-*` scenarios.

use sta_content::{MapDef, Profile, Region, RegionKind, RegionShape, UnitDef};
use sta_core::{Fixed, Vec3};

fn at(x: i32, z: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::ZERO,
        z: Fixed::from_int(z),
    }
}

fn sphere(kind: RegionKind, odf: &str, x: i32, z: i32, r: i32) -> Region {
    Region {
        kind,
        odf: odf.into(),
        shape: RegionShape::Sphere {
            centre: at(x, z),
            radius: Fixed::from_int(r),
        },
    }
}

/// An authored hazard nebula with a pathing cost above open space (100).
/// Loaded through the ODF content path used for an install's nebulae.
pub(super) fn route_hazard() -> UnitDef {
    const ODF: &str =
        "classLabel = \"nebula_obj\"\nunitName = \"Route Hazard\"\npathing_cost = 1700\n";
    let stems = ["fx_route_hazard".to_owned()];
    let (db, warnings) = sta_content::odf::load_with(
        Profile::default(),
        &Default::default(),
        &stems,
        &mut |_, version| (version == 0).then(|| ODF.to_owned()),
    )
    .expect("the route hazard ODF loads");
    db.units()
        .find(|u| u.odf.eq_ignore_ascii_case("fx_route_hazard"))
        .unwrap_or_else(|| panic!("no fx_route_hazard def: {warnings:?}"))
        .clone()
}

/// `fx_nebula_cross`: an empty map with one authored hazard nebula, a sphere of 400
/// centred on (0, 0, 1500).
pub(super) fn nebula_map() -> MapDef {
    let mut map = MapDef::empty();
    map.name = "fx_nebula_cross".into();
    map.regions = vec![sphere(RegionKind::Nebula, "fx_route_hazard", 0, 1500, 400)];
    map
}

/// `fx_barrier`: a Fleet Operations style map barrier, a wall of hidden
/// blocking fields (`trigger_blocking`, `fieldRadius = 400`) every 600 units
/// along z = 1500 from x = -6000 to 6000, with one gap between x = 1600 and
/// x = 3200 (field edges).
pub(super) fn barrier_map() -> MapDef {
    let mut map = MapDef::empty();
    map.name = "fx_barrier".into();
    map.regions = (-6000..=1200)
        .step_by(600)
        .chain((3600..=6000).step_by(600))
        .map(|x| sphere(RegionKind::AsteroidField, "trigger_blocking", x, 1500, 400))
        .collect();
    map
}

/// `fx_enclosed`: twelve fields of 300 on a ring of 900 round (0, 0, 3000),
/// overlapping, so nothing outside can reach the centre.
pub(super) fn enclosed_map() -> MapDef {
    // 900 * (sin, cos) of k * 30 degrees, rounded.
    const RING: [(i32, i32); 12] = [
        (0, 900),
        (450, 779),
        (779, 450),
        (900, 0),
        (779, -450),
        (450, -779),
        (0, -900),
        (-450, -779),
        (-779, -450),
        (-900, 0),
        (-779, 450),
        (-450, 779),
    ];
    let mut map = MapDef::empty();
    map.name = "fx_enclosed".into();
    map.regions = RING
        .iter()
        .map(|&(x, z)| sphere(RegionKind::AsteroidField, "fx_asteroids", x, 3000 + z, 300))
        .collect();
    map
}
