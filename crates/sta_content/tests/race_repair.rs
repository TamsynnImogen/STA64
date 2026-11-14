//! Authored race-repair metadata and digest tests; all ODFs are synthetic.

use sta_content::{ContentDb, Fixed, OdfIndex, Profile, RaceRepair, Races, Rules, Warning};
use sta_vfs::Vfs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("sta64-race-repair-{}-{id}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn write(&self, path: &str, contents: &str) {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn mount(layers: &[&Fixture]) -> Vfs {
    let mut vfs = Vfs::default();
    for (i, fixture) in layers.iter().enumerate() {
        vfs.push_dir(format!("layer{i}"), fixture.0.clone())
            .unwrap();
    }
    vfs
}

fn races(vfs: &Vfs) -> (Races, Vec<Warning>) {
    let (index, _) = OdfIndex::build(vfs);
    Races::load(vfs, &index)
}

fn db(vfs: &Vfs) -> (ContentDb, Vec<Warning>) {
    let (index, _) = OdfIndex::build(vfs);
    ContentDb::load_as(vfs, Profile::Native, &index).unwrap()
}

#[test]
fn layered_race_repairs_inherit_and_override_without_losing_absence() {
    let base = Fixture::new();
    base.write(
        "odf/races.odf",
        "numberOfRaces = 1\nrace0 = \"RaceA.odf\"\n",
    );
    base.write("odf/RaceA.odf", "name = \"shared\"\nrepairStrength = 1.25f\nrepairYellow = 2\nrepairRed = -0.5\ncrewYellowStatus = 4\ncrewRedStatus = 5\n");
    let child = Fixture::new();
    child.write(
        "odf/RACEA.odf",
        "#include \"racea.odf\"\nrepairYellow = 0\ncrewRedStatus = -1.5;\n",
    );
    let vfs = mount(&[&child, &base]);
    let (loaded, warnings) = races(&vfs);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        loaded.repair_rules().collect::<Vec<_>>(),
        [(
            "racea",
            RaceRepair {
                strength: Some(Fixed::from_f64(1.25)),
                yellow: Some(Fixed::ZERO),
                red: Some(Fixed::from_f64(-0.5)),
                crew_yellow_status: Some(Fixed::from_int(4)),
                crew_red_status: Some(Fixed::from_f64(-1.5)),
            }
        )]
    );
    let (content, warnings) = db(&vfs);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        content.rules().race_repair("RACEA.ODF"),
        loaded.repair_rules().next().map(|(_, r)| r)
    );
    assert_eq!(
        content.rules().race_repair("shared"),
        None,
        "lookup is by ODF stem only"
    );
}

#[test]
fn invalid_values_warn_and_valid_signed_values_survive() {
    let f = Fixture::new();
    f.write("odf/races.odf", "race0 = racea.odf\n");
    f.write("odf/racea.odf", "name = shared\nrepairStrength = banana\nrepairYellow = NaN\nrepairRed = 2147483648\ncrewYellowStatus = -2147483648\ncrewRedStatus = -2.25\n");
    let vfs = mount(&[&f]);
    let (loaded, warnings) = races(&vfs);
    let repair = loaded.repair_rules().next().unwrap().1;
    assert_eq!(repair.strength, None);
    assert_eq!(repair.yellow, None);
    assert_eq!(repair.red, None);
    assert_eq!(repair.crew_yellow_status, Some(Fixed::from_int(i32::MIN)));
    assert_eq!(repair.crew_red_status, Some(Fixed::from_f64(-2.25)));
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    for key in ["repairStrength", "repairYellow", "repairRed"] {
        assert!(
            warnings.iter().any(|w| w.file == "racea.odf"
                && w.message
                    .to_ascii_lowercase()
                    .contains(&key.to_ascii_lowercase())),
            "{key}: {warnings:?}"
        );
    }
    let (_, db_warnings) = db(&vfs);
    for key in ["repairStrength", "repairYellow", "repairRed"] {
        assert!(
            db_warnings.iter().any(|w| w.file == "racea.odf"
                && w.message
                    .to_ascii_lowercase()
                    .contains(&key.to_ascii_lowercase())),
            "{key}: {db_warnings:?}"
        );
    }
}

#[test]
fn bare_content_without_races_catalogue_keeps_quiet_fallback() {
    let fixture = Fixture::new();
    let (content, warnings) = db(&mount(&[&fixture]));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(content.rules().race_repair("any"), None);
}

#[test]
fn each_field_and_explicit_zero_change_digest_but_empty_metadata_does_not() {
    let plain = Rules::default();
    let digest = plain.digest_entries();
    assert_eq!(sta_content::fake::db().hash(), 0x9ef5_3881_3050_10f8);
    assert_eq!(
        plain
            .clone()
            .with_race_repairs(vec![("RaceA".into(), RaceRepair::default())])
            .digest_entries(),
        digest
    );
    for repair in [
        RaceRepair {
            strength: Some(Fixed::ZERO),
            ..Default::default()
        },
        RaceRepair {
            yellow: Some(Fixed::ZERO),
            ..Default::default()
        },
        RaceRepair {
            red: Some(Fixed::ZERO),
            ..Default::default()
        },
        RaceRepair {
            crew_yellow_status: Some(Fixed::ZERO),
            ..Default::default()
        },
        RaceRepair {
            crew_red_status: Some(Fixed::ZERO),
            ..Default::default()
        },
    ] {
        let entries = plain
            .clone()
            .with_race_repairs(vec![("RaceA.ODF".into(), repair)])
            .digest_entries();
        assert_ne!(entries, digest);
        assert!(entries.iter().any(|e| e.key == "race-repair:racea"));
    }
}

#[test]
fn builder_canonicalizes_order_case_and_duplicate_stems_first_wins() {
    let a = RaceRepair {
        strength: Some(Fixed::ONE),
        ..Default::default()
    };
    let b = RaceRepair {
        red: Some(Fixed::from_int(2)),
        ..Default::default()
    };
    let first = Rules::default().with_race_repairs(vec![
        ("B.ODF".into(), b),
        ("A.odf".into(), a),
        ("a".into(), b),
    ]);
    let reordered = Rules::default().with_race_repairs(vec![("a".into(), a), ("b".into(), b)]);
    assert_eq!(first.digest_entries(), reordered.digest_entries());
    assert_eq!(first.race_repair("a"), Some(a));
    assert_eq!(first.race_repair("B"), Some(b));
}

#[test]
fn distinct_odf_stems_with_the_same_race_key_remain_distinct() {
    let f = Fixture::new();
    f.write("odf/races.odf", "race0 = alpha.odf\nrace1 = beta.odf\n");
    f.write("odf/alpha.odf", "name = shared\nrepairStrength = 1\n");
    f.write("odf/beta.odf", "name = shared\nrepairStrength = 2\n");
    let (content, warnings) = db(&mount(&[&f]));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        content.rules().race_repair("alpha").unwrap().strength,
        Some(Fixed::ONE)
    );
    assert_eq!(
        content.rules().race_repair("beta").unwrap().strength,
        Some(Fixed::from_int(2))
    );
    assert_eq!(content.rules().race_repair("shared"), None);
    let entries = content.rules().digest_entries();
    assert!(entries.iter().any(|e| e.key == "race-repair:alpha"));
    assert!(entries.iter().any(|e| e.key == "race-repair:beta"));
}
