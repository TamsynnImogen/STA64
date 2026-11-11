use sta_bzn::{MapObject, parse};

fn rec(kind: u32, data: &[u8]) -> Vec<u8> {
    let mut out = kind.to_le_bytes().to_vec();
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    out
}

fn floats(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|value| value.to_le_bytes()).collect()
}

fn binary_object(out: &mut Vec<u8>, name: &str, id: i32, prefix: [f32; 3], final_matrix: Option<[f32; 12]>) {
    out.extend(rec(3, &(name.len() as i16).to_le_bytes()));
    out.extend(rec(2, name.as_bytes()));
    out.extend(rec(4, &id.to_le_bytes()));
    out.extend(rec(9, &floats(&prefix)));
    out.extend(rec(4, &1i32.to_le_bytes()));
    out.extend(rec(2, b"Synthetic"));
    out.extend(rec(8, &0u32.to_le_bytes()));
    if let Some(matrix) = final_matrix {
        out.extend(rec(11, &floats(&matrix)));
    }
}

fn binary_map(objects: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut out = b"version [1] =\r\n2172\r\nBinaryMode [1] =\r\ntrue\r\n".to_vec();
    for width in [100, 100, 128, 128] {
        out.extend(rec(2, &vec![0; width]));
    }
    objects(&mut out);
    out
}

fn text_map(objects: &str) -> Vec<u8> {
    format!("version [1] =\n2177\nBinaryMode [1] =\nfalse\n{objects}").into_bytes()
}

fn text_object(name: &str, pos_fields: &str, final_fields: &str) -> String {
    format!("[GameObject]\nPrjID = {name}\n{pos_fields}transform [1] =\n{final_fields}")
}

#[test]
fn binary_retains_each_prefix_separately_from_later_matrix_and_orientation() {
    let first_matrix = [0.0, 0.0, 1.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 100.0, 200.0, 300.0];
    let second_matrix = [1.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 1.0, 0.0, -5.0, 6.0, 7.0];
    let bytes = binary_map(|out| {
        binary_object(out, "first.odf", 11, [1.0, 2.0, 3.0], Some(first_matrix));
        binary_object(out, "second.odf", 22, [8.0, -9.0, 10.0], Some(second_matrix));
    });
    let map = parse(&bytes).unwrap();
    assert_eq!(map.objects.len(), 2);
    assert_eq!((map.objects[0].odf.as_str(), map.objects[0].entity_id), ("first", 11));
    assert_eq!(map.objects[0].initial_position, Some([1.0, 2.0, 3.0]));
    assert_eq!(map.objects[0].transform, first_matrix);
    assert_eq!((map.objects[1].odf.as_str(), map.objects[1].entity_id), ("second", 22));
    assert_eq!(map.objects[1].initial_position, Some([8.0, -9.0, 10.0]));
    assert_eq!(map.objects[1].transform, second_matrix);
}

#[test]
fn binary_without_matrix_keeps_identity_orientation_and_prefix_translation() {
    let bytes = binary_map(|out| binary_object(out, "fallback", 1, [-0.0, 0.0, 4.0], None));
    let map = parse(&bytes).unwrap();
    assert_eq!(map.objects.len(), 1);
    let object = &map.objects[0];
    let prefix = object.initial_position.unwrap();
    assert_eq!(prefix[0].to_bits(), (-0.0f32).to_bits());
    assert_eq!(&object.transform[..9], &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    assert_eq!(object.transform[9].to_bits(), (-0.0f32).to_bits());
    assert_eq!(&object.transform[10..], &[0.0, 4.0]);
}

#[test]
fn text_complete_prefix_does_not_override_final_matrix_coordinates() {
    let object = text_object(
        "first.odf",
        "pos [1] =\n  x [1] =\n10\n  y [1] =\n-2.5\n  z [1] =\n30\n",
        "  right_x [1] =\n0\n  right_z [1] =\n-1\n  position.x [1] =\n100\n  position.y [1] =\n200\n  position.z [1] =\n300\n",
    );
    let map = parse(&text_map(&object)).unwrap();
    assert_eq!(map.objects.len(), 1);
    assert_eq!(map.objects[0].initial_position, Some([10.0, -2.5, 30.0]));
    assert_eq!(&map.objects[0].transform[9..], &[100.0, 200.0, 300.0]);
    assert_eq!(&map.objects[0].transform[..3], &[0.0, 0.0, -1.0]);
}

#[test]
fn text_missing_or_incomplete_prefix_is_none_while_legacy_final_fallback_stays() {
    let absent = text_object(
        "absent",
        "",
        "  position.x [1] =\n4\n  position.y [1] =\n5\n  position.z [1] =\n6\n",
    );
    let partial = text_object("partial", "pos [1] =\n  x [1] =\n7\n  z [1] =\n-9\n", "");
    let unparseable = text_object(
        "unparseable",
        "pos [1] =\n  x [1] =\n1\n  y [1] =\nnot-a-number\n  z [1] =\n3\n",
        "  position.x [1] =\n40\n  position.y [1] =\n50\n  position.z [1] =\n60\n",
    );
    let map = parse(&text_map(&format!("{absent}{partial}{unparseable}"))).unwrap();
    assert_eq!(map.objects.len(), 3);
    assert_eq!(
        map.objects.iter().map(|object| object.odf.as_str()).collect::<Vec<_>>(),
        ["absent", "partial", "unparseable"]
    );
    assert!(map.objects.iter().all(|object| object.initial_position.is_none()));
    assert_eq!(&map.objects[0].transform[9..], &[4.0, 5.0, 6.0]);
    assert_eq!(&map.objects[1].transform[9..], &[7.0, 0.0, -9.0]);
    assert_eq!(&map.objects[2].transform[9..], &[40.0, 50.0, 60.0]);
}

#[test]
fn text_complete_signed_zero_prefix_is_retained_with_legacy_fallback() {
    let object = text_object("zero", "pos [1] =\n  x [1] =\n-0\n  y [1] =\n0\n  z [1] =\n0\n", "");
    let map = parse(&text_map(&object)).unwrap();
    assert_eq!(map.objects.len(), 1);
    let prefix = map.objects[0].initial_position.unwrap();
    assert_eq!(prefix[0].to_bits(), (-0.0f32).to_bits());
    assert_eq!(&prefix[1..], &[0.0, 0.0]);
    assert_eq!(map.objects[0].transform[9].to_bits(), (-0.0f32).to_bits());
    assert_eq!(&map.objects[0].transform[10..], &[0.0, 0.0]);
}

#[test]
fn default_object_has_no_initial_position() {
    assert_eq!(MapObject::default().initial_position, None);
}
