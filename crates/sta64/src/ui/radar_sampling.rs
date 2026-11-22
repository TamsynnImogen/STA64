//! RGB sampling of a cropped minimap sprite before it is added to the map picture.

use bevy::prelude::Vec2;

/// Sample tightly packed RGBA8 with bilinear filtering. UVs address the full image,
/// with texel centres at `(i + 0.5) / size`; outside UVs clamp to the edge texel.
/// Alpha is ignored because the minimap picture adds source RGB directly.
pub(super) fn sample_rgb(rgba: &[u8], width: usize, height: usize, uv: Vec2) -> [u8; 3] {
    assert!(width > 0 && height > 0);
    assert_eq!(rgba.len(), width * height * 4);

    let x = (uv.x * width as f32 - 0.5).clamp(0.0, (width - 1) as f32);
    let y = (uv.y * height as f32 - 0.5).clamp(0.0, (height - 1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
    let (tx, ty) = (x - x0 as f32, y - y0 as f32);
    let at = |px: usize, py: usize, channel: usize| rgba[(py * width + px) * 4 + channel] as f32;

    [0, 1, 2].map(|channel| {
        let top = at(x0, y0, channel) * (1.0 - tx) + at(x1, y0, channel) * tx;
        let bottom = at(x0, y1, channel) * (1.0 - tx) + at(x1, y1, channel) * tx;
        (top * (1.0 - ty) + bottom * ty).round() as u8
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIXELS: [u8; 16] = [0, 0, 0, 0, 255, 0, 0, 0, 0, 255, 0, 0, 0, 0, 255, 0];

    #[test]
    fn midpoint_mixes_four_texels_without_alpha() {
        assert_eq!(sample_rgb(&PIXELS, 2, 2, Vec2::splat(0.5)), [64, 64, 64]);
    }

    #[test]
    fn outside_uvs_clamp_to_edge_texels() {
        assert_eq!(sample_rgb(&PIXELS, 2, 2, Vec2::new(-3.0, -2.0)), [0, 0, 0]);
        assert_eq!(sample_rgb(&PIXELS, 2, 2, Vec2::new(4.0, -2.0)), [255, 0, 0]);
        assert_eq!(sample_rgb(&PIXELS, 2, 2, Vec2::new(-3.0, 5.0)), [0, 255, 0]);
        assert_eq!(sample_rgb(&PIXELS, 2, 2, Vec2::new(4.0, 5.0)), [0, 0, 255]);
    }
}
