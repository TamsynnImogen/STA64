//! Authored SOD fixtures for repair attachment geometry; no game files.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use sta_content::{ContentDb, Fixed, OdfIndex, Profile, RepairAttachment, UnitDef, Vec3, Warning};
use sta_vfs::Vfs;

static NEXT: AtomicU64 = AtomicU64::new(0);
const GLB: &[u8] = include_bytes!("../../../fixtures/models/fx_cruiser.glb");
const ROOT: [f32; 12] = [
    0.0, 1.0, 0.0, -2.0, 0.0, 0.0, 0.0, 0.0, 1.0, 10.0, 20.0, 30.0,
];
const DOCK: [f32; 12] = [2.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 4.0, 1.0, 2.0, 3.0];

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("sta_repair_attachment_{}_{id}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn write(&self, path: &str, data: &[u8]) {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, data).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn text(bytes: &mut Vec<u8>, s: &str) {
    bytes.extend_from_slice(&(s.len() as u16).to_le_bytes());
    bytes.extend_from_slice(s.as_bytes());
}

fn sod(root: [f32; 12], dock: [f32; 12]) -> Vec<u8> {
    let mut bytes = b"Storm3D_SW".to_vec();
    bytes.extend_from_slice(&1.93f32.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes()); // lighting materials
    bytes.extend_from_slice(&2u16.to_le_bytes()); // null nodes
    for (name, parent, matrix) in [("root", "", root), ("RepairDock", "root", dock)] {
        bytes.extend_from_slice(&0u16.to_le_bytes());
        text(&mut bytes, name);
        text(&mut bytes, parent);
        for value in matrix {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&0u16.to_le_bytes()); // channels
    bytes.extend_from_slice(&0u16.to_le_bytes()); // animation references
    bytes
}

fn mount(fixtures: &[&Fixture]) -> Vfs {
    let mut vfs = Vfs::default();
    for (i, fixture) in fixtures.iter().enumerate() {
        vfs.push_dir(format!("layer{i}"), fixture.0.clone())
            .unwrap();
    }
    vfs
}

fn load(vfs: &Vfs) -> (ContentDb, Vec<Warning>) {
    let (index, _) = OdfIndex::build(vfs);
    let (db, warnings) = ContentDb::load_as(vfs, Profile::Native, &index).unwrap();
    // These isolated fixtures intentionally have no skirmish tech tree.
    (
        db,
        warnings.into_iter().filter(|w| w.file != "tech1").collect(),
    )
}

fn unit(db: &ContentDb) -> &UnitDef {
    db.unit(db.key("ship").unwrap()).unwrap()
}

fn fx(n: f64) -> Fixed {
    Fixed::from_f64(n)
}

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 {
        x: fx(x),
        y: fx(y),
        z: fx(z),
    }
}

fn fixture(odf: &str, model: Option<&[u8]>) -> Fixture {
    let f = Fixture::new();
    f.write("odf/ship.odf", odf.as_bytes());
    if let Some(model) = model {
        f.write("sod/fx_ship.sod", model);
    }
    f
}

fn odf(key: &str, scale: f32) -> String {
    format!("classLabel = craft\nunitName = Ship\nbasename = fx_ship\nScaleSOD = {scale}\n{key}")
}

#[test]
fn composed_full_transform_preserves_y_rotation_and_nonuniform_basis() {
    let model = sod(ROOT, DOCK);
    let f = fixture(&odf("repairHardpoint = REPAIRDOCK\n", 7.0), Some(&model));
    let (db, warnings) = load(&mount(&[&f]));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(unit(&db).scale, fx(7.0));
    assert_eq!(
        unit(&db).repair_attachment,
        Some(RepairAttachment {
            origin: v(6.0, 21.0, 33.0),
            axes: [v(0.0, 2.0, 0.0), v(-6.0, 0.0, 0.0), v(0.0, 0.0, 4.0)],
        })
    );
    let f2 = fixture(&odf("repairHardpoint = RepairDock\n", 1.0), Some(&model));
    assert_eq!(
        unit(&load(&mount(&[&f2])).0).repair_attachment,
        unit(&db).repair_attachment,
        "external ScaleSOD stays separate"
    );
    let root = fixture(&odf("repairHardpoint = root\n", 1.0), Some(&model));
    assert_eq!(
        unit(&load(&mount(&[&root])).0)
            .repair_attachment
            .unwrap()
            .origin,
        v(10.0, 20.0, 30.0)
    );
}

#[test]
fn missing_node_warns_but_absent_key_or_unreadable_model_stays_quiet() {
    let model = sod(ROOT, DOCK);
    let absent = fixture(&odf("", 1.0), Some(&model));
    let (db, warnings) = load(&mount(&[&absent]));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(unit(&db).repair_attachment, None);

    let missing_model = fixture(&odf("repairHardpoint = RepairDock\n", 1.0), None);
    let (db, warnings) = load(&mount(&[&missing_model]));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(unit(&db).repair_attachment, None);

    let bad_model = fixture(
        &odf("repairHardpoint = RepairDock\n", 1.0),
        Some(b"not a sod"),
    );
    let (db, warnings) = load(&mount(&[&bad_model]));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(unit(&db).repair_attachment, None);

    let missing_node = fixture(&odf("repairHardpoint = absent\n", 1.0), Some(&model));
    let (db, warnings) = load(&mount(&[&missing_node]));
    assert_eq!(unit(&db).repair_attachment, None);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].message.contains("repairHardpoint absent"));
}

#[test]
fn invalid_matrix_warns_and_clears_attachment() {
    for value in [f32::INFINITY, 2147483648.0] {
        let mut dock = DOCK;
        dock[10] = value;
        let f = fixture(
            &odf("repairHardpoint = RepairDock\n", 1.0),
            Some(&sod(ROOT, dock)),
        );
        let (db, warnings) = load(&mount(&[&f]));
        assert_eq!(unit(&db).repair_attachment, None);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].message.contains("unrepresentable transform"));
    }
    // Both source matrices are individually finite and in Fixed's range;
    // their composed basis exceeds it.
    let mut parent = ROOT;
    parent[0] = 1.0e6;
    let mut child = DOCK;
    child[0] = 1.0e6;
    let f = fixture(
        &odf("repairHardpoint = RepairDock\n", 1.0),
        Some(&sod(parent, child)),
    );
    let (db, warnings) = load(&mount(&[&f]));
    assert_eq!(unit(&db).repair_attachment, None);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].message.contains("unrepresentable transform"));
}

#[test]
fn finite_degenerate_basis_is_retained_as_authored() {
    let mut dock = [0.0; 12];
    dock[9..12].copy_from_slice(&[1.0, 2.0, 3.0]);
    let f = fixture(
        &odf("repairHardpoint = RepairDock\n", 1.0),
        Some(&sod(ROOT, dock)),
    );
    let (db, warnings) = load(&mount(&[&f]));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        unit(&db).repair_attachment,
        Some(RepairAttachment {
            origin: v(6.0, 21.0, 33.0),
            axes: [Vec3::ZERO; 3],
        })
    );
}

#[test]
fn recompute_clears_stale_attachment_and_render_override_cannot_replace_sod() {
    let model = sod(ROOT, DOCK);
    let base = fixture(&odf("repairHardpoint = RepairDock\n", 1.0), Some(&model));
    let (db, warnings) = load(&mount(&[&base]));
    assert!(warnings.is_empty(), "{warnings:?}");
    let initial = unit(&db).repair_attachment;
    let override_layer = Fixture::new();
    override_layer.write("sod/FX_SHIP.GLB", GLB);
    let (with_render, warnings) = load(&mount(&[&override_layer, &base]));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(unit(&with_render).repair_attachment, initial);

    let no_model = fixture(&odf("repairHardpoint = RepairDock\n", 1.0), None);
    let (again, warnings) = db.with_launch_points_warned(&mount(&[&no_model]));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(unit(&again).repair_attachment, None);
}

#[test]
fn repair_transform_changes_digest_without_radius_change_or_empty_hash_change() {
    assert_eq!(sta_content::fake::db().hash(), 0x9ef5_3881_3050_10f8);
    let odf = odf("repairHardpoint = RepairDock\n", 1.0);
    let plain = fixture(&odf, None);
    let (empty, _) = load(&mount(&[&plain]));
    let base = fixture(&odf, Some(&sod(ROOT, DOCK)));
    let (first, _) = load(&mount(&[&base]));
    assert_ne!(empty.hash(), first.hash());
    for component in 0..12 {
        let mut changed = DOCK;
        changed[component] += 1.0;
        let variant = fixture(&odf, Some(&sod(ROOT, changed)));
        let (next, warnings) = load(&mount(&[&variant]));
        assert!(warnings.is_empty(), "component {component}: {warnings:?}");
        assert_eq!(
            unit(&first).radius,
            unit(&next).radius,
            "component {component}"
        );
        assert_ne!(first.hash(), next.hash(), "component {component}");
        assert_eq!(
            first.digest().first_difference(next.digest()).unwrap().key,
            "repair-geometry:ship",
            "component {component}"
        );
    }
}
