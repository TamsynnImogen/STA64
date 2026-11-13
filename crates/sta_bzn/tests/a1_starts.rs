use std::collections::{HashMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use sta_bzn::a1::{AiPath, Header, MapData, StartError, mdf_start_count, native_mdf_start_count, player_start_positions, player_start_positions_with_fallback};

// Exact binary32 payload of 2blue's first Camera_1 point, not MDF preview pixels.
const BLUE_START: [f32; 3] = [f32::from_bits(0x45b9_e866), 38.5, f32::from_bits(0x43c8_a600)];

fn path(label: &str, points: &[[f32; 3]]) -> AiPath {
    AiPath {
        reference: 0,
        label: label.into(),
        points: points.to_vec(),
        path_type: 0,
        asteroid_belt_speed: 0.0,
    }
}

fn map(paths: Vec<AiPath>) -> MapData {
    MapData {
        header: Header {
            version: 2050,
            save_game_desc: [0; 128],
            binary_save: true,
            metadata_offset: 0,
        },
        terrain_name: String::new(),
        source_bounds: ([0.0; 3], [1.0; 3]),
        resized: false,
        declared_object_count: 0,
        object_refs: Vec::new(),
        initial_positions: Vec::new(),
        mission_title: [0; 64],
        mission_reference: 0,
        paths,
        areas: Vec::new(),
    }
}

#[test]
fn canonical_mdf_count_accepts_ascii_spacing_line_endings_and_unrelated_bytes() {
    for (source, expected) in [
        (&b"StartLocations = 2"[..], 2),
        (&b"\t StartLocations\t=\t8 \t\n"[..], 8),
        (&b"Start1 = 999\rStartLocations = 3\r"[..], 3),
        (&b"Start2 = 12\r\nStartLocations=4\r\n"[..], 4),
        (&b"\xff\xfe unrelated\nStartLocations = 5\n"[..], 5),
    ] {
        assert_eq!(mdf_start_count(source), Ok(expected), "{source:?}");
    }
}

#[test]
fn canonical_mdf_count_distinguishes_missing_duplicate_invalid_and_out_of_range() {
    for source in [
        &b""[..],
        &b"Start1 = 2\n"[..],
        &b"StartLocationsExtra = 3\n"[..],
        &b"startlocations = 3\n"[..],
    ] {
        assert_eq!(mdf_start_count(source), Err(StartError::MissingCount), "{source:?}");
    }
    for source in [
        &b"StartLocations = "[..],
        &b"StartLocations = +2"[..],
        &b"StartLocations = -2"[..],
        &b"StartLocations = 2.0"[..],
        &b"StartLocations = 2x"[..],
        &b"StartLocations = \"2\""[..],
        &b"StartLocations = 2 // comment"[..],
        &b"StartLocations = 2 3"[..],
        &b"StartLocations = 4294967296"[..],
    ] {
        assert_eq!(mdf_start_count(source), Err(StartError::InvalidCount), "{source:?}");
    }
    for (count, error) in [
        ("0", StartError::UnsupportedCount(0)),
        ("1", StartError::UnsupportedCount(1)),
        ("9", StartError::UnsupportedCount(9)),
    ] {
        assert_eq!(
            mdf_start_count(format!("StartLocations = {count}").as_bytes()),
            Err(error)
        );
    }
    assert_eq!(
        mdf_start_count(b"StartLocations = 2\nStartLocations = 2\n"),
        Err(StartError::DuplicateCount)
    );
    assert_eq!(
        mdf_start_count(b"StartLocations = 2\r\nStartLocations = 8\r\n"),
        Err(StartError::DuplicateCount)
    );
}

#[test]
fn camera_lookup_preserves_precise_xyz_order_first_match_and_source() {
    let precise = BLUE_START;
    let original = map(vec![
        path("Camera_2", &[[-100.25, -7.5, 9000.75], [1.0, 2.0, 3.0]]),
        path("camera_1", &[[2.0, 2.0, 2.0]]),
        path("Camera_1", &[precise, [0.0, 0.0, 0.0]]),
        path("Camera_1", &[[99.0, 99.0, 99.0]]),
    ]);
    let before = original.clone();
    assert_eq!(
        player_start_positions(&original, 2),
        Ok(vec![precise, [-100.25, -7.5, 9000.75]])
    );
    assert_eq!(original, before, "lookup must not mutate path order, labels or points");
}

#[test]
fn camera_lookup_rejects_missing_empty_nonfinite_and_unsupported_counts() {
    let one = map(vec![path("Camera_1", &[[1.0, 2.0, 3.0]])]);
    for count in [0, 1, 9, u8::MAX] {
        assert_eq!(
            player_start_positions(&one, count),
            Err(StartError::UnsupportedCount(u32::from(count)))
        );
    }
    assert_eq!(player_start_positions(&one, 2), Err(StartError::MissingCamera(2)));
    let empty_first = map(vec![
        path("Camera_1", &[]),
        path("Camera_1", &[[1.0, 2.0, 3.0]]),
        path("Camera_2", &[[4.0, 5.0, 6.0]]),
    ]);
    assert_eq!(player_start_positions(&empty_first, 2), Err(StartError::EmptyCamera(1)));
    for point in [
        [f32::NAN, 2.0, 3.0],
        [1.0, f32::INFINITY, 3.0],
        [1.0, 2.0, f32::NEG_INFINITY],
    ] {
        let data = map(vec![path("Camera_1", &[[0.0, 0.0, 0.0]]), path("Camera_2", &[point])]);
        assert_eq!(player_start_positions(&data, 2), Err(StartError::NonfiniteCamera(2)));
    }
    let later_nonfinite = map(vec![
        path("Camera_1", &[[0.0, 0.0, 0.0], [f32::NAN, 0.0, 0.0]]),
        path("Camera_2", &[[0.0, 0.0, 0.0]]),
    ]);
    assert_eq!(
        player_start_positions(&later_nonfinite, 2),
        Ok(vec![[0.0; 3], [0.0; 3]])
    );
}

#[test]
fn eight_slots_resolve_in_numeric_order_without_neutral_entry() {
    let data = map((1..=8)
        .rev()
        .map(|i| path(&format!("Camera_{i}"), &[[i as f32, -(i as f32), i as f32 + 0.25]]))
        .collect());
    let positions = player_start_positions(&data, 8).unwrap();
    assert_eq!(positions.len(), 8);
    for i in 1..=8 {
        assert_eq!(positions[i - 1], [i as f32, -(i as f32), i as f32 + 0.25]);
    }
}

fn hash_file(path: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    std::fs::read(path).unwrap().hash(&mut hasher);
    hasher.finish()
}

#[test]
#[ignore = "requires STA64_TEST_A1_BZN_DIR pointing at user-owned original A1 maps and MDFs"]
fn installed_a1_corpus_resolves_every_announced_camera_without_mutating_inputs() {
    let root = PathBuf::from(std::env::var_os("STA64_TEST_A1_BZN_DIR").expect("set STA64_TEST_A1_BZN_DIR"));
    let mut bzns = HashMap::new();
    let mut mdfs = HashMap::new();
    let mut hashes = Vec::new();
    for entry in std::fs::read_dir(&root).unwrap() {
        let path = entry.unwrap().path();
        let Some(ext) = path.extension() else { continue };
        let Some(stem) = path.file_stem() else { continue };
        let stem = stem.to_string_lossy().to_ascii_lowercase();
        if ext.eq_ignore_ascii_case("bzn") {
            hashes.push((path.clone(), hash_file(&path)));
            bzns.insert(stem, path);
        } else if ext.eq_ignore_ascii_case("mdf") {
            hashes.push((path.clone(), hash_file(&path)));
            mdfs.insert(stem, path);
        }
    }
    assert_eq!(bzns.len(), 49, "unexpected BZN corpus under {}", root.display());
    assert_eq!(mdfs.len(), 28, "unexpected MDF corpus under {}", root.display());
    for stem in mdfs.keys() {
        assert!(bzns.contains_key(stem), "MDF {stem} has no matching BZN");
    }
    let mut total_objects = 0;
    let mut total_starts = 0;
    let mut saw_2blue = false;
    for (stem, path) in &bzns {
        let map = sta_bzn::parse(&std::fs::read(path).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let a1 = map.a1.as_ref().expect("A1 body");
        assert_eq!(map.objects.len(), a1.declared_object_count, "{}", path.display());
        total_objects += map.objects.len();
        if let Some(mdf) = mdfs.get(stem) {
            let count =
                mdf_start_count(&std::fs::read(mdf).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", mdf.display()));
            // The native reader agrees with the strict one on every original companion.
            assert_eq!(native_mdf_start_count(Some(&std::fs::read(mdf).unwrap())), count, "{}", mdf.display());
            let starts = player_start_positions(a1, count).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert_eq!(starts.len(), usize::from(count), "{}", path.display());
            total_starts += starts.len();
            if stem == "2blue" {
                assert_eq!(starts[0].map(f32::to_bits), BLUE_START.map(f32::to_bits));
                saw_2blue = true;
            }
        }
    }
    assert!(saw_2blue, "2blue MDF/BZN pair missing");
    assert!(total_objects > 0 && total_starts > 0);
    for (path, original) in hashes {
        assert_eq!(hash_file(&path), original, "source changed: {}", path.display());
    }
    eprintln!(
        "A1 corpus: {} BZNs, {} MDFs, {total_objects} objects, {total_starts} resolved starts",
        bzns.len(),
        mdfs.len()
    );
}

#[test]
fn native_reader_never_fails_and_defaults_to_eight() {
    let cases: &[(Option<&[u8]>, u8)] = &[
        (None, 8),
        (Some(b""), 8),
        (Some(b"StartLocations = 4\n"), 4),
        (Some(b"startlocations=3"), 3),                    // case-insensitive, no spaces
        (Some(b"Start1 = 1 2\r\nStartLocations = 5\r\n"), 5),
        (Some(b"StartLocations = 1\n"), 8),               // below range
        (Some(b"StartLocations = 9\n"), 8),               // above range
        (Some(b"StartLocations = -3\n"), 8),
        (Some(b"StartLocations = 99999999999999999999\n"), 8),
        (Some(b"StartLocations = 2x\n"), 2),              // sscanf stops at the first non-digit
        (Some(b"StartLocations = x\n"), 8),               // sscanf fails: default
        (Some(b"StartLocations\nStartLocations = 6\n"), 6), // no '=': line skipped
        (Some(b"StartLocationsX = 7\n"), 7),              // prefix match
        (Some(b"StartLocations = 2\nStartLocations = 6\n"), 2), // first wins
        (Some(b"StartLocations =\n 4\n"), 4),            // LF before a blank continues the line
        (Some(b"StartLocations =\n4\n"), 8),             // no continuation: value is on the next line
        (Some(b"StartLocations = 3 trailing\n"), 3),
        (Some(b"StartLocations = 3\0StartLocations = 5"), 3),
        (Some(b"\0StartLocations = 5"), 8),               // C string ends at the first NUL
        (Some(b"Start Locations = 5\n"), 8),
        (Some(b"StartLocations = x\nStartLocations = 5\n"), 8), // the first matching line decides; no fall-through
        (Some(b"StartLocations =\n\t4\n"), 4),          // tab continuation
        (Some(b"StartLocations =\r\n 4\n"), 4),         // CR is whitespace; the LF before a blank continues the line
        (Some(b" StartLocations = 4\n"), 8),             // leading blank: key not at line start
        (Some(b"StartLocations = +4\n"), 4),
        (Some(b"StartLocations = + 4\n"), 8),            // sign then blank: %d fails
        (Some(b"StartLocations = - 4\n"), 8),
    ];
    for (source, expected) in cases {
        assert_eq!(native_mdf_start_count(*source), *expected, "{source:?}");
    }
}

#[test]
fn missing_camera_uses_the_fallback_but_invalid_paths_still_reject() {
    let path = |label: &str, p: [f32; 3]| AiPath { reference: 0, label: label.into(), points: vec![p], path_type: 0, asteroid_belt_speed: 0.0 };
    let map = map(vec![path("Camera_1", [1.0, 2.0, 3.0]), path("Camera_3", [f32::NAN, 0.0, 0.0])]);
    let fb = Some([5.0, 38.5, 5.0]);
    assert_eq!(player_start_positions_with_fallback(&map, 2, fb).unwrap(), vec![[1.0, 2.0, 3.0], [5.0, 38.5, 5.0]]);
    assert_eq!(player_start_positions_with_fallback(&map, 2, None), Err(StartError::MissingCamera(2)));
    assert_eq!(player_start_positions_with_fallback(&map, 3, fb), Err(StartError::NonfiniteCamera(3)));
}
