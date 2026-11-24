//! Texture loading through the VFS.
//!
//! - TGA: decoded on the CPU, with hand-authored `_1.._n` mip levels where present
//!   and box-filtered levels below them.
//! - DDS: block-compressed files go to the GPU as they are (with their own mips)
//!   when it supports BC; uncompressed files, and every DDS on GPUs without BC,
//!   are decoded on the CPU.
//! - KTX2 / PNG: Bevy's decoders.
//! - Height maps (SOD 1.93 bump path) become tangent-space normal maps.

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageAddressMode, ImageSampler, ImageSamplerDescriptor, ImageType};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension};
use sta_assets::dds::DdsKind;
use sta_assets::{Rgba, TextureFormat as Fmt};
use std::collections::HashMap;

mod pbr;
pub(crate) use pbr::TextureUse;

/// Decoded textures by key; `None` caches a miss.
#[derive(Resource, Default)]
pub struct TextureCache {
    pub map: HashMap<String, Option<Handle<Image>>>,
    /// The mount's specular and emissive map declarations and which `ART_CFG.h` copies they came from (RND-38).
    pub map_config: Option<(String, crate::render::mapped::MapSuffixes)>,
    /// The faction suffixes the mount's races declare, with the layer names they were read for.
    pub race_suffixes: Option<(String, Vec<String>)>,
}

pub fn repeat_sampler() -> ImageSampler {
    ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        anisotropy_clamp: 8,
        ..ImageSamplerDescriptor::linear()
    })
}

pub struct Loader<'a> {
    pub vfs: &'a sta_vfs::Vfs,
    pub cache: &'a mut TextureCache,
    pub images: &'a mut Assets<Image>,
    pub formats: CompressedImageFormats,
}

impl Loader<'_> {
    /// A strictly DDS/TGA faction variant; a miss never aliases the base asset.
    pub fn faction_colour(&mut self, name: &str, suffix: &str) -> Option<Handle<Image>> {
        let key = format!("f:{name}:{suffix}").to_ascii_lowercase();
        if let Some(h) = self.cache.map.get(&key) { return h.clone(); }
        let handle = crate::render::appearance::factions::find_faction_texture(self.vfs, name, suffix)
            .and_then(|(path, fmt)| self.decode_colour(&path, fmt).ok())
            .map(|image| self.images.add(image));
        self.cache.map.insert(key, handle.clone());
        handle
    }

    pub fn faction_roughness(&mut self, name: &str, suffix: &str) -> Option<Handle<Image>> {
        let key = format!("fs:{name}:{suffix}").to_ascii_lowercase();
        if let Some(h) = self.cache.map.get(&key) { return h.clone(); }
        let handle = crate::render::appearance::factions::find_faction_texture(self.vfs, name, suffix)
            .and_then(|(path, fmt)| sta_assets::decode_rgba(self.vfs, &path, fmt).ok())
            .map(|pixels| self.images.add(upload(&sta_assets::pixels::generate_mips(&sta_assets::pixels::roughness_from_specular(&pixels)), false)));
        self.cache.map.insert(key, handle.clone());
        handle
    }

    /// CPU decode permits a colour key on packed as well as loose logo art.
    pub fn decal_colour(&mut self, name: &str, colour_key: Option<[u8; 3]>) -> Option<Handle<Image>> {
        let key = format!("decal:{name}:{colour_key:?}").to_ascii_lowercase();
        if let Some(h) = self.cache.map.get(&key) { return h.clone(); }
        let pixels = decal_pixels(self.vfs, name).map(|mut p| {
            if let Some(key) = colour_key {
                for px in p.pixels.chunks_exact_mut(4) { if px[..3] == key { px[3] = 0; } }
            }
            p
        });
        let handle = pixels.map(|p| self.images.add(upload(&sta_assets::pixels::generate_mips(&p), true)));
        self.cache.map.insert(key, handle.clone());
        handle
    }

    /// A colour texture, optionally its Borg `_b` variant.
    pub fn colour(&mut self, name: &str, borg: bool) -> Option<Handle<Image>> {
        let key = format!("c:{}:{}", borg as u8, name.to_ascii_lowercase());
        if let Some(h) = self.cache.map.get(&key) {
            return h.clone();
        }
        let img = sta_assets::find_texture_variant(self.vfs, name, borg).and_then(|(path, fmt)| {
            self.decode_colour(&path, fmt)
                .map_err(|e| warn!("{path}: {e}"))
                .ok()
        });
        let handle = img.map(|i| self.images.add(i));
        self.cache.map.insert(key, handle.clone());
        handle
    }

    /// A normal map built from the height map `name`.
    pub fn normal_from_height(&mut self, name: &str) -> Option<Handle<Image>> {
        let key = format!("n:{}", name.to_ascii_lowercase());
        if let Some(h) = self.cache.map.get(&key) {
            return h.clone();
        }
        let img = sta_assets::find_texture(self.vfs, name).and_then(|(path, fmt)| {
            sta_assets::decode_rgba(self.vfs, &path, fmt)
                .map_err(|e| warn!("{e}"))
                .ok()
                .map(|h| {
                    let normal = sta_assets::pixels::height_to_normal(&h);
                    upload(&sta_assets::pixels::generate_mips(&normal), false)
                })
        });
        let handle = img.map(|i| self.images.add(i));
        self.cache.map.insert(key, handle.clone());
        handle
    }

    /// An M3D bump texture as a normal map: used directly when it already is one
    /// (green flipped from Direct3D's convention), else treated as a height map.
    pub fn normal_from_bump(&mut self, name: &str) -> Option<Handle<Image>> {
        let key = format!("b:{}", name.to_ascii_lowercase());
        if let Some(h) = self.cache.map.get(&key) {
            return h.clone();
        }
        let img = self.decode_rgba(name).map(|mut px| {
            let normal = if sta_assets::pixels::looks_like_normal_map(&px) {
                sta_assets::pixels::convert_direct3d_normal_map(&mut px);
                px
            } else {
                sta_assets::pixels::height_to_normal(&px)
            };
            upload(&sta_assets::pixels::generate_mips(&normal), false)
        });
        let handle = img.map(|i| self.images.add(i));
        self.cache.map.insert(key, handle.clone());
        handle
    }

    /// A metallic-roughness texture derived from a specular map.
    pub fn roughness_from_specular(&mut self, name: &str) -> Option<Handle<Image>> {
        let key = format!("s:{}", name.to_ascii_lowercase());
        if let Some(h) = self.cache.map.get(&key) {
            return h.clone();
        }
        let img = self
            .decode_rgba(name)
            .map(|px| upload(&sta_assets::pixels::generate_mips(&sta_assets::pixels::roughness_from_specular(&px)), false));
        let handle = img.map(|i| self.images.add(i));
        self.cache.map.insert(key, handle.clone());
        handle
    }

    fn decode_rgba(&self, name: &str) -> Option<Rgba> {
        let (path, fmt) = sta_assets::find_texture(self.vfs, name)?;
        sta_assets::decode_rgba(self.vfs, &path, fmt).map_err(|e| warn!("{e}")).ok()
    }

    fn decode_colour(&self, path: &str, fmt: Fmt) -> Result<Image, String> {
        let data = sta_assets::read(self.vfs, path).map_err(|e| e.to_string())?;
        let gpu = |ext: &str| {
            Image::from_buffer(&data, ImageType::Extension(ext), self.formats, true, repeat_sampler(), RenderAssetUsages::RENDER_WORLD)
                .map_err(|e| e.to_string())
        };
        match fmt {
            Fmt::Tga => {
                let base = sta_assets::decode_tga(path, &data).map_err(|e| e.to_string())?;
                Ok(upload(&sta_assets::tga_mip_chain(self.vfs, path, base), true))
            }
            Fmt::Dds => {
                let kind = sta_assets::dds::kind(&data).map_err(|e| e.to_string())?;
                let bc = matches!(kind, DdsKind::Bc1 | DdsKind::Bc2 | DdsKind::Bc3);
                if bc && self.formats.contains(CompressedImageFormats::BC) {
                    gpu("dds")
                } else {
                    let base = sta_assets::dds::decode(&data).map_err(|e| e.to_string())?;
                    Ok(upload(&sta_assets::pixels::generate_mips(&base), true))
                }
            }
            Fmt::Ktx2 => gpu("ktx2"),
            Fmt::Png => gpu("png"),
        }
    }
}

fn decal_pixels(vfs: &sta_vfs::Vfs, name: &str) -> Option<Rgba> {
    if let Some((path, fmt)) = sta_assets::find_texture(vfs, name) {
        if let Ok(px) = sta_assets::decode_rgba(vfs, &path, fmt) { return Some(px); }
        if let Some(Ok(data)) = vfs.read(&path)
            && let Ok(img) = image::load_from_memory(&data) {
            let rgba = img.to_rgba8();
            return Some(Rgba { width: rgba.width(), height: rgba.height(), pixels: rgba.into_raw(), has_alpha: true });
        }
    }
    // BMP is intentionally not in the hull texture resolver, but is a legacy
    // registry-art format supported by the common asset decoder.
    let name = name.replace('\\', "/");
    let bmp = if name.to_ascii_lowercase().ends_with(".bmp") { name } else { format!("{name}.bmp") };
    [format!("textures/{bmp}"), format!("textures/rgb/{bmp}")].iter()
        .find_map(|path| sta_assets::bmp::load(vfs, path).ok())
}

/// Upload a full mip chain (level 0 first) as RGBA8.
pub fn upload(levels: &[Rgba], srgb: bool) -> Image {
    let base = &levels[0];
    let format = if srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm };
    let mut img = Image::new(
        Extent3d {
            width: base.width,
            height: base.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        base.pixels.clone(),
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    if levels.len() > 1 {
        let mut all = Vec::with_capacity(base.pixels.len() * 4 / 3 + 4);
        for l in levels {
            all.extend_from_slice(&l.pixels);
        }
        img.data = Some(all);
        img.texture_descriptor.mip_level_count = levels.len() as u32;
    }
    img.sampler = repeat_sampler();
    img
}

/// Six independent subsystem planes, in `mapped::EMISSIVE_WORDS` order.
pub(super) fn upload_emissive(channels: &[Option<Rgba>; 6]) -> Option<Image> {
    let first = channels.iter().flatten().next()?;
    if first.width == 0 || first.height == 0 { return None; }
    let (width, height) = (first.width, first.height);
    let mut data = Vec::new();
    let mut levels = 0;
    for channel in channels {
        let pixels = channel.as_ref().map(|source| {
            let mut out = vec![0; (width * height * 4) as usize];
            if source.width > 0 && source.height > 0 && source.pixels.len() >= (source.width * source.height * 4) as usize {
                for y in 0..height {
                    for x in 0..width {
                        let sx = x as u64 * source.width as u64 / width as u64;
                        let sy = y as u64 * source.height as u64 / height as u64;
                        let src = ((sy * source.width as u64 + sx) * 4) as usize;
                        let dst = ((y * width + x) * 4) as usize;
                        out[dst..dst + 3].copy_from_slice(&source.pixels[src..src + 3]);
                    }
                }
            }
            out
        }).unwrap_or_else(|| vec![0; (width * height * 4) as usize]);
        let base = Rgba { width, height, pixels, has_alpha: false };
        let mips = sta_assets::pixels::generate_mips(&base);
        levels = mips.len() as u32;
        for mip in mips { data.extend_from_slice(&mip.pixels); }
    }
    let mut img = Image::new(
        Extent3d { width, height, depth_or_array_layers: 6 },
        TextureDimension::D2,
        vec![0; (width * height * 4 * 6) as usize],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.data = Some(data);
    img.texture_descriptor.mip_level_count = levels;
    img.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..default() });
    img.sampler = repeat_sampler();
    Some(img)
}

/// System parameter bundle for loading textures from any system.
#[derive(bevy::ecs::system::SystemParam)]
pub struct LoaderParams<'w> {
    vfs: Res<'w, crate::viewer::Mounted>,
    cache: ResMut<'w, TextureCache>,
    images: ResMut<'w, Assets<Image>>,
    device: Option<Res<'w, bevy::render::renderer::RenderDevice>>,
}

impl LoaderParams<'_> {
    pub fn loader(&mut self) -> Loader<'_> {
        let formats = self
            .device
            .as_ref()
            .map(|d| CompressedImageFormats::from_features(d.features()))
            .unwrap_or(CompressedImageFormats::BC);
        Loader {
            vfs: &self.vfs.0,
            cache: &mut self.cache,
            images: &mut self.images,
            formats,
        }
    }
}

#[cfg(test)]
mod emissive_tests {
    use super::*;

    #[test]
    fn six_layers_have_independent_mips_and_black_absent_planes() {
        let channels = [
            Some(Rgba { width: 2, height: 2, pixels: vec![100, 0, 0, 255].repeat(4), has_alpha: true }),
            Some(Rgba { width: 1, height: 1, pixels: vec![0, 80, 0, 255], has_alpha: true }),
            None, None, None, None,
        ];
        let image = upload_emissive(&channels).unwrap();
        assert_eq!(image.texture_descriptor.size.depth_or_array_layers, 6);
        assert_eq!(image.texture_descriptor.mip_level_count, 2);
        assert_eq!(image.texture_view_descriptor.as_ref().unwrap().dimension, Some(TextureViewDimension::D2Array));
        let data = image.data.as_ref().unwrap();
        assert_eq!(data.len(), 6 * (2 * 2 * 4 + 4));
        assert_eq!(&data[0..4], &[100, 0, 0, 0]);
        assert_eq!(&data[16..20], &[100, 0, 0, 0]);
        assert_eq!(&data[20..24], &[0, 80, 0, 0]);
        assert_eq!(&data[40..44], &[0, 0, 0, 0]);
    }
}
