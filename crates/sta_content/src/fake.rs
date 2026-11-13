//! Content to build and test against before the real loaders exist
//! (docs/WORKPLAN.md, "Fakes and fixtures"). Made up in code; no game data.
//! CON-04's `fixtures/content/` holds the same units as JSON5 and must load
//! to the same [`ContentDb::hash`].
//!
//! Fakes stay when the real thing lands; tests that use them keep running.

use sta_core::{Fixed, Vec3};

use crate::def::{DefRef, FireArc, Hardpoint, LaunchPoint, MoveStats, ShieldDef, UnitDef, WeaponDef, WeaponKind};
use crate::features::{Features, Profile};
use crate::map::{MapDef, MapObject, Region, RegionKind, RegionShape, Scenery, SceneryLight, SceneryPlanet, StartPos};
use crate::ContentDb;

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}

fn v(x: i32, y: i32, z: i32) -> Vec3 {
    Vec3 { x: n(x), y: n(y), z: n(z) }
}

fn hardpoint(weapon: &str, node: &str, arc: Option<FireArc>) -> Hardpoint {
    Hardpoint { weapon: DefRef::new(weapon), node: node.into(), arc }
}

/// Ahead, 90 degrees either side (cos 45 = 0.7071).
fn fore_arc() -> FireArc {
    FireArc { dir: (Fixed::ZERO, Fixed::ONE), min_dot: Fixed::from_f64(std::f64::consts::FRAC_1_SQRT_2) }
}

/// Six units and three weapons, Native profile with directional shields,
/// fire arcs and point-defence cycles on:
///
/// | name | class | role |
/// |---|---|---|
/// | `fx_cruiser` | `craft` | beam fore, torpedo all round |
/// | `fx_frigate` | `craft` | fast, beam fore |
/// | `fx_station` | `starbase` | static, beam and point defence; builds `fx_miner` |
/// | `fx_yard` | `shipyard` | static; builds `fx_cruiser`, `fx_frigate` |
/// | `fx_miner` | `freighter` | unarmed |
/// | `fx_moon` | `planet` | static resource, no hull |
/// | `fx_beam`, `fx_torpedo`, `fx_pd` | weapons | beam, projectile, point defence |
pub fn db() -> ContentDb {
    let features = Features { directional_shields: true, fire_arcs: true, point_defense_cycles: true, ..Default::default() };

    let mut beam = WeaponDef::new("fx_beam", "phaser", WeaponKind::Beam);
    beam.range = n(600);
    beam.damage = n(20);
    beam.refire = 45;
    beam.duration = 15;
    beam.fire_sound = Some("fx_beam_fire".into());
    beam.fx = Some("fx_beam_red".into());

    let mut torpedo = WeaponDef::new("fx_torpedo", "photontorpedo", WeaponKind::Projectile);
    torpedo.range = n(900);
    torpedo.damage = n(60);
    torpedo.refire = 90;
    torpedo.shot_speed = n(400);
    torpedo.fire_sound = Some("fx_torpedo_fire".into());
    torpedo.fx = Some("fx_torpedo_glow".into());

    let mut pd = WeaponDef::new("fx_pd", "pphaser", WeaponKind::PointDefence);
    pd.range = n(250);
    pd.damage = n(5);
    pd.refire = 10;
    pd.duration = 3;

    let mut cruiser = UnitDef::new("fx_cruiser", "craft");
    cruiser.race = "fx".into();
    cruiser.model = "fx_cruiser".into();
    cruiser.movement = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
    cruiser.hull = n(800);
    cruiser.shields = ShieldDef { max: n(400), recharge: Fixed::from_f64(0.5) };
    cruiser.crew = n(300);
    cruiser.sensor = n(1000);
    cruiser.hardpoints = vec![hardpoint("fx_beam", "hp01", Some(fore_arc())), hardpoint("fx_torpedo", "hp02", None)];
    cruiser.build.cost = vec![n(600)];
    cruiser.build.ticks = 900;
    cruiser.build.officers = n(4);
    cruiser.sounds.select = vec!["fx_cruiser_select".into()];
    cruiser.sounds.ack_move = vec!["fx_cruiser_move".into()];
    cruiser.sounds.ack_attack = vec!["fx_cruiser_attack".into()];
    cruiser.sounds.explode = Some("fx_explode_large".into());
    cruiser.fx.explosion = Some("fx_explosion_large".into());
    cruiser.fx.damage = vec!["fx_smoke".into(), "fx_fire".into()];

    let mut frigate = UnitDef::new("fx_frigate", "craft");
    frigate.race = "fx".into();
    frigate.model = "fx_frigate".into();
    frigate.movement = MoveStats::from_f32(200.0, 1.5, 1.6, 1.6, 0.3);
    frigate.hull = n(300);
    frigate.shields = ShieldDef { max: n(150), recharge: Fixed::from_f64(0.25) };
    frigate.crew = n(80);
    frigate.sensor = n(1200);
    frigate.hardpoints = vec![hardpoint("fx_beam", "hp01", Some(fore_arc()))];
    frigate.build.cost = vec![n(250)];
    frigate.build.ticks = 450;
    frigate.build.officers = n(2);
    frigate.sounds.explode = Some("fx_explode_small".into());
    frigate.fx.explosion = Some("fx_explosion_small".into());

    let mut station = UnitDef::new("fx_station", "starbase");
    station.race = "fx".into();
    station.model = "fx_station".into();
    station.hull = n(3000);
    station.shields = ShieldDef { max: n(1500), recharge: n(1) };
    station.crew = n(1000);
    station.sensor = n(1500);
    station.hardpoints = vec![hardpoint("fx_beam", "hp01", None), hardpoint("fx_pd", "hp02", None)];
    station.build.cost = vec![n(1200)];
    station.build.ticks = 1800;
    station.build.list = vec![DefRef::new("fx_miner")];

    let mut yard = UnitDef::new("fx_yard", "shipyard");
    yard.race = "fx".into();
    yard.model = "fx_yard".into();
    yard.hull = n(2000);
    yard.shields = ShieldDef { max: n(800), recharge: n(1) };
    yard.crew = n(500);
    yard.sensor = n(800);
    yard.build.cost = vec![n(900)];
    yard.build.ticks = 1500;
    yard.build.list = vec![DefRef::new("fx_cruiser"), DefRef::new("fx_frigate")];

    let mut miner = UnitDef::new("fx_miner", "freighter");
    miner.race = "fx".into();
    miner.model = "fx_miner".into();
    miner.movement = MoveStats::from_f32(110.0, 0.8, 0.8, 0.8, 0.2);
    miner.hull = n(250);
    miner.shields = ShieldDef { max: n(100), recharge: Fixed::from_f64(0.25) };
    miner.crew = n(40);
    miner.sensor = n(600);
    miner.build.cost = vec![n(400)];
    miner.build.ticks = 600;
    miner.build.officers = n(1);

    let mut moon = UnitDef::new("fx_moon", "planet");
    moon.model = "fx_moon".into();
    moon.scale = n(4);

    ContentDb::builder()
        .profile(Profile::Native)
        .features(features)
        .weapon(beam)
        .weapon(torpedo)
        .weapon(pd)
        .unit(cruiser)
        .unit(frigate)
        .unit(station)
        .unit(yard)
        .unit(miner)
        .unit(moon)
        .build()
        .expect("fake content is consistent")
}

/// `fx_yard_launch` (SIM-16): a yard like `fx_yard`, building `fx_frigate`,
/// with a launch hardpoint (`hp99`) 40 units ahead of its centre and a clear
/// circle of 60. Not in [`db`], so its pinned hash stays; scenes that need it
/// add it (`sta_bench`'s `fake_content`).
pub fn launch_yard() -> UnitDef {
    let mut y = UnitDef::new("fx_yard_launch", "shipyard");
    y.race = "fx".into();
    y.model = "fx_yard_launch".into();
    y.hull = n(2000);
    y.crew = n(500);
    y.sensor = n(800);
    y.build.cost = vec![n(900)];
    y.build.ticks = 1500;
    y.build.list = vec![DefRef::new("fx_frigate")];
    y.launch = Some(LaunchPoint { offset: (Fixed::ZERO, n(40)), dir: (Fixed::ZERO, Fixed::ONE), clear: n(60), node: Some("hp99".into()) });
    y
}

/// A two-player map for [`db`]: two starts facing each other across a
/// mineable moon, a nebula on one flank, an asteroid field on the other, and
/// a sky with one planet and one light.
pub fn two_bases() -> MapDef {
    let mut map = MapDef::empty();
    map.name = "fx_two_bases".into();
    map.label = Some("Two Bases".into());
    map.starts = vec![
        StartPos { slot: 0, pos: v(-4000, 0, 0), heading: (Fixed::ONE, Fixed::ZERO) },
        StartPos { slot: 1, pos: v(4000, 0, 0), heading: (-Fixed::ONE, Fixed::ZERO) },
    ];
    map.objects = vec![MapObject {
        odf: "fx_moon".into(),
        owner: None,
        pos: v(0, 0, 0),
        heading: (Fixed::ZERO, Fixed::ONE),
        label: Some("moon".into()),
    }];
    map.regions = vec![
        Region {
            kind: RegionKind::Nebula,
            odf: "fx_nebula".into(),
            shape: RegionShape::Sphere { centre: v(0, 0, 3000), radius: n(1200) },
        },
        Region {
            kind: RegionKind::AsteroidField,
            odf: "fx_asteroids".into(),
            shape: RegionShape::Box { min: v(-1500, -200, -4000), max: v(1500, 200, -2500) },
        },
    ];
    map.scenery = Scenery {
        sky: Some("fx_sky".into()),
        starfield: Some("fx_stars".into()),
        ambient: [0.1, 0.1, 0.15],
        planets: vec![SceneryPlanet { odf: "fx_planet".into(), pos: [20000.0, -3000.0, 15000.0], radius: 6000.0 }],
        lights: vec![SceneryLight { pos: [-30000.0, 10000.0, 0.0], colour: [1.0, 0.95, 0.9] }],
    };
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_core::ContentKey;

    #[test]
    fn db_has_six_units_and_three_weapons() {
        let db = db();
        let units: Vec<_> = db.units().map(|u| u.odf.as_str()).collect();
        assert_eq!(units, ["fx_cruiser", "fx_frigate", "fx_miner", "fx_moon", "fx_station", "fx_yard"]);
        let weapons: Vec<_> = db.weapons().map(|w| (w.odf.as_str(), w.kind)).collect();
        assert_eq!(
            weapons,
            [("fx_beam", WeaponKind::Beam), ("fx_pd", WeaponKind::PointDefence), ("fx_torpedo", WeaponKind::Projectile)]
        );
        assert_eq!(db.key("fx_beam"), Some(ContentKey(0)));
        // References are resolved to keys.
        let cruiser = db.unit(db.key("fx_cruiser").unwrap()).unwrap();
        assert_eq!(cruiser.hardpoints[1].weapon.key, db.key("fx_torpedo").unwrap());
        let yard = db.unit(db.key("fx_yard").unwrap()).unwrap();
        assert_eq!(yard.build.list[0].key, cruiser.key);
        assert!(db.features().fire_arcs);
    }

    /// The hash pins the fake: CON-04's JSON5 copy must reproduce it, and a
    /// change here is a change every fake-based golden digest will see.
    #[test]
    fn db_hash_is_stable() {
        assert_eq!(db().hash(), db().hash());
        assert_eq!(db().hash(), 0x6977_6a57_dec2_2e6c);
    }

    #[test]
    fn two_bases_contents() {
        let m = two_bases();
        let db = db();
        assert_eq!(m.starts.len(), 2);
        assert_eq!(m.starts[0].slot, 0);
        assert_eq!(m.starts[1].slot, 1);
        assert_eq!(m.objects.len(), 1);
        assert!(db.key(&m.objects[0].odf).is_some(), "the moon is in the fake db");
        let kinds: Vec<_> = m.regions.iter().map(|r| r.kind).collect();
        assert_eq!(kinds, [RegionKind::Nebula, RegionKind::AsteroidField]);
        assert!(m.scenery.sky.is_some());
        assert_eq!(m.scenery.planets.len(), 1);
        // Everything placed is inside the bounds.
        let (lo, hi) = m.bounds;
        for p in m.starts.iter().map(|s| s.pos).chain(m.objects.iter().map(|o| o.pos)) {
            assert!(lo.x <= p.x && p.x <= hi.x && lo.z <= p.z && p.z <= hi.z);
        }
    }
}
