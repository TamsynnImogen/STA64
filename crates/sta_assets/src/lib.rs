//! Engine-independent asset lookup and decoding over a mounted [`Vfs`].
//!
//! Format history this crate follows:
//! - Armada II: SOD models, TGA textures (`Textures/RGB/`).
//! - Fleet Operations: adds DDS textures, mostly in `Textures/`.
//! - A2FOExtensions: adds M3D models (1.93–2.06); Legacy chunk contexts are explicit.
//! - STA64 native: static glTF/GLB models with PNG/JPEG material images.
//!
//! Lookup by bare name searches the known folders. A file in a higher-priority
//! mod layer always wins. Within one layer the newest format wins:
//! KTX2 > DDS > TGA > PNG, and GLB > glTF > SOD > M3D. Lookup ranking does
//! not imply every texture format has a CPU decoder.

extern crate alloc;

use sta_vfs::Vfs;

pub use a2fmt;

pub mod audio;
pub mod bmp;
pub mod cfg_h;
pub mod dds;
pub mod gltf;
pub mod m3d;
pub mod model;
pub mod pixels;
pub mod silhouette;
pub mod spr_anim;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureFormat {
    Ktx2,
    Dds,
    Tga,
    Png,
}

impl TextureFormat {
    pub const PREFERENCE: [TextureFormat; 4] = [Self::Ktx2, Self::Dds, Self::Tga, Self::Png];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Ktx2 => "ktx2",
            Self::Dds => "dds",
            Self::Tga => "tga",
            Self::Png => "png",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFormat {
    Glb,
    Gltf,
    Sod,
    M3d,
}

impl ModelFormat {
    /// Within one mod layer. A SOD beside an M3D wins, as in A2FOM3D; an M3D in a
    /// child mod still overrides a parent's SOD because layers are compared first.
    pub const PREFERENCE: [ModelFormat; 4] = [Self::Glb, Self::Gltf, Self::Sod, Self::M3d];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Glb => "glb",
            Self::Gltf => "gltf",
            Self::Sod => "sod",
            Self::M3d => "m3d",
        }
    }
}

const TEXTURE_DIRS: [&str; 2] = ["textures", "textures/rgb"];
const MODEL_DIRS: [&str; 1] = ["sod"];

/// Strip any extension a reference carries; SODs and ODFs name textures by stem.
fn stem(name: &str) -> &str {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match base.rfind('.') {
        Some(i) if i > 0 => &base[..i],
        _ => base,
    }
}

/// Find the best file for `name` over `dirs` x `exts`: the lowest layer index
/// wins, then the earliest extension in `exts`.
fn find_best<F: Copy>(vfs: &Vfs, name: &str, dirs: &[&str], formats: &[(F, &str)]) -> Option<(String, F)> {
    let stem = stem(name);
    let mut best: Option<(usize, usize, String, F)> = None;
    for dir in dirs {
        for (rank, (fmt, ext)) in formats.iter().enumerate() {
            let path = format!("{dir}/{stem}.{ext}").to_ascii_lowercase();
            if let Some((layer, _)) = vfs.resolve(&path) {
                let better = match &best {
                    None => true,
                    Some((bl, br, ..)) => (layer, rank) < (*bl, *br),
                };
                if better {
                    best = Some((layer, rank, path, *fmt));
                }
            }
        }
    }
    best.map(|(_, _, p, f)| (p, f))
}

pub fn find_texture(vfs: &Vfs, name: &str) -> Option<(String, TextureFormat)> {
    let formats = TextureFormat::PREFERENCE.map(|f| (f, f.extension()));
    find_best(vfs, name, &TEXTURE_DIRS, &formats)
}

pub fn find_model(vfs: &Vfs, name: &str) -> Option<(String, ModelFormat)> {
    let formats = ModelFormat::PREFERENCE.map(|f| (f, f.extension()));
    find_best(vfs, name, &MODEL_DIRS, &formats)
}

/// The best `.sod` for `name`, ignoring any glTF/GLB/M3D replacement: the
/// simulation's geometry (collision radius, launch points) comes from the SOD
/// even when the renderer shows another format. Lowest layer wins.
pub fn find_sod(vfs: &Vfs, name: &str) -> Option<String> {
    find_best(vfs, name, &MODEL_DIRS, &[(ModelFormat::Sod, "sod")]).map(|(p, _)| p)
}

/// All model names visible in the VFS (stems, lower case, sorted, deduplicated).
pub fn list_models(vfs: &Vfs) -> Vec<String> {
    let mut out: Vec<String> = MODEL_DIRS
        .iter()
        .flat_map(|d| vfs.list(d))
        .filter(|p| ModelFormat::PREFERENCE.iter().any(|f| p.ends_with(&format!(".{}", f.extension()))))
        .map(|p| stem(&p).to_owned())
        .filter(|name| find_model(vfs, name).is_some())
        .collect();
    out.sort();
    out.dedup();
    out
}

#[derive(Debug)]
pub enum AssetError {
    NotFound(String),
    Io(String, std::io::Error),
    Sod(String, a2fmt::sod::Error),
    Tga(String, a2fmt::tga::Error),
    Dds(String, dds::DdsError),
    Bmp(String, bmp::BmpError),
    M3d(String, String),
    Gltf(String, String),
    Unsupported(String),
}

impl std::fmt::Display for AssetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(n) => write!(f, "{n}: not found"),
            Self::Io(n, e) => write!(f, "{n}: {e}"),
            Self::Sod(n, e) => write!(f, "{n}: SOD {e:?}"),
            Self::Tga(n, e) => write!(f, "{n}: TGA {e:?}"),
            Self::Dds(n, e) => write!(f, "{n}: DDS {e}"),
            Self::Bmp(n, e) => write!(f, "{n}: BMP {e}"),
            Self::M3d(n, e) => write!(f, "{n}: M3D {e}"),
            Self::Gltf(n, e) => write!(f, "{n}: glTF {e}"),
            Self::Unsupported(n) => write!(f, "{n}: no CPU decoder for this format"),
        }
    }
}

impl std::error::Error for AssetError {}

pub fn read(vfs: &Vfs, path: &str) -> Result<Vec<u8>, AssetError> {
    vfs.read(path)
        .ok_or_else(|| AssetError::NotFound(path.to_owned()))?
        .map_err(|e| AssetError::Io(path.to_owned(), e))
}

pub fn load_sod(vfs: &Vfs, path: &str) -> Result<a2fmt::sod::Sod, AssetError> {
    let data = read(vfs, path)?;
    a2fmt::sod::Sod::parse(&data).map_err(|e| AssetError::Sod(path.to_owned(), e))
}

/// Load a SOD, M3D or static glTF/GLB model into the shared [`model::Model`].
pub fn load_model(vfs: &Vfs, path: &str, format: ModelFormat) -> Result<model::Model, AssetError> {
    match format {
        ModelFormat::Sod => load_sod(vfs, path).map(|s| model::Model::from_sod(&s)),
        ModelFormat::M3d => {
            let data = read(vfs, path)?;
            // A file named .m3d may really be a SOD (A2FOM3D passes those through).
            if data.starts_with(b"Storm3D_SW") || data.starts_with(b"StarTrekDB") {
                return a2fmt::sod::Sod::parse(&data)
                    .map(|s| model::Model::from_sod(&s))
                    .map_err(|e| AssetError::Sod(path.to_owned(), e));
            }
            m3d::load(&data).map_err(|e| AssetError::M3d(path.to_owned(), e))
        }
        ModelFormat::Glb | ModelFormat::Gltf => gltf::load(vfs, path).map_err(|e| AssetError::Gltf(path.to_owned(), e)),
    }
}

/// An image decoded to RGBA8, rows top-down.
#[derive(Clone, Debug)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
    pub pixels: Vec<u8>,
}

pub fn decode_tga(path: &str, data: &[u8]) -> Result<Rgba, AssetError> {
    let img = a2fmt::tga::decode(data).map_err(|e| AssetError::Tga(path.to_owned(), e))?;
    let has_alpha = img.has_alpha_channel();
    let mut pixels = img.pixels;
    for px in pixels.chunks_exact_mut(4) {
        px.swap(0, 2); // BGRA -> RGBA
    }
    Ok(Rgba {
        width: img.width,
        height: img.height,
        has_alpha,
        pixels,
    })
}

/// The engine's sprite tables: `sprites/sprites.spr` and everything it
/// `@include`s, each resolved through the VFS (highest mod layer first).
pub fn load_sprite_table(vfs: &Vfs) -> a2fmt::spr::SpriteTable {
    a2fmt::spr::SpriteTable::load("sprites.spr", |name| {
        let name = name.replace('\\', "/");
        vfs.read(&format!("sprites/{name}")).and_then(Result::ok)
    })
}

/// Find a texture, preferring its Borg-assimilated `<name>_b` variant when
/// `borg` is set (the engine swaps to these when a ship is assimilated).
pub fn find_texture_variant(vfs: &Vfs, name: &str, borg: bool) -> Option<(String, TextureFormat)> {
    if borg && let Some(found) = find_texture(vfs, &format!("{}_b", stem(name))) {
        return Some(found);
    }
    find_texture(vfs, name)
}

/// Decode a texture file to RGBA8 on the CPU (TGA and DDS).
pub fn decode_rgba(vfs: &Vfs, path: &str, fmt: TextureFormat) -> Result<Rgba, AssetError> {
    let data = read(vfs, path)?;
    match fmt {
        TextureFormat::Tga => decode_tga(path, &data),
        TextureFormat::Dds => dds::decode(&data).map_err(|e| AssetError::Dds(path.to_owned(), e)),
        _ => Err(AssetError::Unsupported(path.to_owned())),
    }
}

/// The full mip chain of a TGA: the base, then hand-authored levels named
/// `<base>_<n>` or `<base><n>` beside it (accepted only at exactly half size per
/// level, as the engine does), then box-filtered levels down to 1x1.
pub fn tga_mip_chain(vfs: &Vfs, path: &str, base: Rgba) -> Vec<Rgba> {
    let (dir, file) = path.rsplit_once('/').unwrap_or(("", path));
    let stem = file.strip_suffix(".tga").unwrap_or(file);
    let mut levels = vec![base];
    for level in 1u32.. {
        let last = levels.last().unwrap();
        let (want_w, want_h) = ((last.width / 2).max(1), (last.height / 2).max(1));
        if last.width == 1 && last.height == 1 {
            break;
        }
        let found = [format!("{stem}_{level}"), format!("{stem}{level}")].into_iter().find_map(|n| {
            let p = if dir.is_empty() { format!("{n}.tga") } else { format!("{dir}/{n}.tga") };
            vfs.resolve(&p)?;
            let img = decode_tga(&p, &read(vfs, &p).ok()?).ok()?;
            (img.width == want_w && img.height == want_h).then_some(img)
        });
        match found {
            Some(img) => levels.push(img),
            None => break,
        }
    }
    pixels::complete_mips(levels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn stems() {
        assert_eq!(stem("fdefiant"), "fdefiant");
        assert_eq!(stem("Textures\\RGB\\fdefiant.TGA"), "fdefiant");
        assert_eq!(stem(".hidden"), ".hidden");
    }

    #[test]
    fn layer_beats_format_and_format_ranks_within_layer() {
        let root = std::env::temp_dir().join(format!("sta_assets_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let w = |p: &str| {
            let p = root.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        };
        w("Data/Textures/RGB/a.tga");
        w("Data/Textures/a.dds");
        w("Data/Textures/RGB/b.tga");
        w("Data/Mods/M/info.ini");
        w("Data/Mods/M/Textures/RGB/b.tga");
        w("Data/sod/ship.sod");
        w("Data/Mods/M/sod/ship.glb");
        // Nested library/data files are not selectable by the bare-name model
        // resolver. A nested duplicate must still leave the root model listed.
        w("Data/Mods/M/sod/chunks/library.m3d");
        w("Data/Mods/M/sod/nested/unreachable.sod");
        w("Data/Mods/M/sod/chunks/ship.m3d");
        let vfs = Vfs::mount_install(&root, Some("M")).unwrap();
        assert_eq!(find_texture(&vfs, "A").unwrap(), ("textures/a.dds".into(), TextureFormat::Dds));
        let (p, f) = find_texture(&vfs, "b.tga").unwrap();
        assert_eq!(f, TextureFormat::Tga);
        assert!(matches!(vfs.resolve(&p), Some((0, _))), "mod layer wins");
        assert_eq!(find_model(&vfs, "ship").unwrap().1, ModelFormat::Glb);
        assert_eq!(list_models(&vfs), ["ship"]);
    }

    /// With STA64_TEST_INSTALL (+ optional STA64_TEST_MOD): parse every SOD and
    /// resolve every texture it names.
    #[test]
    fn real_install_if_available() {
        let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
            return;
        };
        let m = std::env::var("STA64_TEST_MOD").ok();
        let vfs = Vfs::mount_install(Path::new(&root), m.as_deref()).unwrap();
        let (mut ok, mut failed, mut tex_ok) = (0, Vec::new(), 0);
        let mut missing = std::collections::BTreeSet::new();
        let mut by_format: std::collections::BTreeMap<String, usize> = Default::default();
        for name in list_models(&vfs) {
            let Some((path, fmt)) = find_model(&vfs, &name) else { continue };
            match load_model(&vfs, &path, fmt) {
                Ok(model) => {
                    ok += 1;
                    *by_format.entry(format!("{fmt:?}")).or_default() += 1;
                    for n in &model.nodes {
                        if let Some(mesh) = n.mesh() {
                            for g in &mesh.groups {
                                let m = &g.maps;
                                for t in [&m.diffuse, &m.height, &m.bump, &m.specular, &m.glow, &m.detail].into_iter().flatten() {
                                    if find_texture(&vfs, t).is_some() {
                                        tex_ok += 1;
                                    } else {
                                        missing.insert(t.to_ascii_lowercase());
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => failed.push(e.to_string()),
            }
        }
        eprintln!("models by format: {by_format:?}");
        eprintln!(
            "models: {ok} loaded, {} failed; texture refs: {tex_ok} resolved, {} distinct missing",
            failed.len(),
            missing.len()
        );
        for f in &failed {
            eprintln!("  fail {f}");
        }
        for t in missing.iter().take(10) {
            eprintln!("  missing {t}");
        }
        assert!(ok > 0);

        let mut dds_ok = 0;
        let mut dds_fail = Vec::new();
        for p in vfs.list("textures").into_iter().filter(|p| p.ends_with(".dds")) {
            match decode_rgba(&vfs, &p, TextureFormat::Dds) {
                Ok(img) => {
                    assert_eq!(img.pixels.len(), (img.width * img.height * 4) as usize);
                    dds_ok += 1;
                }
                Err(e) => dds_fail.push(e.to_string()),
            }
        }
        if let Some(Ok(b)) = vfs.read("RTS_CFG.h") {
            let c = cfg_h::CfgH::parse(&String::from_utf8_lossy(&b));
            eprintln!("RTS_CFG.h: {} values, SCROLL_BORDER_WIDTH = {:?}", c.len(), c.i64("SCROLL_BORDER_WIDTH"));
        }
        let t = load_sprite_table(&vfs);
        eprintln!(
            "SPR: {} sprites, {} sprite nodes, {} animations, {} emitters, {} missing includes, {} warnings",
            t.sprites.len(),
            t.sprite_nodes.len(),
            t.animations.len(),
            t.emitters.len(),
            t.missing_includes.len(),
            t.warnings.len()
        );
        eprintln!("DDS: {dds_ok} decoded, {} failed", dds_fail.len());
        for f in dds_fail.iter().take(10) {
            eprintln!("  fail {f}");
        }
    }
}
