//! The Instant Action map picture. Preview art is looked up through the current
//! mount so an active mod can replace the base install's picture.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ui::widget::NodeImageMode;

use super::{ScreenData, SetupDraft};
use crate::viewer::Mounted;

/// The square in Game Setup that can show a map's authored preview image.
#[derive(Component)]
pub(super) struct AuthoredPreview;

/// Numbered start positions shown when the map has no usable preview image.
#[derive(Component)]
pub(super) struct PreviewFallback;

#[derive(Default)]
pub(super) struct PreviewCache {
    map: Option<String>,
    image: Option<Handle<Image>>,
    preview: Option<Entity>,
    mounted: bool,
}

/// Load only the selected map, once per map/mount change, and apply its image
/// to the current setup screen's preview square. Rebuilt screens reuse the
/// decoded image handle.
pub(super) fn sync_preview(
    mut commands: Commands,
    data: Res<ScreenData>,
    draft: Option<Res<SetupDraft>>,
    mounted: Option<Res<Mounted>>,
    images: Option<ResMut<Assets<Image>>>,
    mut cache: Local<PreviewCache>,
    previews: Query<Entity, With<AuthoredPreview>>,
    mut fallbacks: Query<&mut Visibility, With<PreviewFallback>>,
) {
    let mounted_changed = mounted.as_ref().is_some_and(|m| m.is_changed());
    if mounted_changed {
        cache.map = None;
        cache.image = None;
    }
    let (Some(preview), Some(mut images)) = (previews.iter().next(), images) else {
        cache.preview = None;
        return;
    };
    let map = draft
        .as_ref()
        .and_then(|d| data.maps.get(d.map))
        .map(|m| m.def.name.as_str());
    let mounted_now = mounted.is_some();
    let reload = cache.map.as_deref() != map || cache.mounted != mounted_now || mounted_changed;
    if reload {
        cache.map = map.map(str::to_owned);
        cache.mounted = mounted_now;
        cache.image = match (mounted.as_ref(), map) {
            (Some(mounted), Some(map)) => {
                load_image(&mounted.0, map).map(|image| images.add(image))
            }
            _ => None,
        };
    }
    if !reload && cache.preview == Some(preview) {
        return;
    }
    cache.preview = Some(preview);
    if let Some(image) = cache.image.as_ref() {
        commands.entity(preview).insert(ImageNode {
            image: image.clone(),
            image_mode: NodeImageMode::Stretch,
            ..default()
        });
    } else {
        commands.entity(preview).remove::<ImageNode>();
    }
    let visibility = if cache.image.is_some() {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    for mut fallback in &mut fallbacks {
        *fallback = visibility;
    }
}

fn load_image(vfs: &sta_vfs::Vfs, stem: &str) -> Option<Image> {
    // A corrupt PNG does not prevent a valid BMP of the same map from showing.
    for ext in ["png", "bmp"] {
        let path = format!("bzn/{stem}.{ext}");
        let Some(Ok(bytes)) = vfs.read(&path) else {
            continue;
        };
        let Some((width, height, pixels)) = decode(&path, &bytes, ext) else {
            continue;
        };
        return Some(Image::new(
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            pixels,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        ));
    }
    None
}

fn decode(path: &str, bytes: &[u8], ext: &str) -> Option<(u32, u32, Vec<u8>)> {
    match ext {
        "png" => {
            let rgba = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
                .ok()?
                .to_rgba8();
            let (width, height) = rgba.dimensions();
            (width > 0 && height > 0).then(|| (width, height, rgba.into_raw()))
        }
        "bmp" => {
            let rgba = sta_assets::bmp::decode_bmp(path, bytes).ok()?;
            Some((rgba.width, rgba.height, rgba.pixels))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_decodes_to_rgba_and_bad_bytes_fall_back() {
        let source = image::RgbaImage::from_pixel(2, 1, image::Rgba([12, 34, 56, 78]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(source)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert_eq!(
            decode("bzn/test.png", &png, "png"),
            Some((2, 1, vec![12, 34, 56, 78, 12, 34, 56, 78]))
        );
        assert!(decode("bzn/test.png", b"broken", "png").is_none());
    }

    #[test]
    fn bmp_decodes_through_the_stock_bitmap_decoder() {
        // One 24-bit pixel, four-byte aligned row, with the Windows BMP headers.
        let mut bmp = b"BM".to_vec();
        bmp.extend_from_slice(&58u32.to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&1i32.to_le_bytes());
        bmp.extend_from_slice(&1i32.to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&24u16.to_le_bytes());
        bmp.extend_from_slice(&[0; 24]);
        bmp.extend_from_slice(&[3, 2, 1, 0]);
        assert_eq!(
            decode("bzn/test.bmp", &bmp, "bmp"),
            Some((1, 1, vec![1, 2, 3, 255]))
        );
    }
}
