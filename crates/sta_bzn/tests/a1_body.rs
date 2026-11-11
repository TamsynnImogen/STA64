use sta_bzn::{a1::inspect_header, parse};

fn text(value: &str, width: usize) -> Vec<u8> {
    let mut bytes = value.as_bytes().to_vec();
    assert!(bytes.len() <= width);
    bytes.resize(width, 0);
    bytes
}

fn floats(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

struct Fixture {
    bytes: Vec<u8>,
    fields: Vec<(&'static str, usize)>,
}

impl Fixture {
    fn new(binary_save: bool) -> Self {
        let header = format!(
            "version [1] =\n2051\nsaveGameDesc = {}\nbinarySave [1] =\n{}\n",
            "A5".repeat(128),
            if binary_save { "true" } else { "false" }
        );
        Self {
            bytes: header.into_bytes(),
            fields: Vec::new(),
        }
    }

    fn add(&mut self, name: &'static str, kind: u8, bytes: &[u8]) {
        let at = self.bytes.len();
        self.fields.push((name, at));
        // Upper tag bytes are deliberately nonzero: only the low type byte is A1's type.
        self.bytes
            .extend_from_slice(&(0xA5_42_19_00_u32 | u32::from(kind)).to_le_bytes());
        self.bytes.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        self.bytes.extend_from_slice(bytes);
    }

    fn at(&self, name: &str) -> usize {
        self.fields.iter().find(|(field, _)| *field == name).unwrap().1
    }

    fn body(&mut self, resized: bool, objects: i32) {
        self.add("msn", 2, &text("mission_a.msn", 16));
        self.add("mission_save", 1, &[1]);
        self.add("terrain", 2, &text("terrain_a", 100));
        for (name, value) in [
            ("min_x", 0.0_f32),
            ("max_x", 100.0),
            ("min_y", -20.0),
            ("max_y", 40.0),
            ("min_z", -50.0),
            ("max_z", 150.0),
        ] {
            self.add(name, 5, &value.to_le_bytes());
        }
        self.add("resize", 1, &[u8::from(resized)]);
        if resized {
            for (name, value) in [
                ("new_min_x", -100.0_f32),
                ("new_max_x", 300.0),
                ("new_min_y", -999.0),
                ("new_max_y", 999.0),
                ("new_min_z", 100.0),
                ("new_max_z", 500.0),
            ] {
                self.add(name, 5, &value.to_le_bytes());
            }
        }
        for (name, value) in [("red", 0.25_f32), ("green", 0.5), ("blue", 0.75)] {
            self.add(name, 5, &value.to_le_bytes());
        }
        self.add("background", 2, &text("space.sod", 100));
        self.add("tech", 2, &text("stock.tt", 128));
        self.add("race", 2, &text("federation", 128));
        self.add("objects", 4, &objects.to_le_bytes());
    }

    fn object(&mut self, name: &'static str, odf: &str, id: i32, pos: [f32; 3], team: i32, reference: u32) {
        self.add(name, 7, &text(odf, 8));
        self.add("entity", 4, &id.to_le_bytes());
        self.add("position", 9, &floats(&pos));
        self.add("team", 4, &team.to_le_bytes());
        self.add("label", 2, &text("Authored label", 40));
        self.add("object_ref", 8, &reference.to_le_bytes());
        self.add(
            "transform",
            11,
            &floats(&[
                0.0, 1.0, 0.0, // right (orientation deliberately nonidentity)
                1.0, 0.0, 0.0, // up
                0.0, 0.0, -1.0, // front
                pos[0], pos[1], pos[2],
            ]),
        );
    }

    fn mission(&mut self, class: &str) {
        self.add("mission_class", 2, &text(class, 40));
        self.add("mission_ref", 8, &0xA0B0_C0D0_u32.to_le_bytes());
        self.add("script", 2, &text("one.dll", 64));
        self.add("rules", 2, &text("two.drl", 64));
        let mut title = text("A1 Mission", 64);
        title[11..15].copy_from_slice(b"TAIL"); // raw bytes after the first NUL survive
        self.add("title", 0, &title);
    }

    fn aoi(&mut self, object_ref: u32, team: i32, interesting: u8, inside: u8, value: i32, force: i32) {
        self.add("aoi_ref", 8, &object_ref.to_le_bytes());
        self.add("aoi_team", 4, &team.to_le_bytes());
        self.add("interesting", 1, &[interesting]);
        self.add("inside", 1, &[inside]);
        self.add("value", 4, &value.to_le_bytes());
        self.add("force", 4, &force.to_le_bytes());
    }

    fn path(&mut self, reference: u32, label: &str, points: &[[f32; 3]], path_type: i32, speed: f32) {
        self.add("path_ref", 0, &reference.to_le_bytes());
        self.add("path_label_len", 4, &(label.len() as i32).to_le_bytes());
        if !label.is_empty() {
            self.add("path_label", 2, label.as_bytes());
        }
        self.add("point_count", 4, &(points.len() as i32).to_le_bytes());
        let values: Vec<f32> = points.iter().flat_map(|p| *p).collect();
        self.add("points", 9, &floats(&values));
        self.add("path_type", 0, &path_type.to_le_bytes());
        self.add("path_speed", 5, &speed.to_le_bytes());
    }
}

fn populated(resized: bool, mission: &str) -> Fixture {
    let mut f = Fixture::new(true);
    f.body(resized, 2);
    f.object("first_project", "ship_a", -7, [25.0, 3.0, 0.0], -2, 0xDEAD_BEEF);
    // Class-specific state is framed but is not imported as placement data.
    f.add("class_payload", 4, &123_i32.to_le_bytes());
    f.object("second_project", "ship_b", 8, [75.0, -4.0, 100.0], 3, 0);
    f.mission(mission);
    f.add("aoi_count", 4, &1_i32.to_le_bytes());
    f.aoi(0xDEAD_BEEF, -3, 2, 0, -9, 12);
    f.add("path_count", 4, &2_i32.to_le_bytes());
    f.path(
        0x0102_0304,
        "Camera_1",
        &[[25.0, 7.0, 0.0], [75.0, -7.0, 100.0]],
        -5,
        1.25,
    );
    f.path(0, "", &[], 0, 0.0);
    f
}

#[test]
fn complete_a1_map_retains_metadata_placements_mission_aois_and_paths() {
    for mission in ["EmptyMission", "Inst4XMission"] {
        let fixture = populated(false, mission);
        let header = inspect_header(&fixture.bytes).unwrap();
        let map = parse(&fixture.bytes).unwrap();
        let a1 = map.a1.as_ref().unwrap();
        assert_eq!(a1.header, header);
        assert_eq!(a1.terrain_name, "terrain_a");
        assert_eq!(a1.source_bounds, ([0.0, -20.0, -50.0], [100.0, 40.0, 150.0]));
        assert!(!a1.resized);
        assert_eq!(a1.declared_object_count, 2);
        assert_eq!(&a1.mission_title[..11], b"A1 Mission\0");
        assert_eq!(&a1.mission_title[11..15], b"TAIL");
        assert_eq!(a1.object_refs, [0xDEAD_BEEF, 0]);
        assert_eq!(a1.initial_positions, [[25.0, 3.0, 0.0], [75.0, -4.0, 100.0]]);
        assert_eq!(map.version, 2051);
        assert!(map.binary);
        assert_eq!(map.mission_type, mission);
        assert_eq!(map.description, "A1 Mission");
        assert_eq!(map.file_name, "mission_a.msn");
        assert_eq!(map.scripts, ["one.dll", "two.drl"]);
        assert_eq!(map.min_extent, [0.0, -20.0, -50.0]);
        assert_eq!(map.size, [100.0, 60.0, 200.0]);
        assert_eq!(map.ambient, [0.25, 0.5, 0.75]);
        assert_eq!(map.background, "space.sod");
        assert_eq!(map.tech_tree_file, "stock.tt");
        assert_eq!(map.race, "federation");
        assert_eq!(map.objects.len(), 2);
        assert_eq!(
            (
                map.objects[0].odf.as_str(),
                map.objects[0].entity_id,
                map.objects[0].team
            ),
            ("ship_a", -7, -2)
        );
        assert_eq!(map.objects[0].label, "Authored label");
        assert_eq!(
            &map.objects[0].transform[..9],
            &[0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, -1.0]
        );
        assert_eq!(&map.objects[0].transform[9..], &[25.0, 3.0, 0.0]);
        assert_eq!(
            (
                a1.areas[0].object_ref,
                a1.areas[0].team,
                a1.areas[0].interesting,
                a1.areas[0].inside,
                a1.areas[0].value,
                a1.areas[0].force
            ),
            (0xDEAD_BEEF, -3, true, false, -9, 12)
        );
        assert_eq!(a1.paths.len(), 2);
        assert_eq!(
            (
                a1.paths[0].reference,
                a1.paths[0].label.as_str(),
                a1.paths[0].path_type,
                a1.paths[0].asteroid_belt_speed
            ),
            (0x0102_0304, "Camera_1", -5, 1.25)
        );
        assert_eq!(a1.paths[0].points, [[25.0, 7.0, 0.0], [75.0, -7.0, 100.0]]);
        assert!(a1.paths[1].label.is_empty() && a1.paths[1].points.is_empty());
    }
}

#[test]
fn resize_maps_xz_only_for_objects_transforms_and_path_points() {
    let fixture = populated(true, "EmptyMission");
    let map = parse(&fixture.bytes).unwrap();
    let a1 = map.a1.as_ref().unwrap();
    assert!(a1.resized);
    assert_eq!(a1.source_bounds, ([0.0, -20.0, -50.0], [100.0, 40.0, 150.0]));
    assert_eq!(map.min_extent, [-100.0, -999.0, 100.0]);
    assert_eq!(map.size, [400.0, 1998.0, 400.0]);
    assert_eq!(a1.initial_positions, [[0.0, 3.0, 200.0], [200.0, -4.0, 400.0]]);
    assert_eq!(
        &map.objects[0].transform[..9],
        &[0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, -1.0]
    );
    assert_eq!(&map.objects[0].transform[9..], &[0.0, 3.0, 200.0]);
    assert_eq!(&map.objects[1].transform[9..], &[200.0, -4.0, 400.0]);
    assert_eq!(a1.paths[0].points, [[0.0, 7.0, 200.0], [200.0, -7.0, 400.0]]);
}

#[test]
fn separate_factory_position_does_not_replace_authored_final_transform() {
    let mut fixture = populated(false, "EmptyMission");
    let at = fixture.at("position") + 8;
    fixture.bytes[at..at + 12].copy_from_slice(&floats(&[150.0, -123.0, 200.0]));
    let map = parse(&fixture.bytes).unwrap();
    let a1 = map.a1.as_ref().unwrap();
    assert_eq!(a1.initial_positions[0], [150.0, -123.0, 200.0]);
    assert_eq!(a1.initial_positions[1], [75.0, -4.0, 100.0]);
    assert_eq!(&map.objects[0].transform[9..], &[25.0, 3.0, 0.0]);
    assert_eq!(
        &map.objects[0].transform[..9],
        &[0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, -1.0]
    );
}

#[test]
fn zero_objects_aois_and_paths_are_complete_not_missing() {
    let mut f = Fixture::new(true);
    f.body(false, 0);
    f.mission("EmptyMission");
    f.add("aoi_count", 4, &0_i32.to_le_bytes());
    f.add("path_count", 4, &0_i32.to_le_bytes());
    let map = parse(&f.bytes).unwrap();
    assert!(map.objects.is_empty());
    let a1 = map.a1.unwrap();
    assert_eq!(a1.declared_object_count, 0);
    assert!(a1.object_refs.is_empty() && a1.areas.is_empty() && a1.paths.is_empty());
}

#[test]
fn wrong_counts_types_sizes_and_truncation_fail_closed() {
    let original = populated(false, "EmptyMission");
    let mut short_project = original.bytes.clone();
    let at = original.at("first_project");
    short_project[at + 4..at + 8].copy_from_slice(&7_u32.to_le_bytes());
    short_project.remove(at + 8 + 7); // still a complete, aligned seven-byte record
    assert!(parse(&short_project).is_err(), "accepted seven-byte PrjID");
    for (field, offset, bytes) in [
        ("objects", 8, 3_i32.to_le_bytes().to_vec()),
        ("objects", 8, (-1_i32).to_le_bytes().to_vec()),
        ("aoi_count", 8, 2_i32.to_le_bytes().to_vec()),
        ("path_count", 8, 3_i32.to_le_bytes().to_vec()),
        ("point_count", 8, 3_i32.to_le_bytes().to_vec()),
        ("first_project", 0, 4_u32.to_le_bytes().to_vec()),
        ("transform", 4, 47_u32.to_le_bytes().to_vec()),
        ("mission_class", 4, 39_u32.to_le_bytes().to_vec()),
    ] {
        let mut bad = original.bytes.clone();
        let at = original.at(field) + offset;
        bad[at..at + bytes.len()].copy_from_slice(&bytes);
        assert!(parse(&bad).is_err(), "accepted malformed {field} offset {offset}");
    }
    for cut in [1, 3, 7] {
        assert!(
            parse(&original.bytes[..original.bytes.len() - cut]).is_err(),
            "accepted {cut}-byte truncation"
        );
    }
    let mut trailing = original.bytes.clone();
    trailing.push(0);
    assert!(parse(&trailing).is_err());
}

#[test]
fn unsupported_mission_saved_state_text_and_nonfinite_values_fail() {
    let original = populated(false, "EmptyMission");
    for field in ["mission_class", "terrain"] {
        let mut bad = original.bytes.clone();
        let at = original.at(field) + 8;
        bad[at] = 0xFF; // unsupported text encoding before the first NUL
        assert!(parse(&bad).is_err(), "accepted non-UTF-8 {field}");
    }
    let mut unknown = populated(false, "MysteryMission");
    assert!(parse(&unknown.bytes).is_err());
    unknown = populated(false, "EmptyMission");
    // A second complete mission descriptor is ambiguous, not ignorable tail data.
    let mut extra = Fixture {
        bytes: Vec::new(),
        fields: Vec::new(),
    };
    extra.mission("Inst4XMission");
    let at = unknown.at("aoi_count");
    unknown.bytes.splice(at..at, extra.bytes);
    assert!(parse(&unknown.bytes).is_err());

    let mut saved = original.bytes.clone();
    saved[original.at("mission_save") + 8] = 0;
    assert!(parse(&saved).is_err());
    let mut text_body = Fixture::new(false);
    text_body.body(false, 0);
    text_body.mission("EmptyMission");
    text_body.add("aoi_count", 4, &0_i32.to_le_bytes());
    text_body.add("path_count", 4, &0_i32.to_le_bytes());
    assert!(parse(&text_body.bytes).is_err());
    for (field, value) in [
        ("max_x", f32::NAN),
        ("red", f32::INFINITY),
        ("path_speed", f32::NEG_INFINITY),
    ] {
        let mut bad = original.bytes.clone();
        bad[original.at(field) + 8..original.at(field) + 12].copy_from_slice(&value.to_le_bytes());
        assert!(parse(&bad).is_err(), "accepted nonfinite {field}");
    }
}

#[test]
#[ignore = "requires STA64_TEST_A1_BZN_DIR pointing at user-owned original A1 maps"]
fn original_a1_corpus_has_complete_nonempty_map_bodies() {
    let root = std::env::var_os("STA64_TEST_A1_BZN_DIR").expect("set STA64_TEST_A1_BZN_DIR");
    let mut count = 0;
    let mut objects = 0;
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("bzn")) {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let map = parse(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let a1 = map
            .a1
            .as_ref()
            .unwrap_or_else(|| panic!("{}: missing A1 body", path.display()));
        assert!(!map.objects.is_empty(), "{}", path.display());
        assert_eq!(map.objects.len(), a1.declared_object_count, "{}", path.display());
        assert_eq!(a1.object_refs.len(), map.objects.len(), "{}", path.display());
        assert!(!map.mission_type.is_empty(), "{}", path.display());
        objects += map.objects.len();
        count += 1;
    }
    assert!(count > 0 && objects > 0, "no nonempty A1 maps found");
    eprintln!("A1 map bodies inspected: {count}; objects: {objects}");
}
