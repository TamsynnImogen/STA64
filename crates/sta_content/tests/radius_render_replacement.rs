//! A render replacement (glTF/GLB/M3D) must not take the unit's collision
//! body away: the radius and the content hash come from the unit's SOD in the
//! mounted layers, whichever model the renderer picks. Synthetic fixtures only.

use std::path::{Path, PathBuf};

use sta_content::launch::FALLBACK_RADIUS;
use sta_content::{ContentDb, Fixed, OdfIndex, Profile, Warning};
use sta_vfs::Vfs;

const GLB: &[u8] = include_bytes!("../../../fixtures/models/fx_cruiser.glb");
const ODF: &str =
    "classLabel = \"craft\"\nunitName = \"Ship\"\nbasename = \"fx_ship\"\nScaleSOD = 2.0\n";

fn s(out: &mut Vec<u8>, t: &str) {
    out.extend_from_slice(&(t.len() as u16).to_le_bytes());
    out.extend_from_slice(t.as_bytes());
}
fn f(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn h(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// A minimal SOD 1.93: a root node and a one-triangle mesh `size` wide.
fn sod(size: f32) -> Vec<u8> {
    let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
    let mut d = b"Storm3D_SW".to_vec();
    f(&mut d, 1.93);
    h(&mut d, 1);
    s(&mut d, "team");
    for _ in 0..10 {
        f(&mut d, 0.5);
    }
    d.push(2);
    d.push(1);
    h(&mut d, 2);
    h(&mut d, 0);
    s(&mut d, "root");
    s(&mut d, "");
    identity.iter().for_each(|v| f(&mut d, *v));
    h(&mut d, 1);
    s(&mut d, "hull");
    s(&mut d, "ROOT");
    identity.iter().for_each(|v| f(&mut d, *v));
    s(&mut d, "opaque");
    d.extend_from_slice(&4u32.to_le_bytes());
    d.extend_from_slice(&1u32.to_le_bytes());
    s(&mut d, "fhull");
    d.extend_from_slice(&0u32.to_le_bytes());
    s(&mut d, "");
    h(&mut d, 0);
    h(&mut d, 3);
    h(&mut d, 2);
    h(&mut d, 1);
    for p in [[0.0, 0.0, 0.0], [size, 0.0, 0.0], [0.0, size, 0.0]] {
        p.iter().for_each(|c| f(&mut d, *c));
    }
    for t in [[0.0, 0.0], [1.0, 1.0]] {
        t.iter().for_each(|c| f(&mut d, *c));
    }
    h(&mut d, 1);
    s(&mut d, "team");
    for (p, t) in [(0, 0), (1, 1), (2, 0)] {
        h(&mut d, p);
        h(&mut d, t);
    }
    d.push(1);
    h(&mut d, 0);
    h(&mut d, 0);
    h(&mut d, 0);
    d
}

/// One layer directory: `files` are (relative path, bytes).
fn layer(tag: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sta_radius_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (rel, bytes) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }
    dir
}

/// The ship's radius, content hash and radius warnings over `layers`
/// (highest priority first).
fn load(layers: &[&Path]) -> (Fixed, u64, Vec<Warning>) {
    let mut vfs = Vfs::default();
    for (i, l) in layers.iter().enumerate() {
        vfs.push_dir(format!("l{i}"), l.to_path_buf()).unwrap();
    }
    let (index, _) = OdfIndex::build(&vfs);
    let (db, warnings) = ContentDb::load_as(&vfs, Profile::Native, &index).unwrap();
    let r = db.unit(db.key("ship").unwrap()).unwrap().radius;
    let warnings = warnings
        .into_iter()
        .filter(|w| w.file.starts_with("sod/"))
        .collect();
    (r, db.hash(), warnings)
}

fn base(tag: &str, extra: &[(&str, &[u8])]) -> PathBuf {
    let sod = sod(10.0);
    let mut files: Vec<(&str, &[u8])> =
        vec![("odf/ship.odf", ODF.as_bytes()), ("sod/fx_ship.sod", &sod)];
    files.extend_from_slice(extra);
    layer(tag, &files)
}

#[test]
fn render_replacement_keeps_the_sod_radius_and_hash() {
    let plain = base("plain", &[]);
    let (r0, hash0, w0) = load(&[&plain]);
    assert!(r0 > Fixed::ZERO, "the SOD alone gives a radius");
    assert!(w0.is_empty(), "{w0:?}");

    // glb and glTF beside the SOD in the same layer.
    let same = base(
        "same",
        &[("sod/fx_ship.glb", GLB), ("sod/fx_ship.gltf", b"{}")],
    );
    assert_eq!(load(&[&same]), (r0, hash0, vec![]));

    // A higher layer supplies the replacement only (any case).
    let below = base("below", &[]);
    let above = layer("above_glb", &[("sod/FX_SHIP.GLB", GLB)]);
    assert_eq!(load(&[&above, &below]), (r0, hash0, vec![]));
    let above = layer("above_gltf", &[("sod/fx_ship.gltf", b"{}")]);
    assert_eq!(load(&[&above, &below]), (r0, hash0, vec![]));

    // An M3D that is not a SOD does the same; one that holds a SOD is the model.
    let above = layer("above_m3d", &[("sod/fx_ship.m3d", b"not a sod")]);
    assert_eq!(load(&[&above, &below]), (r0, hash0, vec![]));
    let bigger = sod(20.0);
    let above = layer("above_m3d_sod", &[("sod/fx_ship.m3d", &bigger)]);
    let (r, hash, w) = load(&[&above, &below]);
    assert!(r > r0 && w.is_empty() && hash != hash0);
}

#[test]
fn render_only_unit_warns_and_takes_the_fallback() {
    let only = layer(
        "only",
        &[("odf/ship.odf", ODF.as_bytes()), ("sod/fx_ship.glb", GLB)],
    );
    let (r, _, w) = load(&[&only]);
    assert_eq!(r, Fixed::from_int(FALLBACK_RADIUS));
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(
        w[0].file == "sod/fx_ship.sod" && w[0].message.contains("ship"),
        "{w:?}"
    );
}

#[test]
fn unit_with_no_model_keeps_zero_without_a_warning() {
    let none = layer("none", &[("odf/ship.odf", ODF.as_bytes())]);
    let (r, _, w) = load(&[&none]);
    assert_eq!(r, Fixed::ZERO);
    assert!(w.is_empty(), "{w:?}");
}
