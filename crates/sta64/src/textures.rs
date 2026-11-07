//! Texture loading through the VFS: KTX2 / DDS / PNG via Bevy's decoders, TGA via
//! the vendored `a2fmt` decoder.

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageAddressMode, ImageSampler, ImageSamplerDescriptor, ImageType};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use sta_assets::TextureFormat as Fmt;
use std::collections::HashMap;

/// Decoded textures by lower-case name; `None` caches a miss.
#[derive(Resource, Default)]
pub struct TextureCache {
    pub map: HashMap<String, Option<Handle<Image>>>,
}

pub fn repeat_sampler() -> ImageSampler {
    ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    })
}

/// Look up, decode and cache a texture. Returns `None` if missing or undecodable.
pub fn load(
    name: &str,
    vfs: &sta_vfs::Vfs,
    cache: &mut TextureCache,
    images: &mut Assets<Image>,
    formats: CompressedImageFormats,
) -> Option<Handle<Image>> {
    let key = name.to_ascii_lowercase();
    if let Some(h) = cache.map.get(&key) {
        return h.clone();
    }
    let handle = decode(name, vfs, formats).map(|img| images.add(img));
    cache.map.insert(key, handle.clone());
    handle
}

fn decode(name: &str, vfs: &sta_vfs::Vfs, formats: CompressedImageFormats) -> Option<Image> {
    let (path, fmt) = sta_assets::find_texture(vfs, name)?;
    let data = match sta_assets::read(vfs, &path) {
        Ok(d) => d,
        Err(e) => {
            warn!("{e}");
            return None;
        }
    };
    let result = match fmt {
        Fmt::Tga => sta_assets::decode_tga(&path, &data).map_err(|e| e.to_string()).map(|rgba| {
            let mut img = Image::new(
                Extent3d {
                    width: rgba.width,
                    height: rgba.height,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                rgba.pixels,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            img.sampler = repeat_sampler();
            img
        }),
        other => Image::from_buffer(
            &data,
            ImageType::Extension(other.extension()),
            formats,
            true,
            repeat_sampler(),
            RenderAssetUsages::RENDER_WORLD,
        )
        .map_err(|e| e.to_string()),
    };
    match result {
        Ok(img) => Some(img),
        Err(e) => {
            warn!("{path}: {e}");
            None
        }
    }
}
