//! CON-55: the craft and object stat keys are typed on `UnitDef::stats`
//! through the normal content load. Nothing in the sim reads them yet, so
//! they are not in the content digest. Authored fixtures first, then an
//! install-gated test of general rules.

use std::path::Path;

use sta_content::{ContentDb, Fixed, OdfIndex, Overrides, Profile, UnitStatsDef, odf::load_with};
use sta_vfs::Vfs;

fn load(files: &[(&str, &str)]) -> ContentDb {
    let names: Vec<String> = files.iter().filter(|f| f.0.starts_with("u_")).map(|f| f.0.trim_end_matches(".odf").to_owned()).collect();
    load_with(Profile::Sta2Classic, &Overrides::default(), &names, &mut |name, version| {
        files.iter().find(|f| f.0.eq_ignore_ascii_case(name) && version == 0).map(|f| f.1.to_owned())
    })
    .unwrap()
    .0
}

fn stats<'a>(db: &'a ContentDb, name: &str) -> &'a UnitStatsDef {
    &db.unit(db.key(name).unwrap()).unwrap().stats
}

const PLAIN: &str = "unitName = \"x\"\nclassLabel = \"craft\"\nmaxHealth = 100\n";

#[test]
fn keys_reach_the_loaded_definition_through_includes() {
    let db = load(&[
        ("u_base.odf", "unitName = \"x\"\nclassLabel = \"craft\"\nintrinsicValue = 0.5\nhullTargetHardpoints = \"hp01\" \"hp02\"\nisHero = 1\n"),
        ("u_child.odf", "#include \"u_base.odf\"\nisHero = 0\nenginesTargetHardpoints = \"hp05\"\n"),
        ("u_none.odf", PLAIN),
    ]);
    assert_eq!(*stats(&db, "u_none"), UnitStatsDef::default());
    let child = stats(&db, "u_child");
    assert_eq!(child.intrinsic_value, Some(Fixed::from_f64(0.5)));
    assert_eq!(child.hull_target_hardpoints, ["hp01", "hp02"]);
    assert_eq!(child.is_hero, Some(false), "an explicit zero is kept, not absent");
    assert_eq!(child.system_target_hardpoints[0], ["hp05"]);
}

#[test]
fn typed_only_keys_do_not_move_the_content_hash() {
    let plain = load(&[("u_a.odf", PLAIN)]).hash();
    let keyed = format!("{PLAIN}intrinsicValue = 0.5\nisHero = 1\nhullTargetHardpoints = \"hp01\"\navoidanceClass = 2\nselfDestruct = \"gselfd\"\n");
    assert_eq!(plain, load(&[("u_a.odf", &keyed)]).hash(), "nothing in the sim reads the stat keys yet (SIM-74 digests what it reads)");
    assert_eq!(sta_content::fake::db().hash(), 0x9ef5_3881_3050_10f8);
}

#[test]
fn every_new_key_is_counted_as_read_by_the_coverage_list() {
    for k in [
        "intrinsicValue", "criticalHitPercent", "hullTargetHardpoints", "criticalTargetHardpoints", "enginesTargetHardpoints",
        "weaponsTargetHardpoints", "shieldGeneratorTargetHardpoints", "lifeSupportTargetHardpoints", "sensorsTargetHardpoints",
        "enginesSparkTime", "weaponsSparkTime", "shieldGeneratorSparkTime", "lifeSupportSparkTime", "sensorsSparkTime", "shipType", "isHero",
        "selfDestruct", "missileDetonator", "defaultGreenAlert", "alwaysUseSpecialWeapons", "damagedScan", "avoidanceClass", "collisionRadius",
        "shieldPad", "maxExtraWeapons", "canAttackPlanets", "canOnlyAttackPlanets", "recyclesInPlace", "shipIsBuiltByEvolution",
        "shipIsBuiltByShips", "healthRate", "percentNebulaWarn", "dockingSpeedFactor", "decommissionValue", "aiName",
    ] {
        assert!(sta_content::coverage::READ_ODF_KEYS.contains(&k.to_ascii_lowercase().as_str()), "{k}");
    }
}

/// Reads the install named by `STA64_TEST_INSTALL` (and `STA64_TEST_MOD`, empty
/// for none) at run time and asserts rules that hold for any install; skipped
/// without it. Counts are printed with `--nocapture`.
#[test]
fn installed_units_obey_the_key_rules() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
    let m = std::env::var("STA64_TEST_MOD").unwrap_or_default();
    let vfs = Vfs::mount_install(Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let (mut units, mut with_stats, mut value, mut hull_hp, mut system_hp, mut hero, mut ship_type, mut avoidance, mut self_destruct) = (0, 0, 0, 0, 0, 0, 0, 0, 0);
    for u in db.units() {
        let s = &u.stats;
        units += 1;
        with_stats += usize::from(*s != UnitStatsDef::default());
        value += usize::from(s.intrinsic_value.is_some());
        hull_hp += usize::from(!s.hull_target_hardpoints.is_empty());
        system_hp += usize::from(s.system_target_hardpoints.iter().any(|l| !l.is_empty()));
        hero += usize::from(s.is_hero == Some(true));
        ship_type += usize::from(s.ship_type.is_some());
        avoidance += usize::from(s.avoidance_class.is_some());
        self_destruct += usize::from(s.self_destruct.is_some());
        // An absent key reads as absent, never as a made-up value.
        assert_eq!(s.hull_target_hardpoints.is_empty(), u.raw.str("hullTargetHardpoints").is_none_or(|t| t.trim().is_empty()), "{}", u.odf);
        if u.raw.raw("intrinsicValue").is_none() {
            assert_eq!(s.intrinsic_value, None, "{}", u.odf);
        }
        if u.raw.raw("isHero").is_none() {
            assert_eq!(s.is_hero, None, "{}", u.odf);
        }
    }
    println!(
        "units {units}: {with_stats} set a stat key; intrinsicValue {value}; hull target hardpoints {hull_hp}; system target hardpoints {system_hp}; heroes {hero}; shipType {ship_type}; avoidanceClass {avoidance}; selfDestruct {self_destruct}"
    );
}
