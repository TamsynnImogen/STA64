//! CON-53: Armada I resource moons are resource nodes. The ODFs here are authored; the
//! install-gated test (STA64_TEST_INSTALL, an Armada I install; skipped when unset) asserts the
//! rule over the install's own ODFs.

use std::path::Path;

use sta_content::{ContentDb, OdfIndex, Overrides, Profile, odf::load_with};
use sta_vfs::Vfs;

const MOON: &str = "classLabel = \"scrap\"\nunitName = \"Moon\"\nmaxHealth = 900\n";
const DEBRIS: &str = "classLabel = \"scrap\"\nunitName = \"Debris\"\nscrapValue = 7\n";
const ROCK: &str = "classLabel = \"asteroid\"\nunitName = \"Rock\"\n";
const KEYED: &str = "classLabel = \"scrap\"\nunitName = \"Rich\"\nresource = \"latinum\"\n";

fn load(profile: Profile, moon: &str) -> ContentDb {
    let stems: Vec<String> = ["moon", "debris", "rock", "keyed"].map(str::to_owned).to_vec();
    load_with(profile, &Overrides::default(), &stems, &mut |name, version| {
        match (name.to_ascii_lowercase().trim_end_matches(".odf"), version) {
            ("moon", 0) => Some(moon.to_owned()),
            ("debris", 0) => Some(DEBRIS.to_owned()),
            ("rock", 0) => Some(ROCK.to_owned()),
            ("keyed", 0) => Some(KEYED.to_owned()),
            _ => None,
        }
    })
    .unwrap()
    .0
}

fn resource(db: &ContentDb, name: &str) -> Option<u8> {
    db.unit(db.key(name).unwrap()).unwrap().resource
}

#[test]
fn only_the_armada_i_profile_reads_a_moon_without_a_resource_key() {
    for profile in Profile::ALL {
        let db = load(profile, MOON);
        let a1 = profile == Profile::Sta1Classic;
        assert_eq!(resource(&db, "moon"), a1.then_some(0), "{profile:?}");
        assert_eq!(resource(&db, "debris"), None, "{profile:?}: scrap pieces stay props");
        assert_eq!(resource(&db, "rock"), None, "{profile:?}");
        // An authored `resource` key is never overridden.
        assert_eq!(resource(&db, "keyed"), Some(1), "{profile:?}");
    }
}

#[test]
fn the_digest_moves_for_armada_i_only() {
    for profile in Profile::ALL {
        let moon = load(profile, MOON);
        // The same node told apart from a moon only by a scrap key.
        let debris = load(profile, &format!("{MOON}scrapValue = 3\n"));
        if profile == Profile::Sta1Classic {
            assert_ne!(moon.hash(), debris.hash(), "{profile:?}");
        } else {
            assert_eq!(moon.hash(), debris.hash(), "{profile:?}");
        }
    }
}

#[test]
fn install_scrap_class_units_follow_the_rule() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
    let vfs = Vfs::mount_install(Path::new(&root), None).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    if db.profile() != Profile::Sta1Classic {
        eprintln!("{root}: profile {:?}, not an Armada I install; skipped", db.profile());
        return;
    }
    let (mut nodes, mut pieces) = (0, 0);
    for u in db.units().filter(|u| u.class.0 == "scrap") {
        let piece = u.raw.raw("scrapValue").is_some() || u.raw.raw("scrapCost").is_some();
        assert_eq!(u.resource.is_some(), !piece || u.raw.raw("resource").is_some(), "{}", u.odf);
        nodes += u.resource.is_some() as usize;
        pieces += piece as usize;
    }
    eprintln!("{nodes} scrap-class nodes, {pieces} scrap pieces");
    assert!(nodes > 0 && pieces > 0, "{nodes} nodes, {pieces} pieces");
}
