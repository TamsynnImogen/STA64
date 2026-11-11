//! VFS-imported native images. Author bytes stay unchanged; GPU uploads use
//! RGBA8 mip chains, with material normal/occlusion factors applied in memory.
use super::*;
use bevy::image::ImageFilterMode;
use sta_assets::model::pbr::{PbrFilter, PbrImageFormat, PbrMinFilter, PbrModel, PbrSampler, PbrTexture, PbrWrap};

#[derive(Clone, Copy, Debug)]
pub(crate) enum TextureUse {
    Color,
    Data,
    Normal(f32),
    Occlusion(f32),
}

impl Loader<'_> {
    pub(crate) fn pbr(&mut self, model: &PbrModel, texture: PbrTexture, usage: TextureUse) -> Option<Handle<Image>> {
        let image = model.images.get(texture.image)?;
        let key = format!("pbr:{:02x?}:{:?}:{usage:?}", image.content_hash, texture.sampler);
        if let Some(handle) = self.cache.map.get(&key) {
            return handle.clone();
        }
        let format = match image.format {
            PbrImageFormat::Png => image::ImageFormat::Png,
            PbrImageFormat::Jpeg => image::ImageFormat::Jpeg,
        };
        let srgb = matches!(usage, TextureUse::Color);
        // Decode author bytes directly with the same image crate used by Bevy.
        // Image::try_into_dynamic does not admit Rgba8Unorm. Passing normal/data
        // maps through that helper would drop their texture despite valid bytes.
        let decoded = image::load_from_memory_with_format(&image.data, format);
        let uploaded = decoded.map_err(|e| e.to_string()).map(|decoded| {
            let rgba = decoded.to_rgba8();
            let mut pixels = Rgba {
                width: rgba.width(),
                height: rgba.height(),
                has_alpha: true,
                pixels: rgba.into_raw(),
            };
            adjust(&mut pixels.pixels, usage);
            let mut uploaded = upload(&sta_assets::pixels::generate_mips(&pixels), srgb);
            uploaded.sampler = ImageSampler::Descriptor(sampler(texture.sampler));
            uploaded
        });
        let handle = match uploaded {
            Ok(image) => Some(self.images.add(image)),
            Err(error) => {
                warn!("native texture {:?}: {error}", image.origin);
                None
            }
        };
        self.cache.map.insert(key, handle.clone());
        handle
    }
}

fn adjust(pixels: &mut [u8], usage: TextureUse) {
    let byte = |value: f64| ((value * 255.0).round().clamp(0.0, 255.0)) as u8;
    for pixel in pixels.chunks_exact_mut(4) {
        match usage {
            TextureUse::Normal(scale) if scale != 1.0 => {
                let x = (f64::from(pixel[0]) / 127.5 - 1.0) * f64::from(scale);
                let y = (f64::from(pixel[1]) / 127.5 - 1.0) * f64::from(scale);
                let z = f64::from(pixel[2]) / 127.5 - 1.0;
                let length = (x * x + y * y + z * z).sqrt();
                let n = if length > 0.0 {
                    [x / length, y / length, z / length]
                } else {
                    [0.0, 0.0, 1.0]
                };
                for i in 0..3 {
                    pixel[i] = byte(n[i] * 0.5 + 0.5);
                }
            }
            TextureUse::Occlusion(strength) if strength != 1.0 => {
                pixel[0] = byte(1.0 + f64::from(strength) * (f64::from(pixel[0]) / 255.0 - 1.0));
            }
            _ => {}
        }
    }
}

fn sampler(source: PbrSampler) -> ImageSamplerDescriptor {
    use ImageFilterMode::{Linear, Nearest};
    let wrap = |value| match value {
        PbrWrap::Repeat => ImageAddressMode::Repeat,
        PbrWrap::Mirror => ImageAddressMode::MirrorRepeat,
        PbrWrap::Clamp => ImageAddressMode::ClampToEdge,
    };
    let (min, mip, mipmapped) = match source.min {
        Some(PbrMinFilter::Nearest) => (Nearest, Nearest, false),
        Some(PbrMinFilter::Linear) => (Linear, Nearest, false),
        Some(PbrMinFilter::NearestMipmapNearest) => (Nearest, Nearest, true),
        Some(PbrMinFilter::LinearMipmapNearest) => (Linear, Nearest, true),
        Some(PbrMinFilter::NearestMipmapLinear) => (Nearest, Linear, true),
        Some(PbrMinFilter::LinearMipmapLinear) | None => (Linear, Linear, true),
    };
    ImageSamplerDescriptor {
        address_mode_u: wrap(source.wrap_s),
        address_mode_v: wrap(source.wrap_t),
        mag_filter: match source.mag {
            Some(PbrFilter::Nearest) => Nearest,
            _ => Linear,
        },
        min_filter: min,
        mipmap_filter: mip,
        lod_max_clamp: if mipmapped { 32.0 } else { 0.0 },
        ..default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn material_image_factors_preserve_alpha_and_neutral_defaults() {
        let source = [220, 128, 220, 17, 128, 220, 220, 23];
        let mut neutral = source;
        adjust(&mut neutral, TextureUse::Normal(1.0));
        assert_eq!(neutral, source);
        let mut flat = source;
        adjust(&mut flat, TextureUse::Normal(0.0));
        assert_eq!(&flat[..3], &[128, 128, 255]);
        assert_eq!(flat[3], 17);
        let mut ao = [0, 40, 60, 77];
        adjust(&mut ao, TextureUse::Occlusion(0.25));
        assert_eq!(ao, [191, 40, 60, 77]);
    }
    #[test]
    fn sampler_keeps_wrap_nearest_and_no_mip_requests() {
        let source = PbrSampler {
            wrap_s: PbrWrap::Mirror,
            wrap_t: PbrWrap::Clamp,
            mag: Some(PbrFilter::Nearest),
            min: Some(PbrMinFilter::Nearest),
        };
        let out = sampler(source);
        assert_eq!(out.address_mode_u, ImageAddressMode::MirrorRepeat);
        assert_eq!(out.address_mode_v, ImageAddressMode::ClampToEdge);
        assert_eq!(out.mag_filter, ImageFilterMode::Nearest);
        assert_eq!(out.min_filter, ImageFilterMode::Nearest);
        assert_eq!(out.lod_max_clamp, 0.0);
        assert_eq!(out.anisotropy_clamp, 1);
    }
}
