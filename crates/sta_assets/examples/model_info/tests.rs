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
    // Build through the public constructor so optional Model annotations do
    // not make this inspector fixture depend on a struct's private evolution.
    let mut empty = b"Storm3D_SW".to_vec();
    empty.extend_from_slice(&1.93_f32.to_le_bytes());
    empty.extend_from_slice(&[0; 8]);
    let mut model = Model::from_sod(&a2fmt::sod::Sod::parse(&empty).unwrap());
    model.format = SourceFormat::M3d;
    model.nodes = vec![
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
    ];
    model
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
    assert!(message.contains(&format!("{:?}", file.0.to_str().unwrap())));
    assert!(message.contains("parsing SOD"));
}

fn header(version: f32) -> Vec<u8> {
    let mut bytes = b"Mad3D_SW\0\0".to_vec();
    bytes.extend_from_slice(&version.to_le_bytes());
    bytes
}

#[test]
fn chunk_modes_validate_arguments_and_parse_minimal_empty_streams() {
    assert!(run(&args(&["--help"])).unwrap().contains("--chunk-materials <path>"));
    assert!(
        run(&args(&["--chunk-materials"]))
            .unwrap_err()
            .contains("--chunk-materials requires")
    );
    assert!(
        run(&args(&["--chunk-geometry"]))
            .unwrap_err()
            .contains("--chunk-geometry requires")
    );

    let mut material_bytes = header(2.04);
    material_bytes.extend_from_slice(&0u32.to_le_bytes());
    material_bytes.extend_from_slice(&1u32.to_le_bytes());
    let material_file = TempFile::new(&material_bytes);
    let report = run(&args(&["--chunk-materials", material_file.0.to_str().unwrap()])).unwrap();
    assert!(report.contains("chunk materials v2.04 records=0 variants=1"));

    let mut geometry_bytes = header(2.04);
    for _ in 0..8 {
        geometry_bytes.extend_from_slice(&0.0f32.to_le_bytes());
    }
    geometry_bytes.extend_from_slice(&0u32.to_le_bytes());
    let geometry_file = TempFile::new(&geometry_bytes);
    let report = run(&args(&["--chunk-geometry", geometry_file.0.to_str().unwrap()])).unwrap();
    assert!(report.contains("chunk geometry v2.04 bounds=[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] chunks=0"));
    assert!(report.contains("external material library required; native destruction/variant playback not implemented"));
}

#[test]
fn chunk_reports_escape_strings_and_preserve_authored_indices_and_flags() {
    use sta_assets::m3d::chunks::{
        ChunkGroup, ChunkMaterial, ChunkMesh, GeometryVariant, MaterialLibrary, TextureBinding,
    };
    let material = MaterialLibrary {
        version: 2.06,
        materials: vec![ChunkMaterial {
            lighting_material: b"light\nname".to_vec(),
            textures: std::array::from_fn(|index| TextureBinding {
                reference: format!("tex{index}\nname").into_bytes(),
                flags: index as u32 + 0x10,
            }),
            shader: b"shader\tname".to_vec(),
            extra_shader: b"extra".to_vec(),
        }],
        variant_count: 3,
    };
    let report = report_chunk_materials(&material, "test\nfile");
    assert!(report.contains("test\\nfile"));
    assert!(report.contains("lighting_material=\"light\\nname\""));
    assert!(report.contains("reference=\"tex0\\nname\" flags=0x00000010"));
    assert!(report.contains("shader=\"shader\\tname\""));

    let geometry = GeometryVariant {
        version: 2.06,
        bounds: [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
        chunks: vec![ChunkMesh {
            special_flag: 0x1234,
            flags: 0x10,
            trailer: 0x4321,
            positions: vec![],
            normals: vec![],
            uvs: vec![],
            groups: vec![ChunkGroup {
                material_index: -1,
                faces: vec![],
            }],
            cull: -1,
        }],
    };
    let report = report_chunk_geometry(&geometry, "test");
    assert!(report.contains("special_flag=0x00001234 flags=0x00000010 trailer=0x4321 cull=-1"));
    assert!(report.contains("group #0 material_index=-1 faces=0"));
}

#[test]
fn chunk_modes_report_contextual_parser_errors() {
    let file = TempFile::new(b"Mad3D_SW\0\0");
    let materials = run(&args(&["--chunk-materials", file.0.to_str().unwrap()])).unwrap_err();
    assert!(materials.contains("parsing chunk material library"));
    assert!(materials.contains(&format!("{:?}", file.0.to_str().unwrap())));
    let geometry = run(&args(&["--chunk-geometry", file.0.to_str().unwrap()])).unwrap_err();
    assert!(geometry.contains("parsing chunk geometry"));
    assert!(geometry.contains(&format!("{:?}", file.0.to_str().unwrap())));
}
