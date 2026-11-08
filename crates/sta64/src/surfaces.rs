//! Recipe surfaces in the game: recipes looked up by texture name, baked on a
//! background thread (`sta_surface`), cached on disk, uploaded as cube maps.
//!
//! Recipes: `surfaces/<texture>.ini` through the VFS (a mod can add or override),
//! else the ones bundled from Armada II Refit (`crates/sta_surface/recipes`, MIT).
//! The disk cache lives in the user's cache directory (`sta64/surfaces`), keyed by
//! the recipe text and the bake sizes, so each surface is baked once.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension};
use sta_surface::{Cube, Recipe, Surface};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Bake sizes: planets fill much of the screen; rocks are small.
pub const PLANET_FACE: u32 = 512;
pub const ROCK_FACE: u32 = 256;
const CLOUD_FACE: u32 = 512;
/// Bump when the bake or the cache format changes.
const CACHE_VERSION: u32 = 1;

const BUNDLED: &[(&str, &str)] = &[
    ("master1", include_str!("../../sta_surface/recipes/Master1.ini")),
    ("master2", include_str!("../../sta_surface/recipes/Master2.ini")),
    ("mdmoon", include_str!("../../sta_surface/recipes/Mdmoon.ini")),
    ("pb_clssd", include_str!("../../sta_surface/recipes/PB_CLSSD.ini")),
    ("pb_clssh", include_str!("../../sta_surface/recipes/PB_CLSSH.ini")),
    ("pb_clssj", include_str!("../../sta_surface/recipes/PB_CLSSJ.ini")),
    ("pb_clssk", include_str!("../../sta_surface/recipes/PB_CLSSK.ini")),
    ("pb_clssl", include_str!("../../sta_surface/recipes/PB_CLSSL.ini")),
    ("pb_clssm", include_str!("../../sta_surface/recipes/PB_CLSSM.ini")),
];

/// A baked surface on the GPU.
pub struct Baked {
    pub albedo: Handle<Image>,
    pub relief: Handle<Image>,
    pub clouds: Option<Handle<Image>>,
    pub recipe: Recipe,
}

enum Entry {
    NoRecipe,
    /// The bake thread fills the slot.
    Baking(Arc<Mutex<Option<(Recipe, Surface)>>>),
    Ready(Arc<Baked>),
}

#[derive(Resource, Default)]
pub struct Surfaces {
    entries: HashMap<String, Entry>,
}

pub fn recipe_text(vfs: &sta_vfs::Vfs, name: &str) -> Option<String> {
    let key = name.to_ascii_lowercase();
    if let Some(Ok(b)) = vfs.read(&format!("surfaces/{key}.ini")) {
        return Some(String::from_utf8_lossy(&b).into_owned());
    }
    BUNDLED.iter().find(|(n, _)| *n == key).map(|(_, t)| (*t).to_owned())
}

fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("sta64").join("surfaces")
}

fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100_0000_01b3))
}

fn write_cube(out: &mut Vec<u8>, c: &Cube) {
    out.extend(c.size.to_le_bytes());
    for f in &c.faces {
        out.extend_from_slice(f);
    }
}

fn read_cube(data: &[u8], at: &mut usize) -> Option<Cube> {
    let size = u32::from_le_bytes(data.get(*at..*at + 4)?.try_into().ok()?);
    *at += 4;
    let n = (size * size * 4) as usize;
    let mut faces: [Vec<u8>; 6] = Default::default();
    for f in &mut faces {
        *f = data.get(*at..*at + n)?.to_vec();
        *at += n;
    }
    Some(Cube { size, faces })
}

fn load_cached(path: &PathBuf) -> Option<Surface> {
    let data = std::fs::read(path).ok()?;
    let mut at = 8;
    if data.get(..8)? != b"STASURF1" {
        return None;
    }
    let albedo = read_cube(&data, &mut at)?;
    let relief = read_cube(&data, &mut at)?;
    let has_clouds = *data.get(at)?;
    at += 1;
    let clouds = if has_clouds == 1 { Some(read_cube(&data, &mut at)?) } else { None };
    Some(Surface { albedo, relief, clouds })
}

fn save_cached(path: &PathBuf, s: &Surface) {
    let mut out = b"STASURF1".to_vec();
    write_cube(&mut out, &s.albedo);
    write_cube(&mut out, &s.relief);
    match &s.clouds {
        Some(c) => {
            out.push(1);
            write_cube(&mut out, c);
        }
        None => out.push(0),
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, out).is_ok() {
        let _ = std::fs::rename(tmp, path);
    }
}

/// Upload a cube with a full mip chain per face.
fn upload(c: &Cube, srgb: bool) -> Image {
    let s = c.size;
    let mut levels_per_face = 0;
    let mut data = Vec::new();
    for f in &c.faces {
        let base = sta_assets::Rgba {
            width: s,
            height: s,
            has_alpha: true,
            pixels: f.clone(),
        };
        let mips = sta_assets::pixels::generate_mips(&base);
        levels_per_face = mips.len() as u32;
        for m in mips {
            data.extend_from_slice(&m.pixels);
        }
    }
    let base_len = (s * s * 4 * 6) as usize;
    let mut img = Image::new(
        Extent3d {
            width: s,
            height: s,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        vec![0; base_len],
        if srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm },
        RenderAssetUsages::RENDER_WORLD,
    );
    img.data = Some(data);
    img.texture_descriptor.mip_level_count = levels_per_face;
    img.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        address_mode_w: ImageAddressMode::ClampToEdge,
        ..ImageSamplerDescriptor::linear()
    });
    img
}

impl Surfaces {
    /// The baked surface for texture `name`, starting its bake on first request.
    /// None while baking or when there is no recipe.
    pub fn request(&mut self, vfs: &sta_vfs::Vfs, name: &str, face: u32) -> Option<Arc<Baked>> {
        let key = name.to_ascii_lowercase();
        if let Some(e) = self.entries.get(&key) {
            return match e {
                Entry::Ready(b) => Some(b.clone()),
                _ => None,
            };
        }
        let Some(text) = recipe_text(vfs, &key) else {
            self.entries.insert(key, Entry::NoRecipe);
            return None;
        };
        let recipe = Recipe::parse(&text);
        let face = if recipe.face > 0 { recipe.face.min(face) } else { face };
        let id = fnv(format!("{CACHE_VERSION}|{face}|{CLOUD_FACE}|{text}").as_bytes());
        let path = cache_dir().join(format!("{key}-{id:016x}.bin"));
        let slot = Arc::new(Mutex::new(None));
        let out = slot.clone();
        std::thread::spawn(move || {
            let s = load_cached(&path).unwrap_or_else(|| {
                let s = sta_surface::bake(&recipe, face, CLOUD_FACE);
                save_cached(&path, &s);
                s
            });
            if let Ok(mut o) = out.lock() {
                *o = Some((recipe, s));
            }
        });
        self.entries.insert(key, Entry::Baking(slot));
        None
    }

    /// Whether `name` has a recipe (known only after a request).
    pub fn has_recipe(&self, name: &str) -> bool {
        !matches!(self.entries.get(&name.to_ascii_lowercase()), Some(Entry::NoRecipe) | None)
    }
}

/// Upload finished bakes.
pub fn poll(mut surfaces: ResMut<Surfaces>, mut images: ResMut<Assets<Image>>) {
    for e in surfaces.entries.values_mut() {
        let Entry::Baking(slot) = e else { continue };
        let Some((recipe, s)) = slot.lock().ok().and_then(|mut o| o.take()) else { continue };
        let baked = Baked {
            albedo: images.add(upload(&s.albedo, true)),
            relief: images.add(upload(&s.relief, false)),
            clouds: s.clouds.as_ref().map(|c| images.add(upload(c, false))),
            recipe,
        };
        *e = Entry::Ready(Arc::new(baked));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_recipes_parse_and_cache_round_trips() {
        for (name, text) in BUNDLED {
            let r = Recipe::parse(text);
            assert!(r.k.noise[0] > 0.0, "{name}");
        }
        let r = Recipe::parse(BUNDLED[0].1);
        let s = sta_surface::bake(&r, 8, 8);
        let p = std::env::temp_dir().join(format!("sta64_surface_{}.bin", std::process::id()));
        save_cached(&p, &s);
        let back = load_cached(&p).unwrap();
        assert_eq!(back.albedo.faces, s.albedo.faces);
        assert_eq!(back.relief.faces, s.relief.faces);
        assert_eq!(back.clouds.is_some(), s.clouds.is_some());
    }

    #[test]
    fn cube_upload_has_six_layers_and_mips() {
        let r = Recipe::parse(BUNDLED[0].1);
        let s = sta_surface::bake(&r, 16, 8);
        let img = upload(&s.albedo, true);
        assert_eq!(img.texture_descriptor.size.depth_or_array_layers, 6);
        assert_eq!(img.texture_descriptor.mip_level_count, 5);
        // 6 faces x (16^2 + 8^2 + 4^2 + 2^2 + 1) texels x 4 bytes.
        assert_eq!(img.data.as_ref().unwrap().len(), 6 * (256 + 64 + 16 + 4 + 1) * 4);
    }
}
