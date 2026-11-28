//! CON-54: the weapon and ordnance base keys are typed on `WeaponDef::base`
//! through the normal content load, includes and all. Nothing in the sim reads
//! them yet, so they are not in the content digest. Authored fixtures first,
//! then an install-gated test of general rules.

use std::path::Path;

use sta_content::{ContentDb, Fixed, OdfIndex, Overrides, Profile, WeaponBaseDef, odf::load_with};
use sta_vfs::Vfs;

const UNIT: &str = "unitName = \"x\"\nclassLabel = \"craft\"\nweapon1 = \"w_gun\"\nweaponHardpoints1 = \"hp01\"\n";

fn load(weapon: &str, ordnance: &str) -> ContentDb {
    let files = [("u_a.odf", UNIT), ("w_gun.odf", weapon), ("w_base.odf", "wpnName = \"Gun\"\nclassLabel = \"cannon\"\nordName = \"o_shot\"\nhitChance = 0.8\n\"fbattle.odf\" 0.5\ntargetEnemyCraft = 1\n"), ("o_shot.odf", ordnance)];
    load_with(Profile::Sta2Classic, &Overrides::default(), &["u_a".to_owned(), "w_gun".to_owned()], &mut |name, version| {
        files.iter().find(|f| f.0.eq_ignore_ascii_case(name) && version == 0).map(|f| f.1.to_owned())
    })
    .unwrap()
    .0
}

fn base(db: &ContentDb) -> &WeaponBaseDef {
    &db.weapon(db.key("w_gun").unwrap()).unwrap().base
}

const PLAIN_WEAPON: &str = "wpnName = \"Gun\"\nclassLabel = \"cannon\"\nordName = \"o_shot\"\nshotDelay = 2\nrange = 500\n";
const PLAIN_ORDNANCE: &str = "classLabel = \"photontorpedo\"\ndamageBase = 20\nshotSpeed = 300\n";

#[test]
fn keys_reach_the_loaded_definition_through_includes() {
    let plain = load(PLAIN_WEAPON, PLAIN_ORDNANCE);
    assert_eq!(*base(&plain), WeaponBaseDef::default());

    // The base file's keys and rows reach the weapon that includes it.
    let inherited = load("#include \"w_base.odf\"\nshotDelay = 2\nrange = 500\n", PLAIN_ORDNANCE);
    let b = base(&inherited);
    let hit = b.hit_chance.as_ref().unwrap();
    assert_eq!(hit.iter().collect::<Vec<_>>(), [("fbattle.odf", Fixed::from_f64(0.5))]);
    assert_eq!(b.targets.enemy_craft, Some(true));

    // The weapon's own assignment replaces the value and the rows together.
    let own = load("#include \"w_base.odf\"\nhitChance = 1.0\ntargetEnemyCraft = 0\n", PLAIN_ORDNANCE);
    let b = base(&own);
    assert_eq!(b.hit_chance.as_ref().map(|l| (l.base, l.rows.len())), Some((Fixed::ONE, 0)));
    assert_eq!(b.targets.enemy_craft, Some(false), "an explicit zero is kept, not absent");

    // Ordnance keys come from the ordnance file.
    let ord = load(PLAIN_WEAPON, "classLabel = \"photontorpedo\"\ndamageBase = 20\n\"fbattle.odf\" 1.5\nshotSpeed = 300\nignoreShield = 1\nomegaTurn = 2\n");
    let b = base(&ord);
    assert_eq!(b.damage_base.as_ref().map(|l| l.iter().collect::<Vec<_>>()), Some(vec![("fbattle.odf", Fixed::from_f64(1.5))]));
    assert_eq!((b.ignore_shield, b.omega_turn), (Some(true), Some(Fixed::from_int(2))));
    assert_eq!(db_damage(&ord), Fixed::from_int(20), "the plain damage value is unchanged by its rows");
}

fn db_damage(db: &ContentDb) -> Fixed {
    db.weapon(db.key("w_gun").unwrap()).unwrap().damage
}

#[test]
fn typed_only_keys_do_not_move_the_content_hash() {
    let plain = load(PLAIN_WEAPON, PLAIN_ORDNANCE).hash();
    let keyed = load(
        &format!("{PLAIN_WEAPON}hitChance = 0.5\n\"fbattle.odf\" 0.1\nneedTarget = 1\ntargetEnemyCraft = 0\nstopToHitModifier = -0.2\n"),
        &format!("{PLAIN_ORDNANCE}ignoreShield = 1\nshotAccel = 2\n"),  // omegaTurn is read now, under `ordnance_flight`
    )
    .hash();
    assert_eq!(plain, keyed, "nothing in the sim reads the base keys yet (SIM-73 digests what it reads)");
    assert_eq!(sta_content::fake::db().hash(), 0x9ef5_3881_3050_10f8);
}

#[test]
fn every_new_key_is_counted_as_read_by_the_coverage_list() {
    for k in [
        "hitChance", "stopToHitModifier", "toHitStopModifier", "impulseToHitModifier", "toHitImpulseModifier", "warpToHitModifier",
        "toHitWarpModifier", "slowToHitModifier", "toHitSlowModifier", "targetEnemyCraft", "targetEnemyBuildings", "targetAlliedCraft",
        "targetAlliedBuildings", "targetEnemyPlanets", "targetEnemyPlanetsOnly", "targetLocation", "AItargetLocation", "needTarget",
        "beneficial", "switchToAttack", "cloakEnable", "warpEnable", "crewAffectsWeaponDelay", "ignoreValidAsTarget", "regularWeaponDelay",
        "ignoreShield", "forceInvalidateTarget", "canAttackStations", "specialValue", "shieldDuration", "omegaTurn", "seekTime", "shotAccel",
        "detectRadius", "travelTimeFraction",
    ] {
        assert!(sta_content::coverage::READ_ODF_KEYS.contains(&k.to_ascii_lowercase().as_str()), "{k}");
    }
}

/// Reads the install named by `STA64_TEST_INSTALL` (and `STA64_TEST_MOD`, empty
/// for none) at run time and asserts rules that hold for any install; skipped
/// without it. Counts are printed with `--nocapture`.
#[test]
fn installed_weapons_obey_the_key_rules() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
    let m = std::env::var("STA64_TEST_MOD").unwrap_or_default();
    let vfs = Vfs::mount_install(Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, warnings) = ContentDb::load(&vfs, &index).unwrap();
    let mut tables = std::collections::HashSet::new();
    let (mut weapons, mut with_base, mut hit, mut hit_rows, mut damage_rows, mut masks, mut to_hit, mut ignore_shield) = (0, 0, 0, 0, 0, 0, 0, 0);
    for w in db.weapons() {
        let b = &w.base;
        weapons += 1;
        with_base += usize::from(*b != WeaponBaseDef::default());
        hit += usize::from(b.hit_chance.is_some());
        hit_rows += b.hit_chance.as_ref().map_or(0, |l| l.rows.len());
        damage_rows += b.damage_base.as_ref().map_or(0, |l| l.rows.len());
        masks += usize::from(b.targets != Default::default());
        to_hit += usize::from(b.to_hit != Default::default());
        ignore_shield += usize::from(b.ignore_shield == Some(true));
        for l in [&b.hit_chance, &b.damage_base].into_iter().flatten().filter(|l| !l.rows.is_empty()) {
            tables.insert(std::sync::Arc::as_ptr(&l.rows) as *const u8 as usize);
        }
        // An absent key reads as absent, never as a made-up value.
        assert_eq!(b.hit_chance.is_some(), w.raw.raw("hitChance").is_some(), "{}", w.odf);
        assert_eq!(b.need_target.is_some(), w.raw.raw("needTarget").is_some() && w.raw.bool("needTarget").is_some(), "{}", w.odf);
        // A table held here always has rows; the plain value stays on the definition.
        for table in [&b.damage_base, &b.damage_variance, &b.damage_threshold, &b.shield_crew_modifier, &b.hull_crew_modifier] {
            assert!(table.as_ref().is_none_or(|l| !l.rows.is_empty()), "{}", w.odf);
        }
        // A row is parsed or skipped with a warning; none is invented.
        assert!(b.hit_chance.as_ref().map_or(0, |l| l.rows.len()) <= w.raw.rows("hitChance").count(), "{}", w.odf);
    }
    let row_warnings = warnings.iter().filter(|x| format!("{x:?}").contains(" row ")).count();
    println!(
        "weapons {weapons}: {with_base} set a base key; hitChance {hit} ({hit_rows} rows); damageBase rows {damage_rows}; target masks {masks}; to-hit modifiers {to_hit}; ignoreShield {ignore_shield}; row warnings {row_warnings}; distinct shared tables {}", tables.len()
    );
}
