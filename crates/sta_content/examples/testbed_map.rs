//! Write a hand-authored map (a small JSON5 spec) as an Armada II binary BZN,
//! for the alpha testbed mod (TOOL-20, docs/ALPHA_TESTBED.md).
//!
//! `cargo run -p sta_content --example testbed_map -- <spec.json5>... --out <bzn dir>`
//!
//! Spec (all lengths in world units; the map runs from 0 to `size` on x and z,
//! the y range is centred on 0):
//!
//! ```json5
//! {
//!   file: "tb_empty",                 // written as <out>/tb_empty.bzn
//!   description: "Empty test map",
//!   mission_type: "mp-regular",       // or "sp-mission" for a scripted map
//!   size: 12000, height: 2500,
//!   background: "tb_sky.sod", starfield: "tb_stars.odf",
//!   tech_tree: "testbed.tt", race: "terran",
//!   ambient: [0.25, 0.25, 0.3],
//!   starts: [[2000, 6000], [10000, 6000]],      // x, z; seat n = entry n
//!   objects: [{odf: "tb_moon", team: 0, pos: [6000, 0, 6000], heading: [0, 1], label: "moon"}],
//!   lights: [[0, -0.7071, 0.7071], [0, 0.7071, -0.7071]],  // directions; default: these two
//! }
//! ```
//!
//! Each file is read back with `sta_bzn::parse` and must give the same map.

use serde::Deserialize;

use sta_bzn::{Map, MapObject, StartLoc};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Spec {
    file: String,
    #[serde(default)]
    description: String,
    #[serde(default = "mp")]
    mission_type: String,
    size: f32,
    #[serde(default = "height")]
    height: f32,
    #[serde(default)]
    background: String,
    #[serde(default)]
    starfield: String,
    #[serde(default)]
    tech_tree: String,
    #[serde(default)]
    race: String,
    #[serde(default = "ambient")]
    ambient: [f32; 3],
    #[serde(default)]
    scripts: Vec<String>,
    starts: Vec<[f32; 2]>,
    #[serde(default)]
    objects: Vec<Obj>,
    #[serde(default = "lights")]
    lights: Vec<[f32; 3]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Obj {
    odf: String,
    #[serde(default)]
    team: i32,
    #[serde(default)]
    label: String,
    pos: [f32; 3],
    /// Facing on the x-z plane; default +z.
    #[serde(default = "north")]
    heading: [f32; 2],
}

fn mp() -> String {
    "mp-regular".into()
}
fn height() -> f32 {
    2500.0
}
fn ambient() -> [f32; 3] {
    [0.25; 3]
}
fn north() -> [f32; 2] {
    [0.0, 1.0]
}
fn lights() -> Vec<[f32; 3]> {
    let h = std::f32::consts::FRAC_1_SQRT_2;
    vec![[0.0, -h, h], [0.0, h, -h]]
}

fn normalise(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l == 0.0 { [0.0, 0.0, 1.0] } else { [v[0] / l, v[1] / l, v[2] / l] }
}

/// Right, up, front, position for a facing `front` with world up (0,1,0),
/// or for a light pointing anywhere.
fn transform(front: [f32; 3], pos: [f32; 3]) -> [f32; 12] {
    let f = normalise(front);
    // right = up x front with up = +y; a vertical front takes +x as right.
    let r = if f[0].abs() < 1e-6 && f[2].abs() < 1e-6 { [1.0, 0.0, 0.0] } else { normalise([f[2], 0.0, -f[0]]) };
    // up = front x right
    let u = [f[1] * r[2] - f[2] * r[1], f[2] * r[0] - f[0] * r[2], f[0] * r[1] - f[1] * r[0]];
    [r[0], r[1], r[2], u[0], u[1], u[2], f[0], f[1], f[2], pos[0], pos[1], pos[2]]
}

fn build(spec: &Spec) -> Result<Map, String> {
    if spec.starts.is_empty() || spec.starts.len() > 8 {
        return Err(format!("{}: 1 to 8 starts, found {}", spec.file, spec.starts.len()));
    }
    let centre = [spec.size / 2.0, 0.0, spec.size / 2.0];
    let loc = |pos: [f32; 3], kind: &str| StartLoc { pos, kind: kind.to_owned(), team: -1, race: String::new() };
    let mut start_locs = vec![loc(centre, "empty")];
    start_locs.extend(spec.starts.iter().map(|s| loc([s[0], 0.0, s[1]], "player")));
    start_locs.resize(9, loc(centre, ""));

    let mut objects = Vec::new();
    for o in &spec.objects {
        let id = objects.len() as i32 + 1;
        objects.push(MapObject {
            odf: o.odf.clone(),
            entity_id: id,
            team: o.team,
            label: if o.label.is_empty() { format!("{}{id}", o.odf) } else { o.label.clone() },
            instance_name: String::new(),
            initial_position: Some(o.pos),
            transform: transform([o.heading[0], 0.0, o.heading[1]], o.pos),
            light_colour: None,
        });
    }
    for (n, d) in spec.lights.iter().enumerate() {
        objects.push(MapObject {
            odf: "dlight".into(),
            entity_id: objects.len() as i32 + 1,
            team: 0,
            label: format!("dlight{}", n + 1),
            instance_name: String::new(),
            initial_position: Some(centre),
            transform: transform(*d, centre),
            light_colour: None,
        });
    }
    Ok(Map {
        version: 2167,
        binary: true,
        mission_type: spec.mission_type.clone(),
        description: spec.description.clone(),
        file_name: format!("{}.bzn", spec.file),
        tech_tree: spec.tech_tree.clone(),
        scripts: spec.scripts.clone(),
        min_extent: [0.0, -spec.height / 2.0, 0.0],
        size: [spec.size, spec.height, spec.size],
        ambient: spec.ambient,
        background: spec.background.clone(),
        starfield_odf: spec.starfield.clone(),
        tech_tree_file: spec.tech_tree.clone(),
        race: spec.race.clone(),
        start_count: spec.starts.len() as i32,
        start_locs,
        objects,
        team_colours: Vec::new(),
        paths: Vec::new(),
        a1: None,
    })
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let out = match args.iter().position(|a| a == "--out") {
        Some(i) if i + 1 < args.len() => {
            let o = args.remove(i + 1);
            args.remove(i);
            std::path::PathBuf::from(o)
        }
        _ => {
            eprintln!("usage: testbed_map <spec.json5>... --out <bzn dir>");
            std::process::exit(2);
        }
    };
    std::fs::create_dir_all(&out).expect("create the output folder");
    let mut failed = false;
    for path in &args {
        let result = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|t| json5::from_str::<Spec>(&t).map_err(|e| e.to_string()))
            .and_then(|spec| build(&spec))
            .and_then(|map| {
                let bytes = sta_bzn::write::write(&map).map_err(|e| e.to_string())?;
                let back = sta_bzn::parse(&bytes).map_err(|e| format!("does not read back: {e}"))?;
                if back.objects.len() != map.objects.len() || back.start_count != map.start_count {
                    return Err("reads back different".into());
                }
                let file = out.join(&map.file_name);
                std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
                Ok(format!("{} ({} starts, {} objects)", file.display(), map.start_count, map.objects.len()))
            });
        match result {
            Ok(s) => println!("wrote {s}"),
            Err(e) => {
                eprintln!("{path}: {e}");
                failed = true;
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
}
