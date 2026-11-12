use super::*;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        // pid + clock alone collided on coarse-clock hosts (macOS CI); the
        // counter makes every fixture in this process unique.
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "sta_gltf_{}_{}_{}",
            std::process::id(),
            stamp,
            n
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn write(&self, path: &str, bytes: &[u8]) {
        let target = self.0.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, bytes).unwrap();
    }
    fn vfs(&self) -> Vfs {
        let mut vfs = Vfs::default();
        vfs.push_dir("fixture".into(), self.0.clone()).unwrap();
        vfs
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn triangle() -> Vec<u8> {
    [[1.0f32, 0.0, 2.0], [0.0, 1.0, 2.0], [0.0, 0.0, 3.0]]
        .into_iter()
        .flat_map(|p| p.into_iter().flat_map(f32::to_le_bytes))
        .collect()
}
fn document(uri: Option<&str>, extra: &str) -> String {
    let buffer = if let Some(uri) = uri {
        format!(r#"{{"uri":"{uri}","byteLength":36}}"#)
    } else {
        r#"{"byteLength":36}"#.into()
    };
    format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{"translation":[1,2,3],"mesh":0}}],"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"mode":4{extra}}}]}}],"buffers":[{buffer}],"bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":36}}],"accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,2],"max":[1,1,3]}}]}}"#
    )
}

#[test]
fn external_triangle_and_handedness() {
    let f = Fixture::new();
    f.write(
        "models/a/model.gltf",
        document(Some("../geo%2Ebin"), "").as_bytes(),
    );
    f.write("models/geo.bin", &triangle());
    let m = load(&f.vfs(), "models/a/model.gltf").unwrap();
    assert_eq!(m.format, SourceFormat::Gltf);
    assert_eq!(m.nodes.len(), 2);
    assert_eq!(m.nodes[0].transform.translation(), [1.0, 2.0, -3.0]);
    let mesh = m.nodes[1].mesh().unwrap();
    assert_eq!(mesh.positions[0], [1.0, 0.0, -2.0]);
    assert_eq!(mesh.groups[0].faces[0], [(0, 0), (1, 1), (2, 2)]);
    assert!(m.pbr.as_ref().unwrap().attributes.contains_key(&1));
}

#[test]
fn glb_triangle_and_reject_out_of_range() {
    let f = Fixture::new();
    let mut json = document(None, "").into_bytes();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let bin = triangle();
    let length = 12 + 8 + json.len() + 8 + bin.len();
    let mut glb = Vec::new();
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(length as u32).to_le_bytes());
    glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&bin);
    f.write("m.glb", &glb);
    assert_eq!(load(&f.vfs(), "m.glb").unwrap().nodes.len(), 2);
    f.write(
        "bad.gltf",
        document(Some("data:application/octet-stream;base64,AA=="), "").as_bytes(),
    );
    assert!(load(&f.vfs(), "bad.gltf").unwrap_err().contains("shorter"));
}

#[test]
fn resource_rejects_escape_and_scheme() {
    for uri in [
        "../../../escape.bin",
        "%2Fetc/passwd",
        "file:secret",
        "https://example.test/a",
        "a%00b",
    ] {
        assert!(resource_path("models/a/model.gltf", uri).is_err(), "{uri}");
    }
    assert_eq!(
        resource_path("models/a/model.gltf", "../../escape.bin").unwrap(),
        "escape.bin"
    );
}

#[test]
fn pbr_factors_and_inline_image() {
    let f = Fixture::new();
    let png = b"\x89PNG\r\n\x1a\nsynthetic";
    let image_uri = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    );
    let b64 = base64::engine::general_purpose::STANDARD.encode(triangle());
    let mut json = document(
        Some(&format!("data:application/octet-stream;base64,{b64}")),
        ",\"material\":0",
    );
    json.pop();
    json.push_str(&format!(r#","images":[{{"uri":"{image_uri}"}}],"textures":[{{"source":0}}],"materials":[{{"name":"hull","pbrMetallicRoughness":{{"baseColorFactor":[0.2,0.3,0.4,0.5],"metallicFactor":0.6,"roughnessFactor":0.7}},"alphaMode":"MASK","alphaCutoff":0.35,"doubleSided":true}}]}}"#));
    f.write("m.gltf", json.as_bytes());
    let m = load(&f.vfs(), "m.gltf").unwrap();
    let p = m.pbr.unwrap();
    assert_eq!(p.images[0].data, png);
    let expected: [u8; 32] = Sha256::digest(png).into();
    assert_eq!(p.images[0].content_hash, expected);
    let mat = &p.materials["gltf_0"];
    assert_eq!(mat.base_color, [0.2, 0.3, 0.4, 0.5]);
    assert_eq!(mat.metallic, 0.6);
    assert_eq!(mat.roughness, 0.7);
    assert_eq!(mat.alpha, PbrAlpha::Mask(0.35));
    assert!(mat.double_sided);
}

#[test]
fn rejects_graph_cycle_and_invalid_accessor_range() {
    let f = Fixture::new();
    let b64 = base64::engine::general_purpose::STANDARD.encode(triangle());
    let json = document(
        Some(&format!("data:application/octet-stream;base64,{b64}")),
        "",
    );
    let cycle = json.replace(r#""mesh":0}"#, r#""mesh":0,"children":[0]}"#);
    f.write("cycle.gltf", cycle.as_bytes());
    assert!(load(&f.vfs(), "cycle.gltf").unwrap_err().contains("cycle"));
    let invalid = json.replace(
        r#""byteOffset":0,"byteLength":36}"#,
        r#""byteOffset":0,"byteLength":20}"#,
    );
    f.write("range.gltf", invalid.as_bytes());
    assert!(
        load(&f.vfs(), "range.gltf")
            .unwrap_err()
            .contains("accessor")
    );
}

#[test]
fn sparse_positions_with_and_without_base() {
    for base in [false, true] {
        let f = Fixture::new();
        let (bytes, views, accessor) = if base {
            let mut bytes = triangle();
            bytes.extend_from_slice(&[1, 0, 0, 0]);
            for x in [4.0f32, 5.0, 6.0] {
                bytes.extend_from_slice(&x.to_le_bytes());
            }
            (bytes,
             r#"[{"buffer":0,"byteOffset":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":1},{"buffer":0,"byteOffset":40,"byteLength":12}]"#.to_owned(),
             r#"{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,2],"max":[4,5,6],"sparse":{"count":1,"indices":{"bufferView":1,"componentType":5121},"values":{"bufferView":2}}}"#.to_owned())
        } else {
            let mut bytes = vec![0, 1, 2, 0];
            bytes.extend_from_slice(&triangle());
            (bytes,
             r#"[{"buffer":0,"byteOffset":0,"byteLength":3},{"buffer":0,"byteOffset":4,"byteLength":36}]"#.to_owned(),
             r#"{"componentType":5126,"count":3,"type":"VEC3","min":[0,0,2],"max":[1,1,3],"sparse":{"count":3,"indices":{"bufferView":0,"componentType":5121},"values":{"bufferView":1}}}"#.to_owned())
        };
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let mut json = document(
            Some(&format!("data:application/octet-stream;base64,{b64}")),
            "",
        );
        json = json.replacen(
            r#""byteLength":36}"#,
            &format!(r#""byteLength":{}}}"#, bytes.len()),
            1,
        );
        json = json.replace(
            r#""bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36}]"#,
            &format!(r#""bufferViews":{views}"#),
        );
        json = json.replace(r#""accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,2],"max":[1,1,3]}]"#, &format!(r#""accessors":[{accessor}]"#));
        f.write("sparse.gltf", json.as_bytes());
        let m = load(&f.vfs(), "sparse.gltf").unwrap();
        let positions = &m.nodes[1].mesh().unwrap().positions;
        assert_eq!(positions.len(), 3);
        if base {
            assert_eq!(positions[1], [4.0, 5.0, -6.0]);
        } else {
            assert_eq!(positions[0], [1.0, 0.0, -2.0]);
        }
    }
}

#[test]
fn sparse_rejects_duplicate_and_out_of_range_indices() {
    let f = Fixture::new();
    let mut bytes = vec![0, 0, 3, 0];
    bytes.extend_from_slice(&triangle());
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let mut json = document(
        Some(&format!("data:application/octet-stream;base64,{b64}")),
        "",
    );
    json = json.replacen(r#""byteLength":36}"#, r#""byteLength":40}"#, 1);
    json = json.replace(r#""bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36}]"#,
        r#""bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":3},{"buffer":0,"byteOffset":4,"byteLength":36}]"#);
    json = json.replace(r#""accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,2],"max":[1,1,3]}]"#,
        r#""accessors":[{"componentType":5126,"count":3,"type":"VEC3","min":[0,0,2],"max":[1,1,3],"sparse":{"count":3,"indices":{"bufferView":0,"componentType":5121},"values":{"bufferView":1}}}]"#);
    f.write("bad.gltf", json.as_bytes());
    assert!(
        load(&f.vfs(), "bad.gltf")
            .unwrap_err()
            .contains("sparse indices")
    );
}

#[test]
fn rejects_view_beyond_declared_buffer_even_when_file_is_longer() {
    let f = Fixture::new();
    let mut bytes = triangle();
    bytes.extend_from_slice(&[0; 8]);
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    let json = document(
        Some(&format!("data:application/octet-stream;base64,{b64}")),
        "",
    )
    .replace(
        r#""byteOffset":0,"byteLength":36}"#,
        r#""byteOffset":8,"byteLength":36}"#,
    );
    f.write("long.gltf", json.as_bytes());
    assert!(load(&f.vfs(), "long.gltf").is_err());
}

#[test]
fn rejects_inline_image_mime_mismatch() {
    let f = Fixture::new();
    let b64 = base64::engine::general_purpose::STANDARD.encode(triangle());
    let image_uri = "data:image/jpeg;base64,iVBORw0KGgo=";
    let mut json = document(
        Some(&format!("data:application/octet-stream;base64,{b64}")),
        "",
    );
    json.pop();
    json.push_str(&format!(r#","images":[{{"uri":"{image_uri}"}}]}}"#));
    f.write("mime.gltf", json.as_bytes());
    assert!(
        load(&f.vfs(), "mime.gltf")
            .unwrap_err()
            .contains("encoding does not match MIME")
    );
}

#[test]
fn zero_count_accessor_returns_error_without_panic() {
    let f = Fixture::new();
    let b64 = base64::engine::general_purpose::STANDARD.encode(triangle());
    let json = document(
        Some(&format!("data:application/octet-stream;base64,{b64}")),
        "",
    )
    .replace(r#""count":3"#, r#""count":0"#);
    f.write("zero.gltf", json.as_bytes());
    let result = std::panic::catch_unwind(|| load(&f.vfs(), "zero.gltf"));
    assert!(result.is_ok(), "loader panicked on zero-count accessor");
    assert!(result.unwrap().is_err());
}

#[test]
fn malformed_attribute_and_index_shapes_return_errors_without_panic() {
    let f = Fixture::new();
    let b64 = base64::engine::general_purpose::STANDARD.encode(triangle());
    let original = document(
        Some(&format!("data:application/octet-stream;base64,{b64}")),
        "",
    );
    for (name, primitive, accessor, expected) in [
        (
            "float_index",
            r#""mode":4,"indices":1"#,
            r#""componentType":5126,"count":3,"type":"SCALAR""#,
            "indices accessor",
        ),
        (
            "bad_color",
            r#""attributes":{"POSITION":0,"COLOR_0":1}"#,
            r#""componentType":5126,"count":3,"type":"VEC2""#,
            "Colors(0)",
        ),
        (
            "bad_uv",
            r#""attributes":{"POSITION":0,"TEXCOORD_0":1}"#,
            r#""componentType":5126,"count":3,"type":"VEC3""#,
            "TexCoords(0)",
        ),
    ] {
        let mut json = original.clone();
        if name == "float_index" {
            json = json.replace(r#""mode":4"#, primitive);
        } else {
            json = json.replace(r#""attributes":{"POSITION":0}"#, primitive);
        }
        json = json.replace(
            r#""max":[1,1,3]}]"#,
            &format!(r#""max":[1,1,3]}},{{"bufferView":0,{accessor}}}]"#),
        );
        f.write(&format!("{name}.gltf"), json.as_bytes());
        let result = std::panic::catch_unwind(|| load(&f.vfs(), &format!("{name}.gltf")));
        assert!(result.is_ok(), "loader panicked on {name}");
        assert!(result.unwrap().unwrap_err().contains(expected), "{name}");
    }
}
