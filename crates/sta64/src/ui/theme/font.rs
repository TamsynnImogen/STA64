//! The baked SDF font (`assets/ui/font-{regular,bold}.bin`, made by
//! `sta_fontbake`, Noto Sans Condensed under the OFL; see `assets/ui/NOTICE.md`).
//! Ported from Refit's `ui/src/text` (MIT).
//!
//! Blob layout, little-endian:
//! ```text
//!   0  "A2SD"   4 atlas_w u32   8 atlas_h u32   12 em_texels f32   16 spread f32
//!   20 ascent f32   24 descent f32   28 line_height f32   32 nglyphs u32   36 nkern u32
//!   40 glyphs, 40 bytes each, sorted by codepoint:
//!        cp u32, advance f32, bounds[4] f32 (ems from the pen, y down), uv[4] f32
//!   .. kerning pairs, 12 bytes each, sorted by (a, b): a u32, b u32, em f32
//!   .. the atlas, atlas_w * atlas_h bytes, top row first; 128 on the outline
//! ```
//!
//! This is the loader and the layout. Drawing the quads needs a UI material
//! with an SDF fragment shader (smoothstep around 0.5 of the atlas value); until
//! that lands the widgets keep Bevy's text (see `STYLE.md`, "Fonts").

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

static REGULAR: &[u8] = include_bytes!("../../../assets/ui/font-regular.bin");
static BOLD: &[u8] = include_bytes!("../../../assets/ui/font-bold.bin");

const GLYPH_BYTES: usize = 40;
const KERN_BYTES: usize = 12;
const HEADER: usize = 40;

/// One glyph, in ems from the pen on the baseline (y down), and its atlas cell.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Glyph {
    pub advance: f32,
    /// x0, y0, x1, y1 of the atlas cell (padding included).
    pub bounds: [f32; 4],
    /// u0, v0, u1, v1 in the atlas.
    pub uv: [f32; 4],
}

/// A glyph placed in pixels (y down) with its atlas UVs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub rect: [f32; 4],
    pub uv: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadFont;

#[derive(Clone, Copy)]
pub struct SdfFont {
    pub atlas_w: u32,
    pub atlas_h: u32,
    pub em_texels: f32,
    /// Distance in atlas texels from the outline to 0 or 255.
    pub spread: f32,
    pub ascent: f32,
    pub descent: f32,
    pub line_height: f32,
    pixels: &'static [u8],
    glyphs: &'static [u8],
    kerns: &'static [u8],
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_bits(u32_at(b, o))
}

impl SdfFont {
    pub fn parse(b: &'static [u8]) -> Result<SdfFont, BadFont> {
        if b.len() < HEADER || &b[..4] != b"A2SD" {
            return Err(BadFont);
        }
        let (w, h) = (u32_at(b, 4), u32_at(b, 8));
        let (ng, nk) = (u32_at(b, 32) as usize, u32_at(b, 36) as usize);
        let g0 = HEADER;
        let k0 = g0 + ng * GLYPH_BYTES;
        let p0 = k0 + nk * KERN_BYTES;
        if b.len() != p0 + (w * h) as usize {
            return Err(BadFont);
        }
        Ok(SdfFont {
            atlas_w: w,
            atlas_h: h,
            em_texels: f32_at(b, 12),
            spread: f32_at(b, 16),
            ascent: f32_at(b, 20),
            descent: f32_at(b, 24),
            line_height: f32_at(b, 28),
            glyphs: &b[g0..k0],
            kerns: &b[k0..p0],
            pixels: &b[p0..],
        })
    }

    /// The UI's regular weight (Noto Sans Condensed Medium).
    pub fn regular() -> SdfFont {
        SdfFont::parse(REGULAR).expect("baked regular font")
    }

    /// The bold weight (Noto Sans Condensed SemiBold).
    pub fn bold() -> SdfFont {
        SdfFont::parse(BOLD).expect("baked bold font")
    }

    pub fn pixels(&self) -> &'static [u8] {
        self.pixels
    }

    fn glyph_count(&self) -> usize {
        self.glyphs.len() / GLYPH_BYTES
    }

    pub fn glyph(&self, c: char) -> Option<Glyph> {
        let (mut lo, mut hi) = (0, self.glyph_count());
        while lo < hi {
            let mid = (lo + hi) / 2;
            let o = mid * GLYPH_BYTES;
            let cp = u32_at(self.glyphs, o);
            if cp == c as u32 {
                let f = |i: usize| f32_at(self.glyphs, o + 8 + i * 4);
                return Some(Glyph { advance: f32_at(self.glyphs, o + 4), bounds: [f(0), f(1), f(2), f(3)], uv: [f(4), f(5), f(6), f(7)] });
            }
            if cp < c as u32 {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        None
    }

    /// Kerning between two characters, in ems.
    pub fn kerning(&self, a: char, b: char) -> f32 {
        let key = (a as u32, b as u32);
        let (mut lo, mut hi) = (0, self.kerns.len() / KERN_BYTES);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let o = mid * KERN_BYTES;
            let k = (u32_at(self.kerns, o), u32_at(self.kerns, o + 4));
            if k == key {
                return f32_at(self.kerns, o + 8);
            }
            if k < key {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        0.0
    }

    /// Width of `s` at `size` pixels per em.
    pub fn measure(&self, s: &str, size: f32) -> f32 {
        let mut x = 0.0;
        let mut prev = None;
        for c in s.chars() {
            let Some(g) = self.glyph(c).or_else(|| self.glyph('?')) else { continue };
            if let Some(p) = prev {
                x += self.kerning(p, c);
            }
            x += g.advance;
            prev = Some(c);
        }
        x * size
    }

    /// Quads for `s` with the pen at `(x, baseline)` in pixels, y down.
    pub fn layout(&self, s: &str, size: f32, x: f32, baseline: f32) -> Vec<Quad> {
        let mut out = Vec::new();
        let mut pen = 0.0;
        let mut prev = None;
        for c in s.chars() {
            let Some(g) = self.glyph(c).or_else(|| self.glyph('?')) else { continue };
            if let Some(p) = prev {
                pen += self.kerning(p, c);
            }
            if g.uv[2] > g.uv[0] {
                let b = g.bounds;
                out.push(Quad {
                    rect: [x + (pen + b[0]) * size, baseline + b[1] * size, x + (pen + b[2]) * size, baseline + b[3] * size],
                    uv: g.uv,
                });
            }
            pen += g.advance;
            prev = Some(c);
        }
        out
    }

    /// The atlas as a single-channel texture, linearly filtered.
    pub fn image(&self) -> Image {
        let mut img = Image::new(
            Extent3d { width: self.atlas_w, height: self.atlas_h, depth_or_array_layers: 1 },
            TextureDimension::D2,
            self.pixels.to_vec(),
            TextureFormat::R8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        );
        img.sampler = ImageSampler::linear();
        img
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_weights_load_and_cover_latin_1() {
        for f in [SdfFont::regular(), SdfFont::bold()] {
            assert_eq!(f.em_texels, 48.0);
            assert_eq!(f.pixels().len(), (f.atlas_w * f.atlas_h) as usize);
            for c in ['A', 'z', '0', ' ', 'é', 'ß', '\u{2013}'] {
                assert!(f.glyph(c).is_some(), "{c:?}");
            }
            assert!(f.glyph('\u{4e2d}').is_none());
        }
    }

    #[test]
    fn glyph_centre_is_inside_the_outline() {
        let f = SdfFont::regular();
        let g = f.glyph('I').unwrap();
        let (u, v) = ((g.uv[0] + g.uv[2]) * 0.5, (g.uv[1] + g.uv[3]) * 0.5);
        let (x, y) = ((u * f.atlas_w as f32) as usize, (v * f.atlas_h as f32) as usize);
        assert!(f.pixels()[y * f.atlas_w as usize + x] > 128);
        let corner = f.pixels()[(g.uv[1] * f.atlas_h as f32) as usize * f.atlas_w as usize + (g.uv[0] * f.atlas_w as f32) as usize];
        assert!(corner < 128);
    }

    #[test]
    fn layout_advances_and_bold_is_wider() {
        let (r, b) = (SdfFont::regular(), SdfFont::bold());
        assert!(r.measure("ENGAGE", 20.0) > r.measure("EN", 20.0));
        assert!(b.measure("ENGAGE", 20.0) >= r.measure("ENGAGE", 20.0));
        let q = r.layout("AV", 20.0, 10.0, 30.0);
        assert_eq!(q.len(), 2);
        assert!(q[1].rect[0] > q[0].rect[0] && q[0].rect[1] < 30.0);
        // Space has an advance and no quad.
        assert_eq!(r.layout(" ", 20.0, 0.0, 0.0).len(), 0);
        assert!(r.measure(" ", 20.0) > 0.0);
    }

    #[test]
    fn corrupt_blobs_are_refused() {
        assert_eq!(SdfFont::parse(b"nope").err(), Some(BadFont));
        assert_eq!(SdfFont::parse(&REGULAR[..REGULAR.len() - 1]).err(), Some(BadFont));
    }
}
