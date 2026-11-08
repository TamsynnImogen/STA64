//! Pixel helpers: mip chains and height-to-normal conversion.

use crate::Rgba;

/// Box-filter `base` down to 1x1. Level 0 is `base` itself.
pub fn generate_mips(base: &Rgba) -> Vec<Rgba> {
    let mut levels = vec![base.clone()];
    while let Some(last) = levels.last() {
        if last.width == 1 && last.height == 1 {
            break;
        }
        levels.push(half(last));
    }
    levels
}

/// One box-filtered level below `src`.
pub fn half(src: &Rgba) -> Rgba {
    let (sw, sh) = (src.width as usize, src.height as usize);
    let (w, h) = ((sw / 2).max(1), (sh / 2).max(1));
    let mut px = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            for c in 0..4 {
                let mut s = 0u32;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (x * 2 + dx).min(sw - 1);
                    let sy = (y * 2 + dy).min(sh - 1);
                    s += src.pixels[(sy * sw + sx) * 4 + c] as u32;
                }
                px.push(((s + 2) / 4) as u8);
            }
        }
    }
    Rgba {
        width: w as u32,
        height: h as u32,
        has_alpha: src.has_alpha,
        pixels: px,
    }
}

/// Complete a partial chain (e.g. hand-authored levels that stop early) by
/// box-filtering from the last level down to 1x1.
pub fn complete_mips(mut levels: Vec<Rgba>) -> Vec<Rgba> {
    while let Some(last) = levels.last() {
        if last.width == 1 && last.height == 1 {
            break;
        }
        let next = half(last);
        levels.push(next);
    }
    levels
}

/// The engine's height-map-to-normal-map conversion (`ST3D_CreateNormalMap`):
/// height in 0..1 from the red channel, central differences with wrap-around.
///
/// The output is a tangent-space normal map in the glTF/Bevy convention (green
/// points toward -V, "up" in the image). Direct3D's original stored green the
/// other way, so the V component is negated relative to it.
pub fn height_to_normal(height: &Rgba) -> Rgba {
    let (w, h) = (height.width as i64, height.height as i64);
    let hgt = |x: i64, y: i64| -> f32 {
        let x = x.rem_euclid(w);
        let y = y.rem_euclid(h);
        height.pixels[((y * w + x) * 4) as usize] as f32 / 255.0
    };
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let du = hgt(x + 1, y) - hgt(x - 1, y);
            let dv = hgt(x, y + 1) - hgt(x, y - 1);
            let l = (du * du + dv * dv + 1.0).sqrt();
            // Green holds the slope along +V when no flip is needed (Direct3D
            // convention), along -V otherwise.
            let gv = if D3D_GREEN_FLIP { dv } else { -dv };
            let n = [-du / l, gv / l, 1.0 / l];
            let b = |c: f32| ((c * 0.5 + 0.5) * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
            px.extend_from_slice(&[b(n[0]), b(n[1]), b(n[2]), 255]);
        }
    }
    Rgba {
        width: height.width,
        height: height.height,
        has_alpha: false,
        pixels: px,
    }
}

/// Whether a bump texture is an RGB tangent-space normal map (bluish, red and
/// green centred on 0.5) rather than a greyscale height map.
pub fn looks_like_normal_map(img: &Rgba) -> bool {
    let n = (img.width * img.height) as usize;
    if n == 0 {
        return false;
    }
    let step = (n / 4096).max(1);
    let (mut grey, mut count) = (0usize, 0usize);
    let (mut r, mut g, mut b) = (0f64, 0f64, 0f64);
    for px in img.pixels.chunks_exact(4).step_by(step) {
        let (pr, pg, pb) = (px[0] as i32, px[1] as i32, px[2] as i32);
        if (pr - pg).abs() < 8 && (pg - pb).abs() < 8 {
            grey += 1;
        }
        r += pr as f64;
        g += pg as f64;
        b += pb as f64;
        count += 1;
    }
    let c = count as f64 * 255.0;
    let (r, g, b) = (r / c, g / c, b / c);
    grey * 2 < count && b > 0.6 && (r - 0.5).abs() < 0.2 && (g - 0.5).abs() < 0.2
}

/// Convert a Direct3D-authored normal map (green pointing down the image, +V)
/// to the renderer's convention (see [`D3D_GREEN_FLIP`]).
pub fn convert_direct3d_normal_map(img: &mut Rgba) {
    if D3D_GREEN_FLIP {
        for px in img.pixels.chunks_exact_mut(4) {
            px[1] = 255 - px[1];
        }
    }
}

/// Whether Direct3D normal maps need their green channel flipped for Bevy, and
/// (the same fact) whether generated normal maps store -V rather than +V in
/// green. True: Bevy's mikktspace bitangent points toward -V. Verified by
/// `sta64`'s `height_maps_tilt_away_from_the_slope` and
/// `direct3d_normal_maps_keep_their_meaning` tests against Bevy's own tangents
/// (both fail if this is set the other way).
pub const D3D_GREEN_FLIP: bool = true;

/// A metallic-roughness map (glTF layout: G roughness, B metalness) from a
/// specular map: bright specular means smooth, metalness stays 0.
pub fn roughness_from_specular(spec: &Rgba) -> Rgba {
    let mut out = spec.clone();
    out.has_alpha = false;
    for px in out.pixels.chunks_exact_mut(4) {
        let l = (px[0] as u32 * 54 + px[1] as u32 * 183 + px[2] as u32 * 19) >> 8;
        px[0] = 0;
        px[1] = 255 - l.min(255) as u8;
        px[2] = 0;
        px[3] = 255;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, v: u8) -> Rgba {
        Rgba {
            width: w,
            height: h,
            has_alpha: false,
            pixels: vec![v; (w * h * 4) as usize],
        }
    }

    #[test]
    fn mips_go_to_one_by_one() {
        let m = generate_mips(&solid(8, 2, 100));
        let dims: Vec<_> = m.iter().map(|l| (l.width, l.height)).collect();
        assert_eq!(dims, [(8, 2), (4, 1), (2, 1), (1, 1)]);
        assert!(m.iter().all(|l| l.pixels.iter().all(|&p| p == 100)));
    }

    #[test]
    fn classifies_bump_textures() {
        let grey = solid(16, 16, 90);
        assert!(!looks_like_normal_map(&grey));
        let mut normal = solid(16, 16, 0);
        for px in normal.pixels.chunks_exact_mut(4) {
            px.copy_from_slice(&[128, 120, 250, 255]);
        }
        assert!(looks_like_normal_map(&normal));
    }

    #[test]
    fn flat_height_is_straight_up() {
        let n = height_to_normal(&solid(4, 4, 77));
        assert_eq!(&n.pixels[..4], &[128, 128, 255, 255]);
    }

    #[test]
    fn slope_tilts_against_gradient() {
        // Height rising along +x: the normal leans toward -x (red below 128).
        let mut img = solid(4, 4, 0);
        for y in 0..4 {
            for x in 0..4 {
                img.pixels[((y * 4 + x) * 4) as usize] = (x * 60) as u8;
            }
        }
        let n = height_to_normal(&img);
        let px = &n.pixels[((4 + 1) * 4) as usize..][..3]; // texel (1, 1)
        assert!(px[0] < 128 && px[1] == 128 && px[2] > 200, "{px:?}");
    }
}
