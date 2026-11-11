use super::*;
use sta_assets::model::{Model, Node, NodeKind, SourceFormat};

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn test_model() -> Model {
    let mesh = |faces: usize| sta_assets::model::Mesh {
        positions: Vec::new(),
        uvs: Vec::new(),
        groups: vec![sta_assets::model::Group {
            material: "mat\nname".to_owned(),
            blend: "default".to_owned(),
            maps: Default::default(),
            faces: vec![[(0, 0); 3]; faces],
        }],
        two_sided: false,
    };
    Model {
        format: SourceFormat::M3d,
        version: 1.93,
        materials: Vec::new(),
        nodes: vec![
            Node {
                name: "geometry0".to_owned(),
                parent: None,
                children: vec![1, 2, 3],
                transform: a2fmt::Matrix34::IDENTITY,
                kind: NodeKind::Null,
            },
            Node {
                name: "lod0".to_owned(),
                parent: Some(0),
                children: Vec::new(),
                transform: a2fmt::Matrix34::IDENTITY,
                kind: NodeKind::Mesh(mesh(1)),
            },
            Node {
                name: "lod1".to_owned(),
                parent: Some(0),
                children: Vec::new(),
                transform: a2fmt::Matrix34::IDENTITY,
                kind: NodeKind::Mesh(mesh(3)),
            },
            Node {
                name: "control\nnode".to_owned(),
                parent: Some(0),
                children: Vec::new(),
                transform: a2fmt::Matrix34::IDENTITY,
                kind: NodeKind::Lod,
            },
            Node {
                name: "lamp".to_owned(),
                parent: None,
                children: Vec::new(),
                transform: a2fmt::Matrix34::IDENTITY,
                kind: NodeKind::Light(a2fmt::sod::Light {
                    kind: 2,
                    colour: [0.2, 0.4, 0.8],
                    falloff_start: 3.0,
                    falloff_range: 12.0,
                }),
            },
        ],
        channels: Vec::new(),
        flipbooks: Vec::new(),
        warnings: Vec::new(),
    }
}

struct TempFile(std::path::PathBuf);

impl TempFile {
    fn new(bytes: &[u8]) -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("model-info-{}-{nonce}.sod", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        Self(path)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn help_is_successful_and_argument_errors_are_reported() {
    assert!(run(&args(&["--help"])).unwrap().contains("--file <path>"));
    assert!(run(&[]).unwrap().contains("Usage:"));
    assert!(run(&args(&["--file"])).unwrap_err().contains("--file requires"));
    assert!(run(&args(&["--unknown"])).unwrap_err().contains("expected <install>"));
}

#[test]
fn report_escapes_names_and_describes_typed_lod_and_light() {
    let output = report_model(&test_model(), "fixture", None);
    assert!(output.contains("name=\"control\\nnode\""));
    assert!(output.contains("kind=LOD control (typed)"));
    assert!(output.contains("lod0\": hidden coarse"));
    assert!(output.contains("lod1\": selected finest"));
    assert!(output.contains("subtype=2 rgb=[0.2, 0.4, 0.8] falloff_start=3 falloff_range=12"));
    assert!(output.contains("parsed data; renderer light playback unavailable"));
    assert!(output.contains("material=\"mat\\nname\""));
}

#[test]
fn file_mode_reports_a_contextual_parser_error_for_malformed_sod() {
    let file = TempFile::new(b"Storm3D_SW");
    let message = run(&args(&["--file", file.0.to_str().unwrap()])).unwrap_err();
    assert!(message.contains(&file.0.display().to_string()));
    assert!(message.contains("parsing SOD"));
}
