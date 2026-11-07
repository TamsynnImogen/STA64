//! Engine-independent asset lookup and decoding over a mounted [`Vfs`].
//!
//! Format history this crate follows:
//! - Armada II: SOD models, TGA textures (`Textures/RGB/`).
//! - Fleet Operations: adds DDS textures, mostly in `Textures/`.
//! - A2FOExtensions: adds M3D models (planned here).
//! - STA64 native: glTF/GLB models, KTX2 textures (PNG fallback).
//!
//! Lookup by bare name searches the known folders. A file in a higher-priority
//! mod layer always wins. Within one layer the newest format wins:
//! KTX2 > DDS > TGA > PNG, and GLB > glTF > SOD.

use sta_vfs::Vfs;

pub use a2fmt;

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
}

impl ModelFormat {
    pub const PREFERENCE: [ModelFormat; 3] = [Self::Glb, Self::Gltf, Self::Sod];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Glb => "glb",
            Self::Gltf => "gltf",
            Self::Sod => "sod",
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

/// All model names visible in the VFS (stems, lower case, sorted, deduplicated).
pub fn list_models(vfs: &Vfs) -> Vec<String> {
    let mut out: Vec<String> = MODEL_DIRS
        .iter()
        .flat_map(|d| vfs.list(d))
        .filter(|p| ModelFormat::PREFERENCE.iter().any(|f| p.ends_with(&format!(".{}", f.extension()))))
        .map(|p| stem(&p).to_owned())
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
}

impl std::fmt::Display for AssetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(n) => write!(f, "{n}: not found"),
            Self::Io(n, e) => write!(f, "{n}: {e}"),
            Self::Sod(n, e) => write!(f, "{n}: SOD {e:?}"),
            Self::Tga(n, e) => write!(f, "{n}: TGA {e:?}"),
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

/// A TGA decoded to RGBA8, rows top-down.
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
        for name in list_models(&vfs) {
            let Some((path, ModelFormat::Sod)) = find_model(&vfs, &name) else { continue };
            match load_sod(&vfs, &path) {
                Ok(sod) => {
                    ok += 1;
                    for n in &sod.nodes {
                        if let Some(mesh) = n.mesh() {
                            for t in mesh.textures.iter().filter(|t| !t.name.is_empty()) {
                                if find_texture(&vfs, &t.name).is_some() {
                                    tex_ok += 1;
                                } else {
                                    missing.insert(t.name.to_ascii_lowercase());
                                }
                            }
                        }
                    }
                }
                Err(e) => failed.push(e.to_string()),
            }
        }
        eprintln!(
            "SOD: {ok} parsed, {} failed; texture refs: {tex_ok} resolved, {} distinct missing",
            failed.len(),
            missing.len()
        );
        for f in failed.iter().take(10) {
            eprintln!("  fail {f}");
        }
        for t in missing.iter().take(10) {
            eprintln!("  missing {t}");
        }
        assert!(ok > 0);
    }
}
