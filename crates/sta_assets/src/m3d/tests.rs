//! Independent, original byte streams for the public Legacy loader.

use super::load;
use crate::model::{NodeKind, SourceFormat};

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

fn matrix(out: &mut Vec<u8>) {
    for value in [1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.] {
        f32_(out, value);
    }
}

fn material(out: &mut Vec<u8>, version: f32) {
    id(out, "team");
    for value in [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 8.0] {
        f32_(out, value);
    }
    if version >= 1.99 {
        u32_(out, 2);
        u32_(out, 1);
    } else {
        out.extend_from_slice(&[2, 1]);
    }
}

fn header(version: f32, nodes: u16) -> Vec<u8> {
    let mut out = b"Mad3D_SW\0\0".to_vec();
    f32_(&mut out, version);
    u16_(&mut out, 1);
    material(&mut out, version);
    u16_(&mut out, nodes);
    out
}

fn footer(out: &mut Vec<u8>, version: f32) {
    u16_(out, 0); // animation count
    if version >= 1.96 {
        u32_(out, 0); // database value
    }
    u16_(out, 0); // reference count
}

fn named_node(out: &mut Vec<u8>, kind: u16, name: &str, parent: &str) {
    u16_(out, kind);
    id(out, name);
    id(out, parent);
    matrix(out);
}

fn numeric_node(out: &mut Vec<u8>, kind: u16, name: &str, id_value: u32, parent: u32) {
    id(out, name);
    u32_(out, id_value);
    u16_(out, kind);
    u32_(out, parent);
    matrix(out);
}

fn light(out: &mut Vec<u8>, subtype: u8, values: [f32; 5]) {
    out.push(subtype);
    for value in values {
        f32_(out, value);
    }
}

// The 1.93 layout follows the documented SOD reader: mesh material and texture
// list precede counts, with positions/UVs but no stored normals.
fn old_mesh(out: &mut Vec<u8>) {
    id(out, "opaque");
    u32_(out, 4); // mesh flags
    u32_(out, 1); // texture count
    id(out, "textures\\old_hull.TGA");
    u32_(out, 0);
    id(out, ""); // second texture
    u16_(out, 0); // unknown field
    u16_(out, 3); // positions
    u16_(out, 3); // UVs
    u16_(out, 1); // groups
    for pos in [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]] {
        for value in pos {
            f32_(out, value);
        }
    }
    for uv in [[0., 0.], [1., 0.], [0., 1.]] {
        for value in uv {
            f32_(out, value);
        }
    }
    u16_(out, 1);
    id(out, "team");
    for corner in 0..3 {
        u16_(out, corner);
        u16_(out, corner);
    }
    out.push(1);
    u16_(out, 0);
}

fn old_fixture() -> Vec<u8> {
    let mut out = header(1.93, 3);
    named_node(&mut out, 1, "hull", "detail"); // child before parent
    old_mesh(&mut out);
    named_node(&mut out, 11, "detail", "root");
    named_node(&mut out, 0, "root", "");
    footer(&mut out, 1.93);
    out
}

fn old_light_fixture(value: f32) -> Vec<u8> {
    let mut out = header(1.93, 2);
    named_node(&mut out, 2, "lamp", "root");
    light(&mut out, 3, [value, 0.25, 0.5, 2., 6.]);
    named_node(&mut out, 0, "root", "");
    footer(&mut out, 1.93);
    out
}

fn texture(out: &mut Vec<u8>, name: &str) {
    id(out, name);
    u32_(out, 0);
}

// For 1.94/1.96 the M3D reader stores mesh normals and direct face corners.
fn newer_mesh(out: &mut Vec<u8>, version: f32, faces: u16) {
    u32_(out, 0); // mesh flags
    u16_(out, 0); // trailer
    u16_(out, 3); // vertices
    if version >= 2.02 {
        u16_(out, 0); // normals
    }
    u16_(out, 3); // UVs
    u16_(out, 1); // groups
    for pos in [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]] {
        for value in pos {
            f32_(out, value);
        }
    }
    if version < 2.02 {
        for _ in 0..3 {
            for value in [0., 0., 1.] {
                f32_(out, value);
            }
        }
    }
    for uv in [[0., 0.], [1., 0.], [0., 1.]] {
        for value in uv {
            f32_(out, value);
        }
    }
    u16_(out, faces);
    id(out, "team");
    if version >= 1.99 {
        for name in ["new_hull.dds", "bump.dds", "spec.dds", "glow.dds"] {
            texture(out, name);
        }
        if version >= 2.0 {
            texture(out, "detail.dds");
        }
    } else {
        u32_(out, 1); // group texture count
        texture(out, "textures\\new_hull.DDS");
    }
    id(out, "alpha");
    if version >= 2.05 {
        id(out, "");
    }
    for _ in 0..faces {
        for corner in 0..3 {
            u16_(out, corner);
            u16_(out, corner);
        }
    }
    out.push(1); // cull
    if version < 2.01 {
        u16_(out, 0); // mesh extra count
    }
}

fn named_fixture() -> Vec<u8> {
    let mut out = header(1.94, 3);
    named_node(&mut out, 2, "lamp", "root");
    light(&mut out, 7, [0.25, 0.5, 0.75, 4., 12.]);
    named_node(&mut out, 1, "hull", "root");
    newer_mesh(&mut out, 1.94, 1);
    named_node(&mut out, 0, "root", "");
    footer(&mut out, 1.94);
    out
}

fn lod_fixture() -> Vec<u8> {
    let mut out = header(1.96, 6);
    numeric_node(&mut out, 1, "fine_mesh", 6, 4);
    newer_mesh(&mut out, 1.96, 2);
    numeric_node(&mut out, 1, "coarse_mesh", 5, 3);
    newer_mesh(&mut out, 1.96, 1);
    numeric_node(&mut out, 0, "lod_coarse", 3, 2);
    numeric_node(&mut out, 11, "control", 2, 1);
    numeric_node(&mut out, 0, "lod_fine", 4, 2);
    numeric_node(&mut out, 0, "root", 1, 0);
    footer(&mut out, 1.96);
    out
}

fn numeric_light_fixture(version: f32) -> Vec<u8> {
    let mut out = header(version, 2);
    numeric_node(&mut out, 2, "lamp", 8, 7);
    light(&mut out, 2, [0.125, 0.25, 0.5, 3., 9.]);
    numeric_node(&mut out, 0, "root", 7, 0);
    footer(&mut out, version);
    out
}

fn duplicate_id_fixture() -> Vec<u8> {
    let mut out = header(1.96, 3);
    numeric_node(&mut out, 2, "lamp", 8, 7);
    light(&mut out, 2, [0.125, 0.25, 0.5, 3., 9.]);
    numeric_node(&mut out, 0, "first", 7, 0);
    numeric_node(&mut out, 0, "last", 7, 0);
    footer(&mut out, 1.96);
    out
}

fn node<'a>(model: &'a crate::model::Model, name: &str) -> (usize, &'a crate::model::Node) {
    model.nodes.iter().enumerate().find(|(_, n)| n.name == name).unwrap()
}

#[test]
fn old_layout_keeps_sod_winding_material_and_hierarchy() {
    let bytes = old_fixture();
    let original = bytes.clone();
    let model = load(&bytes).unwrap();
    assert_eq!(bytes, original, "in-memory magic normalization must not mutate input");
    assert_eq!(model.format, SourceFormat::M3d);
    assert_eq!(model.version, 1.93);
    assert_eq!(model.materials[0].name, "team");
    assert_eq!(model.materials[0].diffuse, [0.4, 0.5, 0.6]);
    assert!(model.materials[0].self_illuminating);
    let (root, _) = node(&model, "root");
    let (detail, lod) = node(&model, "detail");
    let (_, hull) = node(&model, "hull");
    assert!(matches!(lod.kind, NodeKind::Lod));
    assert_eq!(lod.parent, Some(root));
    assert_eq!(hull.parent, Some(detail));
    let group = &hull.mesh().unwrap().groups[0];
    assert_eq!(group.material, "team");
    assert_eq!(group.blend, "opaque");
    assert_eq!(group.maps.diffuse.as_deref(), Some("textures\\old_hull.TGA"));
    assert_eq!(group.faces, vec![[(0, 0), (1, 1), (2, 2)]]);
}

#[test]
fn named_new_layout_preserves_light_values_and_reverses_mesh_winding() {
    let model = load(&named_fixture()).unwrap();
    assert_eq!(model.format, SourceFormat::M3d);
    assert_eq!(model.version, 1.94);
    let (root, _) = node(&model, "root");
    let (_, lamp) = node(&model, "lamp");
    assert_eq!(lamp.parent, Some(root));
    let NodeKind::Light(light) = &lamp.kind else {
        panic!("expected typed light")
    };
    assert_eq!(light.kind, 7);
    assert_eq!(light.colour, [0.25, 0.5, 0.75]);
    assert_eq!(light.falloff_start, 4.);
    assert_eq!(light.falloff_range, 12.);
    assert!(
        model
            .warnings
            .iter()
            .any(|w| w.contains("renderer light playback unavailable"))
    );
    let (_, hull) = node(&model, "hull");
    let group = &hull.mesh().unwrap().groups[0];
    assert_eq!(group.material, "team");
    assert_eq!(group.blend, "alpha");
    assert_eq!(group.maps.diffuse.as_deref(), Some("new_hull"));
    assert_eq!(group.faces, vec![[(0, 0), (2, 2), (1, 1)]]);
}

#[test]
fn typed_control_chooses_branch_with_most_faces_without_dropping_geometry() {
    let model = load(&lod_fixture()).unwrap();
    let (control_id, control) = node(&model, "control");
    assert!(matches!(control.kind, NodeKind::Lod));
    let (coarse_id, coarse) = node(&model, "lod_coarse");
    let (fine_id, fine) = node(&model, "lod_fine");
    assert_eq!(coarse.parent, Some(control_id));
    assert_eq!(fine.parent, Some(control_id));
    assert_eq!(node(&model, "coarse_mesh").1.parent, Some(coarse_id));
    assert_eq!(node(&model, "fine_mesh").1.parent, Some(fine_id));
    assert_eq!(node(&model, "coarse_mesh").1.mesh().unwrap().groups[0].faces.len(), 1);
    assert_eq!(node(&model, "fine_mesh").1.mesh().unwrap().groups[0].faces.len(), 2);
    assert_eq!(model.coarse_lods(), vec![coarse_id]);
}

#[test]
fn numeric_206_light_links_child_before_parent() {
    let model = load(&numeric_light_fixture(2.06)).unwrap();
    let (root, _) = node(&model, "root");
    let (_, lamp) = node(&model, "lamp");
    assert_eq!(lamp.parent, Some(root));
    let NodeKind::Light(light) = &lamp.kind else {
        panic!("expected typed light")
    };
    assert_eq!(light.kind, 2);
    assert_eq!(light.colour, [0.125, 0.25, 0.5]);
    assert_eq!((light.falloff_start, light.falloff_range), (3., 9.));
}

#[test]
fn duplicate_numeric_parent_id_resolves_to_last_source_node() {
    let model = load(&duplicate_id_fixture()).unwrap();
    let (last, _) = node(&model, "last");
    assert_eq!(node(&model, "lamp").1.parent, Some(last));
    assert!(node(&model, "first").1.parent.is_some()); // both roots attach to synthesized root
}

#[test]
fn old_light_payload_is_typed_and_finite() {
    let model = load(&old_light_fixture(0.125)).unwrap();
    let (root, _) = node(&model, "root");
    let (_, lamp) = node(&model, "lamp");
    assert_eq!(lamp.parent, Some(root));
    let NodeKind::Light(light) = &lamp.kind else {
        panic!("expected typed light")
    };
    assert_eq!(light.kind, 3);
    assert_eq!(light.colour, [0.125, 0.25, 0.5]);
    assert_eq!((light.falloff_start, light.falloff_range), (2., 6.));
    assert!(load(&old_light_fixture(f32::NAN)).unwrap_err().contains("Non-finite"));
}

#[test]
fn all_prefixes_of_complete_layouts_fail_without_panicking() {
    for bytes in [
        old_fixture(),
        old_light_fixture(0.125),
        named_fixture(),
        lod_fixture(),
        numeric_light_fixture(2.06),
        duplicate_id_fixture(),
    ] {
        assert!(load(&bytes).is_ok());
        for end in 0..bytes.len() {
            let result = std::panic::catch_unwind(|| load(&bytes[..end]));
            assert!(matches!(result, Ok(Err(_))), "prefix {end} of {} bytes", bytes.len());
        }
    }
}

#[test]
fn rejects_nonfinite_new_light_and_unsupported_version_with_context() {
    let mut bytes = numeric_light_fixture(1.96);
    let light_value = bytes.windows(4).position(|w| w == 0.125f32.to_le_bytes()).unwrap();
    bytes[light_value..light_value + 4].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(load(&bytes).unwrap_err().contains("Non-finite M3D light data"));

    let mut unsupported = named_fixture();
    unsupported[10..14].copy_from_slice(&1.92f32.to_le_bytes());
    let error = load(&unsupported).unwrap_err();
    assert!(error.contains("Unsupported Legacy M3D version 1.92"), "{error}");
}
