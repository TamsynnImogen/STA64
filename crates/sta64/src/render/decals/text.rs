//! Rasterize registry text with the mounted game's sprite font.
//!
//! Layout and character conversion come from `ui::sprite_font`; this module only
//! samples its glyph rectangles and tints their alpha into a standalone image.

use bevy::math::UVec2;
use sta_assets::Rgba;
use sta_vfs::Vfs;

use crate::ui::sprite_font::{SpriteFont, read_fonts, texel_rect};

const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_WIDTH: u32 = 4096;
const MAX_HEIGHT: u32 = 512;

/// Rasterize text with the game's byte mapping, layout and glyph advances.
pub fn rasterize(
    font: &SpriteFont,
    pages: &[Rgba],
    text: &str,
    colour: [u8; 4],
) -> Result<Rgba, String> {
    if text.is_empty() {
        return Err("registry text is empty".into());
    }
    if text.len() > MAX_TEXT_BYTES {
        return Err(format!(
            "registry text exceeds {MAX_TEXT_BYTES} UTF-8 bytes"
        ));
    }
    if font.pages.is_empty() || pages.len() != font.pages.len() {
        return Err(format!(
            "font has {} pages but {} decoded page textures were supplied",
            font.pages.len(),
            pages.len()
        ));
    }
    if let Some([rw, rh]) = font.reference
        && (!rw.is_finite() || !rh.is_finite() || rw <= 0.0 || rh <= 0.0)
    {
        return Err("font reference dimensions must be finite and positive".into());
    }

    for (index, (page, image)) in font.pages.iter().zip(pages).enumerate() {
        if !page.height.is_finite() || page.height <= 0.0 {
            return Err(format!("font page {index} has invalid glyph height"));
        }
        let expected = usize::try_from(image.width)
            .ok()
            .and_then(|width| {
                usize::try_from(image.height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4));
        if image.width == 0 || image.height == 0 || expected != Some(image.pixels.len()) {
            return Err(format!(
                "font page {index} has invalid decoded dimensions or pixel data"
            ));
        }
        for (glyph_index, glyph) in page.glyphs.iter().enumerate() {
            if !glyph.x.is_finite() || !glyph.y.is_finite() || !glyph.w.is_finite() || glyph.w < 0.0
            {
                return Err(format!(
                    "font page {index} glyph {glyph_index} has invalid coordinates"
                ));
            }
        }
    }

    let layout = font.layout(text, 1.0);
    if !layout.width.is_finite()
        || !layout.height.is_finite()
        || layout.width < 0.0
        || layout.height <= 0.0
    {
        return Err("font layout has invalid output dimensions".into());
    }
    let width = (layout.width.ceil() as u32).max(1);
    let height = layout.height.ceil() as u32;
    if width > MAX_WIDTH || height > MAX_HEIGHT {
        return Err(format!(
            "registry text raster bounds {width}x{height} exceed {MAX_WIDTH}x{MAX_HEIGHT}"
        ));
    }
    let byte_len = usize::try_from(width)
        .ok()
        .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "registry text raster size overflow".to_owned())?;
    let mut output = vec![0; byte_len];

    for quad in layout.quads {
        let Some(page) = pages.get(quad.page) else {
            return Err(format!("font layout references missing page {}", quad.page));
        };
        let [left, top, glyph_width, glyph_height] = quad.rect;
        let [sx, sy, sw, sh] = quad.src;
        if ![left, top, glyph_width, glyph_height, sx, sy, sw, sh]
            .into_iter()
            .all(f32::is_finite)
            || glyph_width <= 0.0
            || glyph_height <= 0.0
            || sw <= 0.0
            || sh <= 0.0
        {
            return Err(format!(
                "font layout contains invalid glyph rectangle for code {}",
                quad.code
            ));
        }
        let source = texel_rect(
            font.reference,
            UVec2::new(page.width, page.height),
            [sx, sy, sw, sh],
        );
        let x0 = (left.floor() as i64).clamp(0, i64::from(width));
        let y0 = (top.floor() as i64).clamp(0, i64::from(height));
        let x1 = ((left + glyph_width).ceil() as i64).clamp(0, i64::from(width));
        let y1 = ((top + glyph_height).ceil() as i64).clamp(0, i64::from(height));

        for y in y0..y1 {
            let center_y = y as f32 + 0.5;
            if center_y < top || center_y >= top + glyph_height {
                continue;
            }
            let v = (center_y - top) / glyph_height;
            let source_y = source.min.y + v * (source.max.y - source.min.y);
            let py = source_y.floor() as i64;
            if py < 0 || py >= i64::from(page.height) {
                continue;
            }
            for x in x0..x1 {
                let center_x = x as f32 + 0.5;
                if center_x < left || center_x >= left + glyph_width {
                    continue;
                }
                let u = (center_x - left) / glyph_width;
                let source_x = source.min.x + u * (source.max.x - source.min.x);
                let px = source_x.floor() as i64;
                if px < 0 || px >= i64::from(page.width) {
                    continue;
                }
                let src_i = (py as usize * page.width as usize + px as usize) * 4;
                let src_alpha =
                    (u16::from(page.pixels[src_i + 3]) * u16::from(colour[3]) + 127) / 255;
                if src_alpha == 0 {
                    continue;
                }
                let dst_i = (y as usize * width as usize + x as usize) * 4;
                let dst_alpha = u16::from(output[dst_i + 3]);
                let alpha = src_alpha + (dst_alpha * (255 - src_alpha) + 127) / 255;
                output[dst_i] = colour[0];
                output[dst_i + 1] = colour[1];
                output[dst_i + 2] = colour[2];
                output[dst_i + 3] = alpha.min(255) as u8;
            }
        }
    }

    Ok(Rgba {
        width,
        height,
        has_alpha: true,
        pixels: output,
    })
}

/// Load a named sprite font and its page images through the active mod VFS.
pub fn from_vfs(vfs: &Vfs, font_name: &str, text: &str, colour: [u8; 4]) -> Result<Rgba, String> {
    let wanted = font_stem(font_name);
    let font = read_fonts(vfs)
        .into_iter()
        .find(|font| font.name.eq_ignore_ascii_case(wanted))
        .ok_or_else(|| format!("sprite font {font_name:?} was not found"))?;
    let mut pages = Vec::with_capacity(font.pages.len());
    for page in &font.pages {
        let (path, format) = sta_assets::find_texture(vfs, &page.texture)
            .ok_or_else(|| format!("font page texture {:?} was not found", page.texture))?;
        let image = sta_assets::decode_rgba(vfs, &path, format)
            .map_err(|error| format!("could not decode font page {:?}: {error:?}", page.texture))?;
        pages.push(image);
    }
    rasterize(&font, &pages, text, colour)
}

fn font_stem(name: &str) -> &str {
    let name = name.trim();
    name.rsplit_once('.')
        .filter(|(_, extension)| extension.eq_ignore_ascii_case("spr"))
        .map_or(name, |(stem, _)| stem)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::sprite_font::{GlyphEntry, Page};

    #[test]
    fn unicode_font_names_do_not_require_ascii_byte_boundaries() {
        assert_eq!(font_stem("字体"), "字体");
        assert_eq!(font_stem(" FontΣ.SPR "), "FontΣ");
        assert_eq!(font_stem("FontSmall"), "FontSmall");
    }

    fn fixture(reference: Option<[f32; 2]>, width: u32, height: u32) -> (SpriteFont, Rgba) {
        let mut glyphs = vec![
            GlyphEntry {
                x: 0.0,
                y: 0.0,
                w: 0.0
            };
            34
        ];
        glyphs[0] = GlyphEntry {
            x: 0.0,
            y: 0.0,
            w: 1.0,
        }; // space advances, has transparent ink
        glyphs[33] = GlyphEntry {
            x: 1.0,
            y: 0.0,
            w: 1.0,
        }; // A
        let font = SpriteFont {
            name: "fontsmall".into(),
            reference,
            pages: vec![Page {
                texture: "synthetic".into(),
                height: 1.0,
                glyphs,
            }],
        };
        let mut pixels = vec![0; width as usize * height as usize * 4];
        if width >= 4 && height >= 2 {
            for y in 0..2 {
                for x in 2..4 {
                    let i = (y * width as usize + x) * 4;
                    pixels[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
        }
        (
            font,
            Rgba {
                width,
                height,
                has_alpha: true,
                pixels,
            },
        )
    }

    #[test]
    fn keeps_space_advance_and_scales_reference_texels_into_tinted_pixels() {
        let (font, page) = fixture(Some([2.0, 1.0]), 4, 2);
        let image = rasterize(&font, &[page], " A", [9, 80, 210, 128]).unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(&image.pixels[..4], &[0, 0, 0, 0]);
        assert_eq!(&image.pixels[4..], &[9, 80, 210, 128]);
        assert!(image.has_alpha);
    }

    #[test]
    fn uses_the_sprite_fonts_existing_windows_1252_mapping() {
        let mut glyphs = vec![
            GlyphEntry {
                x: 0.0,
                y: 0.0,
                w: 0.0
            };
            202
        ];
        glyphs[201] = GlyphEntry {
            x: 0.0,
            y: 0.0,
            w: 1.0,
        }; // 0xe9, 'é'
        let font = SpriteFont {
            name: "f".into(),
            reference: None,
            pages: vec![Page {
                texture: "p".into(),
                height: 1.0,
                glyphs,
            }],
        };
        let page = Rgba {
            width: 1,
            height: 1,
            has_alpha: true,
            pixels: vec![1, 1, 1, 200],
        };
        let image = rasterize(&font, &[page], "é", [10, 20, 30, 255]).unwrap();
        assert_eq!(image.pixels, vec![10, 20, 30, 200]);
    }

    #[test]
    fn clips_source_rectangles_and_rejects_invalid_pages_dimensions_and_long_text() {
        let (mut font, page) = fixture(Some([2.0, 1.0]), 4, 2);
        font.pages[0].glyphs[33].x = -1.0;
        let clipped = rasterize(&font, &[page.clone()], "A", [255; 4]).unwrap();
        assert_eq!((clipped.width, clipped.height), (1, 1));
        assert!(clipped.pixels.iter().all(|byte| *byte == 0));

        assert!(
            rasterize(&font, &[], "A", [255; 4])
                .unwrap_err()
                .contains("pages")
        );
        let invalid_page = Rgba {
            width: 4,
            height: 2,
            has_alpha: true,
            pixels: vec![0; 3],
        };
        assert!(
            rasterize(&font, &[invalid_page], "A", [255; 4])
                .unwrap_err()
                .contains("dimensions")
        );

        let (large, page) = fixture(Some([1.0, 1.0]), 1, 1);
        assert!(
            rasterize(&large, &[page], &"A".repeat(65_537), [255; 4])
                .unwrap_err()
                .contains("bytes")
        );
    }

    #[test]
    fn rejects_empty_text_and_layouts_over_the_output_bounds() {
        let (font, page) = fixture(Some([2.0, 1.0]), 4, 2);
        assert!(
            rasterize(&font, &[page.clone()], "", [255; 4])
                .unwrap_err()
                .contains("empty")
        );
        let mut wide = font.clone();
        wide.pages[0].glyphs[33].w = 4097.0;
        assert!(
            rasterize(&wide, &[page], "A", [255; 4])
                .unwrap_err()
                .contains("bounds")
        );
    }
}
