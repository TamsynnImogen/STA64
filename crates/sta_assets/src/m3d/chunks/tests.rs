//! Original synthetic streams for the explicit chunk roles.

use super::{Corner, load_geometry, load_materials};

fn u16_(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn u32_(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn f32_(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn id(out: &mut Vec<u8>, value: &str) {
    u16_(out, value.len() as u16);
    out.extend_from_slice(value.as_bytes());
}

fn header(version: f32) -> Vec<u8> {
    let mut out = b"Mad3D_SW\0\0".to_vec();
    f32_(&mut out, version);
    out
}

fn library(version: f32) -> Vec<u8> {
    let mut out = header(version);
    u32_(&mut out, 1); // material records
    id(&mut out, "team");
    for (name, flags) in [
        ("Textures\\Diffuse.DDS", 0x1001),
        ("Bump.tga", 0x200),
        ("Spec.dds", 0x4000),
        ("Glow.dds", 0x8000),
        ("Detail.dds", 0x1234_5678),
    ] {
        id(&mut out, name);
        u32_(&mut out, flags);
    }
    id(&mut out, "alpha");
    if version >= 2.05 {
        id(&mut out, "native-extra");
    }
    u32_(&mut out, 3); // geometry variants
    out
}

struct GeometryBytes {
    bytes: Vec<u8>,
    records_count_at: usize,
    first_attribute_at: usize,
}

fn geometry(version: f32, normals: bool, material_index: i32, record_count: u32, face_record: u32) -> GeometryBytes {
    let mut out = header(version);
    for bound in [1., 2., 3., 4., 5., 6., 7., 8.] {
        f32_(&mut out, bound);
    }
    u32_(&mut out, 1); // one chunk
    u32_(&mut out, 0xace0_00c5); // special flag
    u32_(&mut out, if normals { 0x10 } else { 0 }); // mesh flags
    u16_(&mut out, 0x52); // trailer
    u16_(&mut out, 3); // positions
    u16_(&mut out, if normals { 1 } else { 0 }); // normal count
    u16_(&mut out, 3); // UVs
    u16_(&mut out, 1); // groups
    for pos in [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]] {
        for value in pos {
            f32_(&mut out, value);
        }
    }
    if normals {
        for value in [0., 0., 1.] {
            f32_(&mut out, value);
        }
    }
    for uv in [[0., 0.], [1., 0.], [0., 1.]] {
        for value in uv {
            f32_(&mut out, value);
        }
    }
    u16_(&mut out, 1); // one face; no inline material record
    for corner in [0, face_record, 2] {
        if version >= 2.06 {
            u32_(&mut out, corner);
        } else {
            u16_(&mut out, corner as u16);
        }
    }
    let records_count_at = out.len();
    u32_(&mut out, record_count);
    let first_attribute_at = out.len();
    for record in 0..record_count {
        let position = if record == 0 {
            0
        } else if record == face_record {
            1
        } else if record == 2 {
            2
        } else {
            0
        };
        for attribute in [position, 0, position] {
            if version >= 2.06 {
                u32_(&mut out, attribute);
            } else {
                u16_(&mut out, attribute as u16);
            }
        }
    }
    for _ in 0..6 {
        f32_(&mut out, 0.); // six group bound floats
    }
    u32_(&mut out, 0); // bounds records
    u32_(&mut out, 0); // bounds indices
    u16_(&mut out, 0); // bounds depth
    out.push(1); // cull flag
    u32_(&mut out, material_index as u32); // signed library index follows mesh
    GeometryBytes {
        bytes: out,
        records_count_at,
        first_attribute_at,
    }
}

#[test]
fn material_records_keep_all_texture_flags_and_versioned_shader_data() {
    for version in [2.04, 2.05, 2.06] {
        let bytes = library(version);
        let original = bytes.clone();
        let parsed = load_materials(&bytes).unwrap();
        assert_eq!(bytes, original);
        assert_eq!(parsed.version, version);
        assert_eq!(parsed.variant_count, 3);
        assert_eq!(parsed.materials.len(), 1);
        let material = &parsed.materials[0];
        assert_eq!(material.lighting_material.as_slice(), b"team");
        assert_eq!(material.textures[0].reference.as_slice(), b"Textures\\Diffuse.DDS");
        assert_eq!(material.textures[0].flags, 0x1001);
        assert_eq!(material.textures[1].flags, 0x200);
        assert_eq!(material.textures[2].flags, 0x4000);
        assert_eq!(material.textures[3].flags, 0x8000);
        assert_eq!(material.textures[4].flags, 0x1234_5678);
        assert_eq!(material.shader.as_slice(), b"alpha");
        assert_eq!(
            material.extra_shader.as_slice(),
            if version >= 2.05 {
                b"native-extra".as_slice()
            } else {
                b"".as_slice()
            }
        );
    }
}

#[test]
fn geometry_keeps_bounds_flag_normals_raw_winding_and_library_index() {
    let library = load_materials(&library(2.04)).unwrap();
    for version in [2.04, 2.05, 2.06] {
        for normals in [false, true] {
            let fixture = geometry(version, normals, 0, 3, 1);
            let original = fixture.bytes.clone();
            let variant = load_geometry(&fixture.bytes).unwrap();
            assert_eq!(fixture.bytes, original);
            assert_eq!(variant.version, version);
            assert_eq!(variant.bounds, [1., 2., 3., 4., 5., 6., 7., 8.]);
            assert_eq!(variant.chunks.len(), 1);
            let mesh = &variant.chunks[0];
            assert_eq!(mesh.special_flag, 0xace0_00c5);
            assert_eq!(mesh.flags, if normals { 0x10 } else { 0 });
            assert_eq!(mesh.trailer, 0x52);
            assert_eq!(mesh.positions.len(), 3);
            assert_eq!(mesh.normals, if normals { vec![[0., 0., 1.]] } else { vec![] });
            assert_eq!(mesh.uvs.len(), 3);
            assert_eq!(mesh.cull, 1);
            assert_eq!(mesh.groups[0].material_index, 0);
            assert_eq!(
                mesh.groups[0].faces[0],
                [
                    Corner {
                        position: 0,
                        normal: 0,
                        uv: 0
                    },
                    Corner {
                        position: 1,
                        normal: 0,
                        uv: 1
                    },
                    Corner {
                        position: 2,
                        normal: 0,
                        uv: 2
                    },
                ]
            );
            variant.validate_materials(&library).unwrap();
        }
    }
}

#[test]
fn negative_binding_selects_default_and_positive_out_of_range_fails_validation() {
    let library = load_materials(&library(2.06)).unwrap();
    let default = load_geometry(&geometry(2.06, false, -1, 3, 1).bytes).unwrap();
    assert_eq!(default.chunks[0].groups[0].material_index, -1);
    default.validate_materials(&library).unwrap();

    let invalid = load_geometry(&geometry(2.06, false, 1, 3, 1).bytes).unwrap();
    assert!(
        invalid
            .validate_materials(&library)
            .unwrap_err()
            .contains("material 1 is outside library")
    );
}

#[test]
fn wide_206_corner_record_index_is_not_narrowed_to_u16() {
    // 65,537 records make index 65,536 valid. All records are tiny and generated
    // explicitly; this stream is excluded from the prefix sweep below.
    let variant = load_geometry(&geometry(2.06, false, 0, 65_537, 65_536).bytes).unwrap();
    let face = variant.chunks[0].groups[0].faces[0];
    assert_eq!(
        face[1],
        Corner {
            position: 1,
            normal: 0,
            uv: 1
        }
    );
}

#[test]
fn roles_and_every_small_truncated_prefix_are_rejected_without_panic() {
    for version in [2.04, 2.05, 2.06] {
        let materials = library(version);
        let geometry = geometry(version, true, 0, 3, 1).bytes;
        assert!(load_geometry(&materials).is_err(), "material library in geometry role");
        assert!(load_materials(&geometry).is_err(), "geometry in material role");
        for end in 0..materials.len() {
            let result = std::panic::catch_unwind(|| load_materials(&materials[..end]));
            assert!(matches!(result, Ok(Err(_))), "material version {version}, prefix {end}");
        }
        for end in 0..geometry.len() {
            let result = std::panic::catch_unwind(|| load_geometry(&geometry[..end]));
            assert!(matches!(result, Ok(Err(_))), "geometry version {version}, prefix {end}");
        }
    }
}

#[test]
fn malformed_counts_attributes_bounds_and_trailing_bytes_have_context() {
    let mut material_count = library(2.04);
    material_count[14..18].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(
        load_materials(&material_count)
            .unwrap_err()
            .contains("chunk material count")
    );

    let mut mesh_count = geometry(2.05, false, 0, 3, 1).bytes;
    mesh_count[46..50].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(load_geometry(&mesh_count).unwrap_err().contains("chunk mesh count"));

    let mut bad_record = geometry(2.06, false, 0, 3, 1);
    bad_record.bytes[bad_record.records_count_at..bad_record.records_count_at + 4].copy_from_slice(&0u32.to_le_bytes());
    assert!(
        load_geometry(&bad_record.bytes)
            .unwrap_err()
            .contains("out-of-range corner record")
    );

    let mut bad_attribute = geometry(2.06, false, 0, 3, 1);
    bad_attribute.bytes[bad_attribute.first_attribute_at..bad_attribute.first_attribute_at + 4]
        .copy_from_slice(&3u32.to_le_bytes());
    assert!(
        load_geometry(&bad_attribute.bytes)
            .unwrap_err()
            .contains("out-of-range attribute index")
    );

    let mut nonfinite = geometry(2.04, false, 0, 3, 1).bytes;
    nonfinite[14..18].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(
        load_geometry(&nonfinite)
            .unwrap_err()
            .contains("Non-finite M3D chunk bounds")
    );

    let valid = geometry(2.06, false, 0, 3, 1);
    let bounds_count_at = valid.first_attribute_at + 3 * 12 + 24;
    for (offset, context) in [
        (bounds_count_at, "bounds record count"),
        (bounds_count_at + 4, "bounds index count"),
    ] {
        let mut oversized = valid.bytes.clone();
        oversized[offset..offset + 4].copy_from_slice(&10_000_001u32.to_le_bytes());
        assert!(load_geometry(&oversized).unwrap_err().contains(context));
    }

    let mut trailing = library(2.06);
    trailing.push(0);
    assert!(load_materials(&trailing).unwrap_err().contains("trailing data"));
}

#[test]
fn stored_nonfinite_uv_bits_are_retained_and_reported_without_changing_inputs() {
    let mut bytes = geometry(2.06, true, 0, 3, 1).bytes;
    let uv_at = 14 + 32 + 4 + 4 + 14 + 36 + 12;
    let nan_bits = 0xffc1_2345u32;
    let infinity_bits = f32::INFINITY.to_bits();
    bytes[uv_at..uv_at + 4].copy_from_slice(&nan_bits.to_le_bytes());
    bytes[uv_at + 4..uv_at + 8].copy_from_slice(&infinity_bits.to_le_bytes());
    let original = bytes.clone();
    let parsed = load_geometry(&bytes).unwrap();
    assert_eq!(parsed.chunks[0].uvs[0][0].to_bits(), nan_bits);
    assert_eq!(parsed.chunks[0].uvs[0][1].to_bits(), infinity_bits);
    assert_eq!(
        parsed.attribute_warnings(),
        ["chunk 0: 2 non-finite UV components retained; not render-safe"]
    );
    assert_eq!(bytes, original);
}
