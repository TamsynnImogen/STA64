use super::*;
use sta_assets::a2fmt::Matrix34;
use sta_assets::model::{Group, Maps, Mesh, Node, NodeKind, SourceFormat};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Tree(PathBuf);

impl Tree {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "sta_model_textures_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn put(&self, relative: &str, contents: &[u8]) {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn vfs(&self) -> Vfs {
        let mut vfs = Vfs::default();
        vfs.push_dir("root".into(), self.0.clone()).unwrap();
        vfs
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn model(groups: Vec<Group>) -> Model {
    Model {
        format: SourceFormat::Sod,
        version: 1.7,
        materials: vec![],
        nodes: vec![
            Node {
                name: "null".into(),
                parent: None,
                children: vec![],
                transform: Matrix34::IDENTITY,
                kind: NodeKind::Null,
            },
            Node {
                name: "mesh".into(),
                parent: None,
                children: vec![],
                transform: Matrix34::IDENTITY,
                kind: NodeKind::Mesh(Mesh {
                    positions: vec![],
                    uvs: vec![],
                    groups,
                    two_sided: false,
                }),
            },
        ],
        channels: vec![],
        flipbooks: vec![],
        warnings: vec!["warning\nnext".into()],
        pbr: None,
    }
}

fn group(material: &str, maps: Maps) -> Group {
    Group {
        material: material.into(),
        blend: "default".into(),
        maps,
        faces: vec![],
    }
}

#[test]
fn native_pbr_bindings_report_encoded_sources_and_missing_indices() {
    use sta_assets::model::pbr::*;
    let mut model = model(vec![group("native", Maps::default())]);
    model.format = SourceFormat::Gltf;
    let mut pbr = PbrModel::default();
    pbr.images.push(PbrImage { name: None, origin: "buffer view 2\nnot another row".into(),
        format: PbrImageFormat::Png, data: vec![1,2,3], content_hash: [0;32] });
    pbr.materials.insert("native".into(), PbrMaterial {
        base_color_texture: Some(PbrTexture {image:0, tex_coord:1, sampler:PbrSampler::default()}),
        normal_texture: Some(PbrTexture {image:8, tex_coord:0, sampler:PbrSampler::default()}),
        ..Default::default()
    });
    model.pbr = Some(pbr);
    let report = report(&Vfs::default(), "sod/box.glb", &model);
    assert_eq!(report.missing,1);
    assert!(report.output.contains("base_color image=0"));
    assert!(report.output.contains("encoded_bytes=3 uv=1"));
    assert!(report.output.contains("(decode not checked)"));
    assert!(report.output.contains("buffer view 2\\nnot another row"));
    assert!(report.output.contains("normal image=8 missing"));
}

#[test]
fn six_roles_in_model_order_and_preserved_references() {
    let tree = Tree::new();
    for name in ["d", "h", "b", "s", "g", "t"] {
        tree.put(&format!("Textures/{name}.png"), b"not decoded");
    }
    let maps = Maps {
        diffuse: Some("D.TGA".into()),
        height: Some("h".into()),
        bump: Some("b".into()),
        specular: Some("s".into()),
        glow: Some("g".into()),
        detail: Some("t".into()),
    };
    let report = report(&tree.vfs(), "sod/test.sod", &model(vec![group("first", maps)]));
    assert_eq!(report.missing, 0);
    let roles: Vec<&str> = report
        .output
        .lines()
        .filter(|l| l.starts_with("  "))
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(roles, ["diffuse", "height", "bump", "specular", "glow", "detail"]);
    assert!(
        report
            .output
            .contains("node=1 name=\"mesh\" group=0 material=\"first\"")
    );
    assert!(report.output.contains("diffuse ref=\"D.TGA\" path=\"textures/d.png\""));
}

#[test]
fn repeated_and_empty_maps_remain_distinct_occurrences() {
    let tree = Tree::new();
    tree.put("textures/shared.tga", b"x");
    let groups = vec![
        group(
            "one",
            Maps {
                diffuse: Some("shared".into()),
                height: Some("".into()),
                ..Default::default()
            },
        ),
        group(
            "two",
            Maps {
                diffuse: Some("shared".into()),
                ..Default::default()
            },
        ),
    ];
    let report = report(&tree.vfs(), "sod/test.sod", &model(groups));
    assert_eq!(report.missing, 1);
    assert_eq!(report.output.matches("diffuse ref=\"shared\"").count(), 2);
    assert!(report.output.contains("height ref=\"\" missing"));
    assert!(report.output.contains("group=0 material=\"one\""));
    assert!(report.output.contains("group=1 material=\"two\""));
}

#[test]
fn lookup_reports_winning_layer_before_format_and_case_insensitive_format_rank() {
    let tree = Tree::new();
    tree.put("Data/Textures/Ship.ktx2", b"x");
    tree.put("Data/Textures/Second.tga", b"x");
    tree.put("Data/Textures/Second.DDS", b"x");
    tree.put("Data/Mods/Child/info.ini", b"[Mod]\nParentMod = Parent\n");
    tree.put("Data/Mods/Parent/info.ini", b"");
    tree.put("Data/Mods/Parent/Textures/SHIP.dds", b"x");
    tree.put("Data/Mods/Child/Textures/RGB/sHiP.tga", b"x");
    let vfs = Vfs::mount_install(&tree.0, Some("Child")).unwrap();
    let report = report(
        &vfs,
        "sod/test.sod",
        &model(vec![group(
            "m",
            Maps {
                diffuse: Some("SHIP".into()),
                height: Some("second".into()),
                ..Default::default()
            },
        )]),
    );
    assert!(
        report
            .output
            .contains("path=\"textures/rgb/ship.tga\" format=Tga layer=0 layer_name=\"Child\"")
    );
    assert!(
        report
            .output
            .contains("path=\"textures/second.dds\" format=Dds layer=2 layer_name=\"Data\"")
    );
}

#[test]
fn empty_model_maps_are_valid_and_output_escapes_row_breaks() {
    let tree = Tree::new();
    let mut m = model(vec![group(
        "mat\nforge",
        Maps {
            diffuse: Some("bad\nmap".into()),
            ..Default::default()
        },
    )]);
    m.nodes[1].name = "node\nforge".into();
    let escaped = report(&tree.vfs(), "sod/test.sod", &m);
    assert_eq!(escaped.missing, 1);
    assert_eq!(escaped.output.lines().count(), 4);
    assert!(escaped.output.contains("warning \"warning\\nnext\""));
    assert!(escaped.output.contains("name=\"node\\nforge\""));
    assert!(escaped.output.contains("ref=\"bad\\nmap\" missing"));
    assert_eq!(
        report(
            &tree.vfs(),
            "sod/empty.sod",
            &model(vec![group("empty", Maps::default())])
        )
        .missing,
        0
    );
    assert_eq!(report(&tree.vfs(), "sod/none.sod", &model(vec![])).missing, 0);
}

#[test]
fn argument_forms_and_invalid_arguments() {
    let parse = |args: &[&str]| parse_args(args.iter().map(|s| (*s).to_owned()));
    assert_eq!(parse(&["--help"]).unwrap(), None);
    let install = parse(&["--deny-missing", "install", "model", "mod"]).unwrap().unwrap();
    assert!(install.deny_missing);
    assert_eq!(install.active_mod.as_deref(), Some("mod"));
    let root = parse(&["--root", "assets", "--", "-ship"]).unwrap().unwrap();
    assert!(root.loose_root);
    assert_eq!(root.model, "-ship");
    for args in [
        &["--bogus"][..],
        &["--root"],
        &["--root", "assets"],
        &["install"],
        &["a", "b", "c", "d"],
    ] {
        assert!(parse(args).is_err(), "{args:?}");
    }
}

#[test]
fn mount_missing_and_corrupt_model_errors() {
    let tree = Tree::new();
    let options = |root: PathBuf, model: &str| Options {
        root,
        model: model.into(),
        active_mod: None,
        loose_root: true,
        deny_missing: false,
    };
    assert!(
        run(&options(tree.0.join("absent"), "ship"))
            .unwrap_err()
            .contains("mount")
    );
    assert!(run(&options(tree.0.clone(), "ship")).unwrap_err().contains("not found"));
    tree.put("sod/ship.glb", b"not glb");
    assert!(
        run(&options(tree.0.clone(), "ship"))
            .unwrap_err()
            .contains("glTF parsing")
    );
    std::fs::remove_file(tree.0.join("sod/ship.glb")).unwrap();
    tree.put("sod/ship.sod", b"corrupt");
    assert!(run(&options(tree.0.clone(), "ship")).unwrap_err().contains("SOD"));
}
