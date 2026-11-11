//! Inspect a model from the install VFS or a single user-owned model file.
//! `cargo run -p sta_assets --example model_info -- <install> <mod> <model>`
//! `cargo run -p sta_assets --example model_info -- --file <path>`

use std::fmt::Write as _;
use std::path::Path;

fn usage() -> &'static str {
    "Usage:\n  model_info <install> <mod> <model>\n  model_info --file <path>\n  model_info --chunk-materials <path>\n  model_info --chunk-geometry <path>\n"
}

fn run(args: &[String]) -> Result<String, String> {
    if args.is_empty() || args == ["--help"] || args == ["-h"] {
        return Ok(usage().to_owned());
    }
    if args.first().is_some_and(|arg| arg == "--file") {
        if args.len() != 2 || args[1].starts_with('-') {
            return Err(format!("--file requires exactly one path\n{}", usage()));
        }
        return report_file(Path::new(&args[1]));
    }
    if args.first().is_some_and(|arg| arg == "--chunk-materials") {
        if args.len() != 2 || args[1].starts_with('-') {
            return Err(format!("--chunk-materials requires exactly one path\n{}", usage()));
        }
        return report_chunk_material_file(Path::new(&args[1]));
    }
    if args.first().is_some_and(|arg| arg == "--chunk-geometry") {
        if args.len() != 2 || args[1].starts_with('-') {
            return Err(format!("--chunk-geometry requires exactly one path\n{}", usage()));
        }
        return report_chunk_geometry_file(Path::new(&args[1]));
    }
    if args.len() != 3 {
        return Err(format!("expected <install> <mod> <model>\n{}", usage()));
    }

    let vfs = sta_vfs::Vfs::mount_install(Path::new(&args[0]), Some(&args[1]))
        .map_err(|e| format!("install {:?}, mod {:?}: {e}", args[0], args[1]))?;
    let (path, format) = sta_assets::find_model(&vfs, &args[2])
        .ok_or_else(|| format!("model {:?}: not found in mounted VFS", args[2]))?;
    let model = sta_assets::load_model(&vfs, &path, format).map_err(|e| format!("loading VFS model {path:?}: {e}"))?;
    Ok(report_model(&model, &format!("{path} {format:?}"), Some(&vfs)))
}

fn report_chunk_material_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("reading chunk material library {path:?}: {e}"))?;
    let library = sta_assets::m3d::chunks::load_materials(&bytes)
        .map_err(|e| format!("parsing chunk material library {path:?}: {e}"))?;
    Ok(report_chunk_materials(&library, &format!("file {path:?}")))
}

fn report_chunk_geometry_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("reading chunk geometry {path:?}: {e}"))?;
    let geometry =
        sta_assets::m3d::chunks::load_geometry(&bytes).map_err(|e| format!("parsing chunk geometry {path:?}: {e}"))?;
    Ok(report_chunk_geometry(&geometry, &format!("file {path:?}")))
}

fn report_chunk_materials(library: &sta_assets::m3d::chunks::MaterialLibrary, source: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{source:?} chunk materials v{} records={} variants={}",
        library.version,
        library.materials.len(),
        library.variant_count
    );
    for (index, material) in library.materials.iter().enumerate() {
        let _ = writeln!(
            out,
            "  material #{index} lighting_material={:?} shader={:?} extra_shader={:?}",
            String::from_utf8_lossy(&material.lighting_material),
            String::from_utf8_lossy(&material.shader),
            String::from_utf8_lossy(&material.extra_shader)
        );
        for (slot, binding) in material.textures.iter().enumerate() {
            let _ = writeln!(
                out,
                "    texture[{slot}] reference={:?} flags=0x{:08x}",
                String::from_utf8_lossy(&binding.reference),
                binding.flags
            );
        }
    }
    out
}

fn report_chunk_geometry(geometry: &sta_assets::m3d::chunks::GeometryVariant, source: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{source:?} chunk geometry v{} bounds={:?} chunks={}",
        geometry.version,
        geometry.bounds,
        geometry.chunks.len()
    );
    for warning in geometry.attribute_warnings() {
        let _ = writeln!(out, "  warning={warning:?}");
    }
    for (index, chunk) in geometry.chunks.iter().enumerate() {
        let _ = writeln!(
            out,
            "  chunk #{index} special_flag=0x{:08x} flags=0x{:08x} trailer=0x{:04x} cull={} positions={} normals={} uvs={} groups={}",
            chunk.special_flag,
            chunk.flags,
            chunk.trailer,
            chunk.cull,
            chunk.positions.len(),
            chunk.normals.len(),
            chunk.uvs.len(),
            chunk.groups.len()
        );
        for (group_index, group) in chunk.groups.iter().enumerate() {
            let _ = writeln!(
                out,
                "    group #{group_index} material_index={} faces={}",
                group.material_index,
                group.faces.len()
            );
        }
    }
    let _ = writeln!(
        out,
        "external material library required; native destruction/variant playback not implemented"
    );
    out
}

fn report_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("reading {path:?}: {e}"))?;
    let model = if bytes.starts_with(b"Mad3D_SW") {
        sta_assets::m3d::load(&bytes).map_err(|e| format!("parsing M3D {path:?}: {e}"))?
    } else if bytes.starts_with(b"Storm3D_SW") || bytes.starts_with(b"StarTrekDB") {
        let sod = sta_assets::a2fmt::sod::Sod::parse(&bytes).map_err(|e| format!("parsing SOD {path:?}: {e:?}"))?;
        sta_assets::model::Model::from_sod(&sod)
    } else {
        return Err(format!(
            "{path:?}: unsupported model header (expected Mad3D_SW, Storm3D_SW or StarTrekDB)"
        ));
    };
    Ok(report_model(&model, &format!("file {path:?}"), None))
}

fn report_model(model: &sta_assets::model::Model, source: &str, vfs: Option<&sta_vfs::Vfs>) -> String {
    use sta_assets::model::NodeKind;

    let mut out = String::new();
    let _ = writeln!(out, "{source:?} v{}", model.version);
    let _ = writeln!(out, "  model bounds={:?}", model.bounds());
    for warning in &model.warnings {
        let _ = writeln!(out, "  warning={warning:?}");
    }
    for (i, node) in model.nodes.iter().enumerate() {
        let _ = writeln!(
            out,
            "  node #{i} name={:?} parent={:?} children={:?} transform={:?}",
            node.name, node.parent, node.children, node.transform
        );
        match &node.kind {
            NodeKind::Lod => {
                let _ = writeln!(out, "    kind=LOD control (typed)");
            }
            NodeKind::Light(light) => {
                let _ = writeln!(
                    out,
                    "    kind=light subtype={} rgb={:?} falloff_start={} falloff_range={}; parsed data; renderer light playback unavailable",
                    light.kind, light.colour, light.falloff_start, light.falloff_range
                );
            }
            NodeKind::Mesh(mesh) => {
                for group in &mesh.groups {
                    let _ = writeln!(
                        out,
                        "    group material={:?} blend={} faces={} maps={:?}",
                        group.material,
                        group.blend,
                        group.faces.len(),
                        group.maps
                    );
                }
            }
            NodeKind::Emitter(emitter) => {
                let _ = writeln!(out, "    kind=emitter {:?}", emitter);
            }
            NodeKind::Null => {
                let _ = writeln!(out, "    kind=null");
            }
            NodeKind::Sprite => {
                let _ = writeln!(out, "    kind=sprite");
            }
        }
    }

    // Report the same choice as the renderer rather than duplicate its policy.
    let hidden = model.coarse_lods();
    for (i, node) in model.nodes.iter().enumerate().filter(|(_, n)| n.is_geometry()) {
        let levels: Vec<usize> = node
            .children
            .iter()
            .copied()
            .filter(|&child| model.nodes[child].is_lod())
            .collect();
        if levels.is_empty() {
            continue;
        }
        for level in levels {
            let state = if hidden.contains(&level) {
                "hidden coarse"
            } else {
                "selected finest"
            };
            let _ = writeln!(
                out,
                "  LOD branch geometry=#{i} branch=#{level} {:?}: {state}",
                model.nodes[level].name
            );
        }
    }

    let mut seen = std::collections::BTreeSet::new();
    for node in &model.nodes {
        if let Some(mesh) = node.mesh() {
            for group in &mesh.groups {
                for texture in [&group.maps.bump, &group.maps.height].into_iter().flatten() {
                    if !seen.insert(texture.to_ascii_lowercase()) {
                        continue;
                    }
                    if let Some(vfs) = vfs {
                        match sta_assets::find_texture(vfs, texture) {
                            Some((path, format)) => match sta_assets::decode_rgba(vfs, &path, format) {
                                Ok(img) => {
                                    let mut sum = [0u64; 3];
                                    for px in img.pixels.chunks_exact(4) {
                                        for c in 0..3 {
                                            sum[c] += px[c] as u64;
                                        }
                                    }
                                    let count = (img.width * img.height) as u64;
                                    let _ = writeln!(
                                        out,
                                        "  bump {texture:?} -> {path:?} {}x{} mean rgb {:?} normal_map={}",
                                        img.width,
                                        img.height,
                                        sum.map(|s| s / count),
                                        sta_assets::pixels::looks_like_normal_map(&img)
                                    );
                                }
                                Err(e) => {
                                    let _ = writeln!(out, "  bump {texture:?} -> {path:?}: {e}");
                                }
                            },
                            None => {
                                let _ = writeln!(out, "  bump {texture:?}: NOT FOUND");
                            }
                        }
                    } else {
                        let _ = writeln!(out, "  bump {texture:?}: texture lookup unavailable in --file mode");
                    }
                }
            }
        }
    }
    let damage = [
        "shields", "engines", "target", "sensors", "life", "borg", "crew", "damage",
    ];
    for (i, node) in model.nodes.iter().enumerate() {
        if damage.iter().any(|d| node.name.eq_ignore_ascii_case(d)) {
            let _ = writeln!(
                out,
                "  damage-ish node {:?} (#{i}) children={}",
                node.name,
                node.children.len()
            );
        }
    }
    out
}

fn main() -> std::process::ExitCode {
    let args = std::env::args_os()
        .skip(1)
        .map(|arg| arg.into_string().map_err(|arg| format!("non-UTF-8 argument {arg:?}")))
        .collect::<Result<Vec<_>, _>>();
    match args.and_then(|args| run(&args)) {
        Ok(output) => {
            print!("{output}");
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::from(2)
        }
    }
}

#[cfg(test)]
#[path = "model_info/tests.rs"]
mod tests;
