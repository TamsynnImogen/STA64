//! Synthetic starting-personnel rules and an optional inspection of one mounted install.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sta_content::{ContentDb, OdfIndex, Profile, Races, Rules};
use sta_vfs::Vfs;

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("sta64-start-rules-{}-{id}", std::process::id()));
        std::fs::create_dir(&path).expect("unique fixture directory");
        Self(path)
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove own fixture");
    }
}

fn mount(layers: &[(&str, &Fixture)]) -> Vfs {
    let mut vfs = Vfs::default();
    for (name, fixture) in layers {
        vfs.push_dir((*name).into(), fixture.path().to_path_buf())
            .unwrap();
    }
    vfs
}

fn loaded(vfs: &Vfs) -> ContentDb {
    let (index, _) = OdfIndex::build(vfs);
    ContentDb::load_as(vfs, Profile::Native, &index).unwrap().0
}

const CLASSIC_HEADER: &str = r#"
    int SINGLE_PLAYER_OFFICER_LIMIT = 600;
    int MORE_OFFICERS = SINGLE_PLAYER_OFFICER_LIMIT + 200;
    int MP_OFFICER_LIST_SIZE = 4;
    Officer MP_OFFICER_LIMIT[4] = {
        "ol_normal", SINGLE_PLAYER_OFFICER_LIMIT,
        "ol_restricted", SINGLE_PLAYER_OFFICER_LIMIT / 2,
        "ol_more", MORE_OFFICERS,
        "ol_max", 999,
    };
"#;

#[test]
fn flat_officer_array_resolves_expressions_in_source_order() {
    let (rules, warnings) = Rules::from_header(CLASSIC_HEADER);
    assert!(warnings.is_empty(), "{warnings:?}");
    let choices: Vec<_> = rules
        .officer_limits
        .iter()
        .map(|choice| (choice.key.as_str(), choice.value))
        .collect();
    assert_eq!(
        choices,
        [
            ("ol_normal", 600),
            ("ol_restricted", 300),
            ("ol_more", 800),
            ("ol_max", 999)
        ]
    );
    assert_eq!(rules.officer_limit(0), Some(600));
    assert_eq!(rules.officer_limit(4), None);

    let (custom, warnings) = Rules::from_header("int MP_OFFICER_LIST_SIZE = 2; Officer MP_OFFICER_LIMIT[2] = {\"custom\", 0, \"normal\", 17};");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        custom
            .officer_limits
            .iter()
            .map(|choice| choice.value)
            .collect::<Vec<_>>(),
        [0, 17]
    );
    assert_eq!(custom.officer_limits[0].key, "custom");
}

#[test]
fn invalid_officer_arrays_fail_closed_without_scalar_fallback() {
    let cases = [
        ("count mismatch", "int MP_OFFICER_LIST_SIZE = 2; Officer MP_OFFICER_LIMIT[2] = {\"one\", 1};"),
        ("negative", "int MP_OFFICER_LIST_SIZE = 1; Officer MP_OFFICER_LIMIT[1] = {\"one\", -1};"),
        ("unknown", "int MP_OFFICER_LIST_SIZE = 1; Officer MP_OFFICER_LIMIT[1] = {\"one\", MISSING};"),
        ("cyclic", "int A = B + 1; int B = A - 1; int MP_OFFICER_LIST_SIZE = 1; Officer MP_OFFICER_LIMIT[1] = {\"one\", A};"),
        ("expression overflow", "int MP_OFFICER_LIST_SIZE = 1; Officer MP_OFFICER_LIMIT[1] = {\"one\", 9223372036854775807 + 1};"),
        ("officer maximum overflow", "int MP_OFFICER_LIST_SIZE = 1; Officer MP_OFFICER_LIMIT[1] = {\"one\", 2147483648};"),
    ];
    for (label, text) in cases {
        let text = format!("int SINGLE_PLAYER_OFFICER_LIMIT = 600; {text}");
        let (rules, warnings) = Rules::from_header(&text);
        assert!(rules.officer_limits.is_empty(), "{label}: {rules:?}");
        assert!(!warnings.is_empty(), "{label}: warning missing");
    }
    let (scalar, warnings) = Rules::from_header("int SINGLE_PLAYER_OFFICER_LIMIT = 600;");
    assert!(warnings.is_empty());
    assert_eq!(scalar.officer_limit(0), Some(600));
}

#[test]
fn fake_rules_are_empty_and_numeric_limits_change_the_digest() {
    let fake = sta_content::fake::db();
    assert_eq!(fake.rules(), &Rules::default());
    let (rules, warnings) = Rules::from_header(CLASSIC_HEADER);
    assert!(warnings.is_empty());
    let configured = fake.clone().with_rules(rules.clone());
    assert_ne!(configured.hash(), fake.hash());
    let mut numeric = rules.clone();
    numeric.officer_limits[0].value += 1;
    assert_ne!(fake.clone().with_rules(numeric).hash(), configured.hash());
    let mut labels = rules;
    labels.officer_limits[0].key = "translated_custom_label".into();
    assert_eq!(fake.with_rules(labels).hash(), configured.hash());
}

#[test]
fn explicit_self_include_resolves_lower_header_and_overrides_it() {
    let base = Fixture::new();
    base.write("RTS_CFG.h", CLASSIC_HEADER);
    let upper = Fixture::new();
    upper.write(
        "RTS_CFG.h",
        "#include \"RTS_CFG.h\"\nint SINGLE_PLAYER_OFFICER_LIMIT = 700;\n",
    );
    let inherited = loaded(&mount(&[("upper", &upper), ("base", &base)]));
    assert_eq!(
        inherited
            .rules()
            .officer_limits
            .iter()
            .map(|c| c.value)
            .collect::<Vec<_>>(),
        [700, 350, 900, 999]
    );

    upper.write("RTS_CFG.h", "int SINGLE_PLAYER_OFFICER_LIMIT = 700;\n");
    let shadowed = loaded(&mount(&[("upper", &upper), ("base", &base)]));
    assert_eq!(
        shadowed
            .rules()
            .officer_limits
            .iter()
            .map(|c| c.value)
            .collect::<Vec<_>>(),
        [700]
    );
    assert_ne!(shadowed.hash(), inherited.hash());
}

#[test]
fn mounted_race_stock_and_crew_affect_rules_digest_but_caption_does_not() {
    let fixture = Fixture::new();
    fixture.write(
        "odf/races.odf",
        "numberOfRaces = 1\nrace0 = \"federation.odf\"\n",
    );
    let race = |display: &str, normal: i64, lots: i64, normal_crew: i64, lots_crew: Option<i64>| {
        format!(
        "name = \"federation\"\ndisplayKey = \"{display}\"\ninstantActionSlot = 0\nnormalDilithium = {normal}\nlotsDilithium = {lots}\nnormalCrew = {normal_crew}\n{}",
        lots_crew.map(|value| format!("lotsCrew = {value}\n")).unwrap_or_default(),
    )
    };
    fixture.write(
        "odf/federation.odf",
        &race("federation_caption", 100, 200, 0, Some(300)),
    );
    let vfs = mount(&[("base", &fixture)]);
    let (index, _) = OdfIndex::build(&vfs);
    let (races, warnings) = Races::load(&vfs, &index);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(races.get("federation").unwrap().normal_crew, Some(0));
    let original = loaded(&vfs);
    assert!(original.rules().officer_limits.is_empty());

    fixture.write(
        "odf/federation.odf",
        &race("new_caption", 100, 200, 0, Some(300)),
    );
    let cosmetic = loaded(&mount(&[("base", &fixture)]));
    assert_eq!(original.hash(), cosmetic.hash());

    for changed in [
        race("new_caption", 101, 200, 0, Some(300)),
        race("new_caption", 100, 201, 0, Some(300)),
        race("new_caption", 100, 200, 1, Some(300)),
        race("new_caption", 100, 200, 0, Some(301)),
        race("new_caption", 100, 200, 0, None),
    ] {
        fixture.write("odf/federation.odf", &changed);
        let db = loaded(&mount(&[("base", &fixture)]));
        assert_ne!(
            original.hash(),
            db.hash(),
            "numeric stock or optional crew changed"
        );
    }
}

#[test]
fn selected_install_reports_exact_mounted_personnel_rules() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
        return;
    };
    let active_mod = std::env::var("STA64_TEST_MOD")
        .ok()
        .filter(|name| !name.is_empty());
    let vfs = Vfs::mount_install(Path::new(&root), active_mod.as_deref())
        .expect("selected install must mount");
    let (index, _) = OdfIndex::build(&vfs);
    let (db, warnings) = ContentDb::load(&vfs, &index).expect("selected content must load");
    let (races, race_warnings) = Races::load(&vfs, &index);
    eprintln!(
        "profile={:?} mod={active_mod:?} rules={:?}",
        db.profile(),
        db.rules().officer_limits
    );
    eprintln!("rule warnings={warnings:?}; race warnings={race_warnings:?}");
    for race in races.iter() {
        eprintln!(
            "race={} normal={:?} lots={:?} crew={:?}/{:?}",
            race.name, race.stock, race.lots_stock, race.normal_crew, race.lots_crew
        );
    }
    if db.profile() == Profile::Sta2Classic {
        let values: Vec<_> = db
            .rules()
            .officer_limits
            .iter()
            .map(|limit| limit.value)
            .collect();
        assert_eq!(values, [600, 300, 800, 999]);
        if let Some(federation) = races
            .playable()
            .into_iter()
            .find(|race| race.stem == "federation")
        {
            assert_eq!(federation.normal_crew, Some(1200));
            assert_eq!(federation.lots_crew, Some(30000));
        }
    }
}
