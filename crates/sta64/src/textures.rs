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
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use sta_assets::dds::DdsKind;
use sta_assets::{Rgba, TextureFormat as Fmt};
use std::collections::HashMap;

/// Decoded textures by key; `None` caches a miss.
#[derive(Resource, Default)]
pub struct TextureCache {
    pub map: HashMap<String, Option<Handle<Image>>>,
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
