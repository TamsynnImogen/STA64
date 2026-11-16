//! CON-52: Armada I's physics keys reach the content database.
//!
//! Authored ODFs through an in-memory loader; the numbers match no shipped
//! file. Rules under test: the play block comes from the profile's play file
//! and only under `Sta1Classic`; a craft's Borg keys come from its own physics
//! file through the unit ODF's include chain; absent keys take the
//! executable's defaults; an unusable block leaves the model off; other
//! profiles neither read nor hash any of it.

use std::collections::HashMap;

use sta_content::odf::load_with;
use sta_content::{ContentDb, Fixed, Overrides, Profile, Warning};

const PLAY: &str = "[GameObjectClass]\nclassLabel = \"physics\"\nimpulseSpeed = 12\nfrontBackInertia = 6\nmaxReverseSpeed = 0.5\nturnRadius = 9\n\
velocCoeffStill = 0.25\nvelocWhenCoeffHalf = 0.6\nbraccelControlStiffness = 2\nsy_yawAngleCoeff = 1.5\nsy_yawVelocCoeff = 0.25\nrollRotationSpeed = 0.75\n";

fn fixtures(play: &str) -> Vec<(String, String)> {
    let f = |n: &str, t: &str| (n.to_owned(), t.to_owned());
    vec![
        f("fastphys", play),
        f("plainphys", "[GameObjectClass]\nclassLabel = \"physics\"\nimpulseSpeed = 15\nwarpSpeed = 40\n"),
        f("hivephys", "[GameObjectClass]\nclassLabel = \"physics\"\nimpulseSpeed = 14\nborgPhysics = 1\nmaxRotationSpeed = 0.5\nrotationAcceleration = 3\nrotationStiffness = 1.5\ntooSlowToTurn = 4\n"),
        f("bareborg", "[GameObjectClass]\nclassLabel = \"physics\"\nimpulseSpeed = 14\nborgPhysics = 1\n"),
        f("hull", "[GameObjectClass]\nclassLabel = \"craft\"\nunitName = \"Base\"\nphysicsFile = \"plainphys.odf\"\n"),
        f("ship_a", "[GameObjectClass]\n#include \"hull.odf\"\nunitName = \"A\"\n"),
        f("ship_b", "[GameObjectClass]\n#include \"hull.odf\"\nunitName = \"B\"\n"),
        f("drone", "[GameObjectClass]\nclassLabel = \"craft\"\nunitName = \"D\"\nphysicsFile = \"hivephys.odf\"\n"),
        f("drone2", "[GameObjectClass]\nclassLabel = \"craft\"\nunitName = \"D2\"\nphysicsFile = \"bareborg.odf\"\n"),
        f("yard", "[GameObjectClass]\nclassLabel = \"shipyard\"\nunitName = \"Y\"\n"),
    ]
}

fn load(profile: Profile, files: &[(String, String)], stems: &[&str]) -> (ContentDb, Vec<Warning>) {
    let map: HashMap<String, String> = files.iter().map(|(n, t)| (format!("{n}.odf"), t.clone())).collect();
    let stems: Vec<String> = stems.iter().map(|s| (*s).to_owned()).collect();
    load_with(profile, &Overrides::default(), &stems, &mut |n, v| {
        let n = if n.ends_with(".odf") { n.to_owned() } else { format!("{n}.odf") };
        if v == 0 { map.get(&n.to_ascii_lowercase()).cloned() } else { None }
    })
    .unwrap()
}

const UNITS: [&str; 5] = ["ship_a", "ship_b", "drone", "drone2", "yard"];

fn near(v: Fixed, x: f64) -> bool {
    (v.to_f32() as f64 - x).abs() < 1e-4
}

#[test]
fn the_play_block_is_read_for_armada_i_as_written() {
    let (db, _) = load(Profile::Sta1Classic, &fixtures(PLAY), &UNITS);
    let p = db.a1_play_physics().expect("play block");
    assert!(near(p.impulse_speed, 12.0) && near(p.front_back_inertia, 6.0) && near(p.max_reverse_speed, 0.5));
    assert!(near(p.turn_radius, 9.0) && near(p.veloc_coeff_still, 0.25) && near(p.veloc_when_coeff_half, 0.6));
    assert!(near(p.braccel_control_stiffness, 2.0) && near(p.yaw_angle_coeff, 1.5) && near(p.yaw_veloc_coeff, 0.25));
    assert!(near(p.roll_rotation_speed, 0.75));
}

#[test]
fn craft_sharing_a_base_share_a_physics_file_and_borg_keys_come_from_the_own_file() {
    let (db, _) = load(Profile::Sta1Classic, &fixtures(PLAY), &UNITS);
    let unit = |n: &str| db.unit(db.key(n).unwrap()).unwrap();
    // Both ships reach `plainphys` through `hull`; neither is a Borg craft.
    assert_eq!(unit("ship_a").movement, unit("ship_b").movement);
    assert!(near(unit("ship_a").movement.max_speed, 15.0));
    assert!(unit("ship_a").a1_borg.is_none() && unit("ship_b").a1_borg.is_none() && unit("yard").a1_borg.is_none());
    let b = unit("drone").a1_borg.expect("borg keys");
    assert!(near(b.max_rotation_speed, 0.5) && near(b.rotation_acceleration, 3.0));
    assert!(near(b.rotation_stiffness, 1.5) && near(b.too_slow_to_turn, 4.0));
}

#[test]
fn absent_keys_take_the_executable_defaults() {
    let bare = "[GameObjectClass]\nclassLabel = \"physics\"\nimpulseSpeed = 12\n";
    let (db, _) = load(Profile::Sta1Classic, &fixtures(bare), &UNITS);
    let p = db.a1_play_physics().expect("play block with defaults");
    assert!(near(p.front_back_inertia, 2.0) && near(p.max_reverse_speed, 15.0) && near(p.turn_radius, 25.0));
    assert!(near(p.braccel_control_stiffness, 10.0) && near(p.veloc_when_coeff_half, 7.5));
    let warned = |w: &[Warning]| w.iter().any(|w| w.file == "fastphys.odf" && w.message.contains("rollRotationSpeed"));
    let (_, w) = load(Profile::Sta1Classic, &fixtures(bare), &UNITS);
    assert!(warned(&w), "an absent roll key says there is no lag: {w:?}");
    let (_, w) = load(Profile::Sta1Classic, &fixtures(PLAY), &UNITS);
    assert!(!warned(&w), "{w:?}");
    let b = db.unit(db.key("drone2").unwrap()).unwrap().a1_borg.expect("borg");
    assert!(near(b.max_rotation_speed, 0.25) && near(b.rotation_acceleration, 2.0));
    assert!(near(b.rotation_stiffness, 1.0) && near(b.too_slow_to_turn, 1.0));
}

#[test]
fn an_unusable_play_block_leaves_the_model_off_without_failing_the_load() {
    for bad in [
        "classLabel = \"physics\"\nimpulseSpeed = 0\n",
        "classLabel = \"physics\"\nimpulseSpeed = 12\nturnRadius = 0\n",
        "classLabel = \"physics\"\nimpulseSpeed = 12\nfrontBackInertia = -3\n",
        "classLabel = \"physics\"\nimpulseSpeed = 12\nbraccelControlStiffness = 0\n",
        "classLabel = \"physics\"\nimpulseSpeed = 12\nvelocWhenCoeffHalf = 0\n",
        "classLabel = \"physics\"\nimpulseSpeed = nonsense\n",
    ] {
        let (db, warnings) = load(Profile::Sta1Classic, &fixtures(bad), &UNITS);
        assert!(db.a1_play_physics().is_none(), "{bad}");
        assert!(warnings.iter().any(|w| w.file == "fastphys.odf"), "{bad}: {warnings:?}");
        assert!(db.unit(db.key("ship_a").unwrap()).is_some(), "the units still load");
    }
    // No play file at all: the model is off and the load is quiet.
    let files: Vec<_> = fixtures(PLAY).into_iter().filter(|(n, _)| n != "fastphys").collect();
    let (db, warnings) = load(Profile::Sta1Classic, &files, &UNITS);
    assert!(db.a1_play_physics().is_none());
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_nonsense_borg_key_falls_back_to_its_default() {
    let mut files = fixtures(PLAY);
    files.push(("junkborg".into(), "[GameObjectClass]\nclassLabel = \"physics\"\nimpulseSpeed = 14\nborgPhysics = 1\nmaxRotationSpeed = abc\n".into()));
    files.push(("junk".into(), "[GameObjectClass]\nclassLabel = \"craft\"\nunitName = \"J\"\nphysicsFile = \"junkborg.odf\"\n".into()));
    let (db, warnings) = load(Profile::Sta1Classic, &files, &["junk"]);
    let b = db.unit(db.key("junk").unwrap()).unwrap().a1_borg.expect("borg");
    assert!(near(b.max_rotation_speed, 0.25));
    assert!(warnings.iter().any(|w| w.message.contains("maxRotationSpeed")));
}

#[test]
fn other_profiles_read_and_hash_nothing_of_it() {
    for profile in [Profile::Native, Profile::Fo4, Profile::Fo3, Profile::Sta2Classic] {
        let (with, _) = load(profile, &fixtures(PLAY), &UNITS);
        assert!(with.a1_play_physics().is_none(), "{profile:?}");
        assert!(with.unit(with.key("drone").unwrap()).unwrap().a1_borg.is_none(), "{profile:?}");
        // The same content with the play file and the Borg keys removed hashes alike.
        let files: Vec<_> = fixtures(PLAY)
            .into_iter()
            .filter(|(n, _)| n != "fastphys")
            .map(|(n, t)| (n, t.replace("borgPhysics = 1\n", "")))
            .collect();
        let (without, _) = load(profile, &files, &UNITS);
        assert_eq!(with.hash(), without.hash(), "{profile:?}");
    }
}

#[test]
fn only_armada_i_content_moves_and_only_by_the_new_entries() {
    let (a1, _) = load(Profile::Sta1Classic, &fixtures(PLAY), &UNITS);
    let (a1_off, _) = load(Profile::Sta1Classic, &fixtures("classLabel = \"physics\"\nimpulseSpeed = 0\n"), &UNITS);
    assert_ne!(a1.hash(), a1_off.hash());
    let names = |db: &ContentDb| db.digest().entries().iter().map(|e| e.key.clone()).collect::<Vec<_>>();
    let (with, without) = (names(&a1), names(&a1_off));
    assert!(with.iter().any(|k| k == "content:a1-play-physics"));
    assert!(!without.iter().any(|k| k == "content:a1-play-physics"));
    // Plain craft, structures: their own entries are the same either way.
    let entry = |db: &ContentDb, k: &str| db.digest().entries().iter().find(|e| e.key == k).map(|e| e.value.clone());
    for k in ["unit:ship_a", "unit:yard"] {
        assert_eq!(entry(&a1, k), entry(&a1_off, k), "{k}");
    }
}

#[test]
fn a_mod_changes_the_block_by_shipping_its_own_play_file() {
    // The loader answers the top layer's file; a different play file changes the block and the hash.
    let other = PLAY.replace("impulseSpeed = 12", "impulseSpeed = 30");
    let (a, _) = load(Profile::Sta1Classic, &fixtures(PLAY), &UNITS);
    let (b, _) = load(Profile::Sta1Classic, &fixtures(&other), &UNITS);
    assert!(near(b.a1_play_physics().unwrap().impulse_speed, 30.0));
    assert_ne!(a.hash(), b.hash());
    // The profile names the file; no other profile has one.
    assert_eq!(Profile::Sta1Classic.play_physics_file(), Some("fastphys"));
    assert!([Profile::Native, Profile::Fo4, Profile::Fo3, Profile::Sta2Classic].iter().all(|p| p.play_physics_file().is_none()));
}
