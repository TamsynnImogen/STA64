use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sta_bzn::a1::{AiPath, Header, MapData};
use sta_content::{ContentDb, Fixed, MapDef, OdfIndex, Vec3, fake};
use sta_vfs::Vfs;
use sta_vfs::sta_write::{Compression, Content};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "sta_selected_map_{}_{}_{}",
            std::process::id(),
            stamp,
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::create_dir(path.join("Data")).unwrap();
        Self(path)
    }

    fn write(&self, relative: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }

    fn archive(&self, relative: &str, items: Vec<(String, Content)>) {
        self.write(relative, &sta_vfs::sta_write::pack(items, Compression::None).unwrap());
    }

    fn mount(&self, active_mod: Option<&str>) -> (Vfs, OdfIndex) {
        let vfs = Vfs::mount_install(&self.0, active_mod).unwrap();
        let (index, _) = OdfIndex::build(&vfs);
        (vfs, index)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn path(label: &str, first: [f32; 3]) -> AiPath {
    AiPath {
        reference: 0,
        label: label.into(),
        points: vec![first],
        path_type: 0,
        asteroid_belt_speed: 0.0,
    }
}

fn source_map() -> sta_bzn::Map {
    let mut map = sta_bzn::Map {
        version: 2051,
        binary: true,
        mission_type: "EmptyMission".into(),
        description: "Source scene".into(),
        file_name: "internal_wrong.bzn".into(),
        min_extent: [0.0, 0.0, 0.0],
        size: [100.0, 100.0, 100.0],
        ..Default::default()
    };
    map.objects = vec![
        sta_bzn::MapObject {
            odf: "fx_cruiser".into(),
            entity_id: 1,
            team: 1,
            transform: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 10.0, 2.0, 20.0],
            ..Default::default()
        },
        sta_bzn::MapObject {
            odf: "fx_moon".into(),
            entity_id: 2,
            team: 0,
            transform: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 90.0, 0.0, 80.0],
            ..Default::default()
        },
    ];
    map.a1 = Some(MapData {
        header: Header {
            version: 2051,
            save_game_desc: [0; 128],
            binary_save: true,
            metadata_offset: 0,
        },
        terrain_name: String::new(),
        source_bounds: ([0.0; 3], [100.0; 3]),
        resized: false,
        declared_object_count: 2,
        object_refs: vec![0, 0],
        initial_positions: vec![[10.0, 2.0, 20.0], [90.0, 0.0, 80.0]],
        mission_title: [0; 64],
        mission_reference: 0,
        paths: vec![
            path("Camera_5", [25.0, 1.0, 25.0]),
            path("Camera_2", [0.0, -7.25, 50.0]),
            path("Camera_1", [50.0, 12.5, -10.0]),
            path("Camera_4", [75.0, 1.0, 75.0]),
            path("Camera_3", [25.0, 1.0, 75.0]),
        ],
        areas: Vec::new(),
    });
    map
}

fn convert(map: &sta_bzn::Map, selected: &str, vfs: &Vfs, index: &OdfIndex) -> sta_content::map::Conversion {
    MapDef::convert_bzn_named_in(map, selected, &fake::db(), vfs, index)
}

fn assert_source_scene(converted: &sta_content::map::Conversion, source: &sta_bzn::Map) {
    assert_eq!(converted.counts.total(), source.objects.len());
    assert_eq!(converted.map.objects.len(), 2);
    assert_eq!(converted.map.objects[0].odf, "fx_cruiser");
    assert_eq!(converted.map.objects[1].odf, "fx_moon");
}

#[test]
fn selected_nested_name_controls_companion_and_precise_starts_without_source_mutation() {
    let root = TempRoot::new();
    root.write(
        "Data/BzN/CusTom/ReNaMeD.MDF",
        b"Start1 = 999 999\r\nStartLocations = 2\r\n",
    );
    root.write("Data/bzn/internal_wrong.mdf", b"StartLocations = 9\n");
    let (vfs, index) = root.mount(None);
    let source = source_map();
    let before = source.clone();
    let converted = convert(&source, "custom/renamed", &vfs, &index);
    assert_eq!(source, before);
    assert_source_scene(&converted, &source);
    assert!(converted.warnings.is_empty(), "{:?}", converted.warnings);
    assert_eq!(converted.map.name, "custom/renamed");
    assert_eq!(converted.map.starts.len(), 2);
    assert_eq!(converted.map.starts[0].slot, 0);
    assert_eq!(
        converted.map.starts[0].pos,
        Vec3 {
            x: Fixed::from_int(50),
            y: Fixed::from_f64(12.5),
            z: Fixed::from_int(-10)
        }
    );
    assert_eq!(converted.map.starts[0].heading, (Fixed::ZERO, Fixed::ONE));
    assert_eq!(converted.map.starts[1].slot, 1);
    assert_eq!(
        converted.map.starts[1].pos,
        Vec3 {
            x: Fixed::ZERO,
            y: Fixed::from_f64(-7.25),
            z: Fixed::from_int(50)
        }
    );
    assert_eq!(converted.map.starts[1].heading, (Fixed::ONE, Fixed::ZERO));

    let case_selected = convert(&source, "CusTom/ReNamed", &vfs, &index);
    assert_eq!(case_selected.map.starts, converted.map.starts);
    assert_eq!(
        convert(&source, "CusTom\\ReNamed", &vfs, &index).map.starts,
        converted.map.starts
    );

    let mut unknown = source.clone();
    unknown.objects[0].odf = "not_a_definition".into();
    let warned = convert(&unknown, "custom/renamed", &vfs, &index);
    assert!(warned.warnings.iter().any(|w| w.message.contains("custom/renamed:")));
    assert!(warned.warnings.iter().all(|w| !w.message.contains("internal_wrong:")));
}

#[test]
fn missing_bad_unreadable_and_unresolved_mdf_never_publish_partial_starts() {
    let root = TempRoot::new();
    let (vfs, index) = root.mount(None);
    let source = source_map();
    let missing = convert(&source, "custom/renamed", &vfs, &index);
    assert_source_scene(&missing, &source);
    assert!(missing.map.starts.is_empty());
    assert!(missing.warnings.is_empty());

    for bytes in [
        &b"StartLocations = x2\n"[..], // unparsable: eight
        &b"StartLocations = 9\n"[..], // out of range: the native reader falls back to eight, Camera_6 is absent
        &b"StartLocations = 6\n"[..], // Camera_6 is absent after five valid slots
        &b"nothing here\n"[..],       // no count: eight again
    ] {
        root.write("Data/bzn/custom/renamed.mdf", bytes);
        let (vfs, index) = root.mount(None);
        let result = convert(&source, "custom/renamed", &vfs, &index);
        assert_source_scene(&result, &source);
        assert!(result.map.starts.is_empty(), "{bytes:?}");
        assert!(
            result.warnings.iter().any(|w| w.file == "bzn/custom/renamed.mdf"),
            "{:?}",
            result.warnings
        );
    }
    let file = root.write("Data/bzn/custom/renamed.mdf", b"StartLocations = 2\n");
    let (vfs, index) = root.mount(None);
    fs::remove_file(file).unwrap(); // the mounted loose source becomes unreadable
    let unreadable = convert(&source, "custom/renamed", &vfs, &index);
    assert_source_scene(&unreadable, &source);
    assert!(unreadable.map.starts.is_empty());
    assert!(unreadable.warnings.iter().any(|w| w.file == "bzn/custom/renamed.mdf"));
}

#[test]
fn packed_loose_mod_and_whiteout_companions_follow_vfs_precedence() {
    let root = TempRoot::new();
    let mdf = "bzn/custom/renamed.mdf";
    root.archive(
        "Data/base.sta",
        vec![(mdf.into(), Content::Data(b"StartLocations = 2\n".to_vec()))],
    );
    let source = source_map();
    let (vfs, index) = root.mount(None);
    assert_eq!(convert(&source, "custom/renamed", &vfs, &index).map.starts.len(), 2);

    root.write("Data/bzn/custom/renamed.mdf", b"StartLocations = 3\n");
    let (vfs, index) = root.mount(None);
    assert_eq!(convert(&source, "custom/renamed", &vfs, &index).map.starts.len(), 3);

    root.write("Data/Mods/Parent/info.ini", b"[Mod]\n");
    root.archive(
        "Data/Mods/Parent/parent.sta",
        vec![(mdf.into(), Content::Data(b"StartLocations = 4\n".to_vec()))],
    );
    root.write("Data/Mods/Child/info.ini", b"[Mod]\nParentMod=Parent\n");
    let child = root.write("Data/Mods/Child/bzn/custom/renamed.mdf", b"StartLocations = 5\n");
    let (vfs, index) = root.mount(Some("Child"));
    assert_eq!(convert(&source, "custom/renamed", &vfs, &index).map.starts.len(), 5);

    root.write("Data/Mods/Child/bzn/custom/renamed.mdf", b"StartLocations = 9\n");
    let (vfs, index) = root.mount(Some("Child"));
    let bad_child = convert(&source, "custom/renamed", &vfs, &index);
    assert!(
        bad_child.map.starts.is_empty(),
        "invalid child must not fall back to the valid parent"
    );
    assert!(bad_child.warnings.iter().any(|w| w.file == mdf));

    fs::remove_file(child).unwrap();
    let (vfs, index) = root.mount(Some("Child"));
    assert_eq!(convert(&source, "custom/renamed", &vfs, &index).map.starts.len(), 4);

    root.archive("Data/Mods/Child/hide.sta", vec![(mdf.into(), Content::Whiteout)]);
    let (vfs, index) = root.mount(Some("Child"));
    let hidden = convert(&source, "custom/renamed", &vfs, &index);
    assert_source_scene(&hidden, &source);
    assert!(hidden.map.starts.is_empty());
    assert!(hidden.warnings.is_empty());
}

#[test]
fn a2_and_existing_unnamed_converters_ignore_same_named_mdf() {
    let root = TempRoot::new();
    root.write("Data/bzn/custom/renamed.mdf", b"StartLocations = 2\n");
    let (vfs, index) = root.mount(None);
    let source = source_map();
    let db = fake::db();
    let old = MapDef::convert_bzn_in(&source, &db, &vfs, &index);
    assert!(old.map.starts.is_empty());
    assert_eq!(old.map.name, "internal_wrong");
    assert_eq!(old.counts.total(), source.objects.len());
    let (simple, warnings) = MapDef::from_bzn(&source, &db);
    assert!(simple.starts.is_empty());
    assert_eq!(simple, old.map);
    assert_eq!(warnings, old.warnings);

    let mut a2 = source.clone();
    a2.a1 = None;
    a2.mission_type = "mp-test".into();
    a2.start_count = 1;
    a2.start_locs = vec![
        sta_bzn::StartLoc {
            pos: [50.0, 0.0, 50.0],
            ..Default::default()
        },
        sta_bzn::StartLoc {
            pos: [10.0, 3.0, 20.0],
            ..Default::default()
        },
    ];
    let existing = MapDef::convert_bzn_in(&a2, &db, &vfs, &index);
    let named = MapDef::convert_bzn_named_in(&a2, "custom/renamed", &db, &vfs, &index);
    assert_eq!(named.map.starts, existing.map.starts);
    assert_eq!(named.counts, existing.counts);
    assert_eq!(named.map.starts.len(), 1);
    assert_eq!(named.map.starts[0].slot, 0);
    let mut expected = existing;
    expected.map.name = "custom/renamed".into();
    assert_eq!(named, expected);
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

#[test]
#[ignore = "requires STA64_TEST_A1_INSTALL pointing at user-owned original A1 root"]
fn original_a1_install_selected_stems_account_for_maps_objects_and_starts() {
    let root = std::env::var_os("STA64_TEST_A1_INSTALL").expect("set STA64_TEST_A1_INSTALL");
    let vfs = Vfs::mount_install(Path::new(&root), None).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let paths: Vec<_> = vfs.list("bzn").into_iter().filter(|p| p.ends_with(".bzn")).collect();
    assert_eq!(paths.len(), 49);
    let mut input_hashes = Vec::new();
    let mut mdf_count = 0;
    let mut starts = 0;
    let mut source_objects = 0;
    for path in &paths {
        let bytes = vfs.read(path).unwrap().unwrap();
        input_hashes.push((path.clone(), hash_bytes(&bytes)));
        let bzn = sta_bzn::parse(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
        let stem = path.strip_prefix("bzn/").unwrap().strip_suffix(".bzn").unwrap();
        let companion = format!("bzn/{stem}.mdf");
        let has_mdf = if let Some(result) = vfs.read(&companion) {
            let mdf = result.unwrap();
            input_hashes.push((companion, hash_bytes(&mdf)));
            mdf_count += 1;
            true
        } else {
            false
        };
        let converted = MapDef::convert_bzn_named_in(&bzn, stem, &db, &vfs, &index);
        assert_eq!(converted.counts.total(), bzn.objects.len(), "{path}");
        if !has_mdf {
            assert!(converted.map.starts.is_empty(), "{path}");
        }
        starts += converted.map.starts.len();
        source_objects += bzn.objects.len();
    }
    assert_eq!((mdf_count, starts, source_objects), (28, 113, 17_595));
    for (path, original_hash) in input_hashes {
        assert_eq!(
            hash_bytes(&vfs.read(&path).unwrap().unwrap()),
            original_hash,
            "source changed: {path}"
        );
    }
    eprintln!(
        "A1 selected maps: {} BZNs, {mdf_count} MDFs, {starts} starts, {source_objects} source objects",
        paths.len()
    );
}

#[test]
#[ignore = "requires STA64_TEST_CONTROL_DIR, or STA64_TEST_CONTROL_INSTALL with optional STA64_TEST_CONTROL_MOD"]
fn installed_a2_format_named_conversion_preserves_existing_map_results() {
    let vfs = if let Some(dir) = std::env::var_os("STA64_TEST_CONTROL_DIR") {
        let mut vfs = Vfs::default();
        vfs.push_dir("root".into(), dir.into()).unwrap();
        vfs
    } else {
        let root = std::env::var_os("STA64_TEST_CONTROL_INSTALL").expect("set a control directory or install");
        let active = std::env::var("STA64_TEST_CONTROL_MOD").unwrap_or_default();
        Vfs::mount_install(Path::new(&root), Some(active.as_str()).filter(|m| !m.is_empty())).unwrap()
    };
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let mut maps = 0;
    let mut starts = 0;
    for file in vfs.list("bzn").into_iter().filter(|p| p.ends_with(".bzn")) {
        let bytes = vfs.read(&file).unwrap().unwrap();
        let before = hash_bytes(&bytes);
        let source = sta_bzn::parse(&bytes).unwrap_or_else(|e| panic!("{file}: {e}"));
        assert!(source.a1.is_none(), "control must use the existing A2 format: {file}");
        let name = file.strip_prefix("bzn/").unwrap().strip_suffix(".bzn").unwrap();
        let mut old = MapDef::convert_bzn_in(&source, &db, &vfs, &index);
        old.map.name = name.into(); // the client applied the selected name after conversion
        let named = MapDef::convert_bzn_named_in(&source, name, &db, &vfs, &index);
        assert_eq!(named.map, old.map, "changed map result: {file}");
        assert_eq!(named.counts, old.counts, "changed object accounting: {file}");
        assert_eq!(
            named.warnings.len(),
            old.warnings.len(),
            "changed warning count: {file}"
        );
        assert_eq!(
            hash_bytes(&vfs.read(&file).unwrap().unwrap()),
            before,
            "changed input: {file}"
        );
        maps += 1;
        starts += named.map.starts.len();
    }
    assert!(maps > 0, "control contains no maps");
    eprintln!("A2-format control: {maps} maps, {starts} starts; selected conversion preserves every map result");
}

/// CON-42: the native reader's malformed-file and default-eight rules, and the
/// configuration-backed fallback for a team without a `Camera_i` path.
#[test]
fn native_count_rules_and_missing_path_fallback() {
    let root = TempRoot::new();
    let source = source_map();
    // A repeated key takes the first; a bad value takes eight.
    root.write("Data/bzn/custom/renamed.mdf", b"startlocations=2\nStartLocations = 5\n");
    let (vfs, index) = root.mount(None);
    assert_eq!(convert(&source, "custom/renamed", &vfs, &index).map.starts.len(), 2);
    // With RTS_CFG.h, the three teams beyond the five paths start at the centre XZ, mid-combat-height Y.
    root.write("Data/RTS_CFG.h", b"float SHIP_COMBAT_FLOOR = 10.0;\nfloat SHIP_COMBAT_CEILING = 67.0;\n");
    root.write("Data/bzn/custom/renamed.mdf", b"StartLocations = banana\n");
    let (vfs, index) = root.mount(None);
    let result = convert(&source, "custom/renamed", &vfs, &index);
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    assert_eq!(result.map.starts.len(), 8);
    for start in &result.map.starts[5..] {
        assert_eq!((start.pos.x, start.pos.y, start.pos.z), (Fixed::from_int(50), Fixed::from_f64(38.5), Fixed::from_int(50)));
    }
    assert_eq!(result.map.starts[0].pos.y, Fixed::from_f64(12.5)); // a real path keeps its own point
}
