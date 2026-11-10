//! The games' own sprite fonts (UI-12): `Sprites/Font*.spr`, resolved through the
//! mod chain, drawn as one image node per glyph. UI-03's baked TTF stays for STA64's
//! own screens and is the fallback wherever the mounted mod has no font.
//!
//! **File format** (as the original Tooltip Lab and Refit read it). A font `.spr` is a
//! table with a `@referenceWidth`/`@referenceHeight` (the texel space of the numbers;
//! absent in Armada II's own files, where the texture's size stands in) and one or more
//! `page<a,b,c>` lines: `pagea<TAB>texture<TAB>..<TAB>glyph height<TAB>@anim=<uv><TAB>@anim=<w>`.
//! Each page names two animation strips (`@animation <id>` ... `@keyframes`, one line per
//! glyph): `<id>_uv` holds each glyph's top left corner `x y`, `<id>_w` its width. Pages
//! continue one another: page a holds codes 0x20.., page b the codes after page a's last
//! entry, and so on. The byte -> glyph table is therefore the strips themselves; it
//! differs per mod (Roots has one page per font on a 1024 wide sheet, the Classic mod
//! three on 256 wide ones) and holds the single-byte glyphs the game draws as pictures
//! (resource symbols at 0x80 and up) like any other code. Text is Windows-1252.
//!
//! [`SpriteFont::parse`] and [`SpriteFont::layout`] are pure; [`SpriteFontPlugin`]
//! loads every font of the mounted install into [`SpriteFonts`] and turns each
//! [`SpriteText`] into glyph nodes, or into a plain `Text` when there is no font.

use std::collections::BTreeMap;

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use sta_vfs::Vfs;

use crate::viewer::Mounted;

/// One glyph of a page: its top left corner and width, in the page's reference texels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphEntry {
    pub x: f32,
    pub y: f32,
    pub w: f32,
}

/// One texture of a font and the glyphs on it.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    /// Texture name, no extension.
    pub texture: String,
    /// Glyph height in texels.
    pub height: f32,
    /// The glyphs of this page, in code order.
    pub glyphs: Vec<GlyphEntry>,
}

/// A parsed font file.
#[derive(Clone, Debug, PartialEq)]
pub struct SpriteFont {
    /// File stem as the VFS reports it (lower case), e.g. `fontsmall`.
    pub name: String,
    /// `@referenceWidth`/`@referenceHeight` when the file sets them.
    pub reference: Option<[f32; 2]>,
    pub pages: Vec<Page>,
}

/// Where a code lives: page, index in the page.
pub const FIRST_CODE: u8 = 0x20;

impl SpriteFont {
    /// Read one font file. `None` when it holds no page with a glyph strip.
    pub fn parse(name: &str, text: &str) -> Option<SpriteFont> {
        let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
        let num = |key: &str| {
            lines
                .iter()
                .find_map(|l| l.strip_prefix(key))
                .and_then(|v| v.split('#').next()?.trim().parse::<f32>().ok())
        };
        let reference = num("@referenceWidth=")
            .zip(num("@referenceHeight="))
            .map(|(w, h)| [w, h]);
        let mut pages = Vec::new();
        for l in lines.iter().filter(|l| l.starts_with("page")) {
            let f: Vec<&str> = l.split('\t').collect();
            let texture = f.get(1).map(|s| s.trim()).unwrap_or("");
            // The glyph height is the last plain number before the `@anim=` fields (Roots'
            // files have an empty column before the numbers, the Classic mod's do not).
            let height = f
                .iter()
                .skip(2)
                .take_while(|s| !s.trim().starts_with("@anim="))
                .filter_map(|s| s.trim().parse::<f32>().ok())
                .last()
                .unwrap_or(0.0);
            let anims: Vec<&str> = f
                .iter()
                .filter_map(|s| s.trim().strip_prefix("@anim="))
                .collect();
            let strip_of = |suffix: &str| anims.iter().copied().find(|a| a.ends_with(suffix));
            let (Some(uv_id), Some(w_id)) = (strip_of("_uv"), strip_of("_w")) else {
                continue;
            };
            let uv = strip(&lines, uv_id);
            let w = strip(&lines, w_id);
            if texture.is_empty() || height <= 0.0 || uv.is_empty() || uv.len() != w.len() {
                continue;
            }
            let glyphs = uv
                .iter()
                .zip(&w)
                .filter_map(|(uv, w)| {
                    Some(GlyphEntry {
                        x: *uv.first()?,
                        y: *uv.get(1)?,
                        w: *w.first()?,
                    })
                })
                .collect::<Vec<_>>();
            if glyphs.len() != uv.len() {
                continue;
            }
            pages.push(Page {
                texture: texture.to_owned(),
                height,
                glyphs,
            });
        }
        (!pages.is_empty()).then(|| SpriteFont {
            name: name.to_owned(),
            reference,
            pages,
        })
    }

    /// Glyph height in texels (the first page's; pages of one font share it).
    pub fn height(&self) -> f32 {
        self.pages.first().map_or(0.0, |p| p.height)
    }

    /// The glyph for a byte: its page index and entry. `None` below 0x20 or past the table.
    pub fn glyph(&self, code: u8) -> Option<(usize, GlyphEntry)> {
        let mut i = usize::from(code.checked_sub(FIRST_CODE)?);
        for (p, page) in self.pages.iter().enumerate() {
            if i < page.glyphs.len() {
                return Some((p, page.glyphs[i]));
            }
            i -= page.glyphs.len();
        }
        None
    }

    /// How many codes the table covers (from 0x20).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.pages.iter().map(|p| p.glyphs.len()).sum()
    }

    /// Place `text`: one quad per drawable glyph, pen advancing by each glyph's width,
    /// `\n` starting a line `height` further down. Everything is scaled by `scale`.
    /// A character the font cannot draw is drawn as `?`, then skipped when that is
    /// missing too.
    pub fn layout(&self, text: &str, scale: f32) -> Layout {
        let h = self.height();
        let (mut x, mut y, mut width) = (0.0f32, 0.0f32, 0.0f32);
        let mut quads = Vec::new();
        for ch in text.chars() {
            if ch == '\n' {
                width = width.max(x);
                x = 0.0;
                y += h * scale;
                continue;
            }
            let code = windows_1252(ch);
            let found = self
                .glyph(code)
                .filter(|(_, g)| g.w > 0.0)
                .or_else(|| self.glyph(b'?'));
            let Some((page, g)) = found else { continue };
            // A space and any other blank glyph still advance; the quad is drawn regardless.
            quads.push(GlyphQuad {
                code,
                page,
                rect: [x, y, g.w * scale, h * scale],
                src: [g.x, g.y, g.w, h],
            });
            x += g.w * scale;
        }
        let lines = text.split('\n').count().max(1);
        Layout {
            quads,
            width: width.max(x),
            height: h * scale * lines as f32,
        }
    }
}

/// The numbers of an animation strip: one entry per line after `@keyframes`, up to the
/// first blank or `@` line.
fn strip(lines: &[&str], id: &str) -> Vec<Vec<f32>> {
    let Some(at) = lines
        .iter()
        .position(|l| l.starts_with("@animation") && l.split_whitespace().nth(1) == Some(id))
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut started = false;
    for l in &lines[at + 1..] {
        let l = l.split('#').next().unwrap_or("").trim();
        if !started {
            started = l == "@keyframes";
            continue;
        }
        if l.is_empty() || l.starts_with('@') {
            break;
        }
        out.push(
            l.split_whitespace()
                .filter_map(|n| n.parse::<f32>().ok())
                .collect(),
        );
    }
    out
}

/// A glyph placed by [`SpriteFont::layout`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphQuad {
    pub code: u8,
    pub page: usize,
    /// Left, top, width, height in output pixels.
    pub rect: [f32; 4],
    /// x, y, w, h in the page's reference texels.
    pub src: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub quads: Vec<GlyphQuad>,
    pub width: f32,
    pub height: f32,
}

/// A glyph's source rectangle in the texture's own pixels: reference texels scaled by
/// texture size over reference size (the file's `@reference*`, else the texture's own
/// size). `src` is `[x, y, w, h]` as in [`GlyphQuad::src`].
pub fn texel_rect(reference: Option<[f32; 2]>, tex: UVec2, src: [f32; 4]) -> Rect {
    let r = reference.unwrap_or([tex.x as f32, tex.y as f32]);
    let (kx, ky) = (tex.x as f32 / r[0], tex.y as f32 / r[1]);
    Rect::new(
        src[0] * kx,
        src[1] * ky,
        (src[0] + src[2]) * kx,
        (src[1] + src[3]) * ky,
    )
}

/// Share of the letters and digits whose rectangle, mapped onto the texture, holds ink
/// and is bounded by empty columns left and right of it. A font whose reference size
/// does not match its strips (Fleet Ops 4.0's FontMedium/FontLarge/FontVeryLarge, whose
/// strips were cut for another texture size) lands rectangles across neighbouring
/// glyphs and scores near zero; a well-formed font scores near one. `None` when the
/// page has none of those glyphs.
pub fn page_fit(font: &SpriteFont, page: usize, tex: UVec2, rgba: &[u8]) -> Option<f32> {
    let p = font.pages.get(page)?;
    let ink = |x: i64, y: i64| {
        if x < 0 || y < 0 || x >= i64::from(tex.x) || y >= i64::from(tex.y) {
            return false;
        }
        let i = (y as usize * tex.x as usize + x as usize) * 4;
        let px = rgba.get(i..i + 4).unwrap_or(&[0; 4]);
        let lum = u32::from(px[0].max(px[1]).max(px[2]));
        u32::from(px[3]) * lum / 255 > 24
    };
    let (mut tested, mut good) = (0u32, 0u32);
    for code in (b'0'..=b'9').chain(b'A'..=b'Z').chain(b'a'..=b'z') {
        let Some((gp, g)) = font.glyph(code) else {
            continue;
        };
        if gp != page || g.w <= 0.0 {
            continue;
        }
        let r = texel_rect(font.reference, tex, [g.x, g.y, g.w, p.height]);
        let (x0, x1) = (r.min.x.round() as i64, r.max.x.round() as i64);
        let (y0, y1) = (r.min.y.round() as i64, r.max.y.round() as i64);
        let inside = (y0..y1).any(|y| (x0..x1).any(|x| ink(x, y)));
        // Bounded: no ink in the column just outside, left and right.
        let edge = |x: i64| (y0..y1).filter(|y| ink(x, *y)).count();
        let clean = edge(x0 - 1) <= 1 && edge(x1) <= 1;
        tested += 1;
        good += u32::from(inside && clean);
    }
    (tested > 0).then(|| good as f32 / tested as f32)
}

/// Fonts scoring below this are not drawn (see [`page_fit`]).
pub const MIN_FIT: f32 = 0.9;

/// A character as the game's single byte (Windows-1252); anything else is `?`.
pub fn windows_1252(c: char) -> u8 {
    const HIGH: [char; 32] = [
        '\u{20ac}', '\u{81}', '\u{201a}', '\u{192}', '\u{201e}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{2c6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8d}', '\u{17d}',
        '\u{8f}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2013}',
        '\u{2014}', '\u{2dc}', '\u{2122}', '\u{161}', '\u{203a}', '\u{153}', '\u{9d}', '\u{17e}',
        '\u{178}',
    ];
    let u = c as u32;
    if u < 0x80 || (0xa0..0x100).contains(&u) {
        return u as u8;
    }
    HIGH.iter()
        .position(|h| *h == c)
        .map_or(b'?', |i| 0x80 + i as u8)
}

/// One loaded font: its table and, per page, the texture and its pixel size.
pub struct LoadedFont {
    pub font: SpriteFont,
    pub pages: Vec<(Handle<Image>, UVec2)>,
}

/// Every sprite font of the mounted install, by file stem. Absent until a mod is
/// mounted and at least one font and its textures were read.
#[derive(Resource, Default)]
pub struct SpriteFonts {
    pub fonts: BTreeMap<String, LoadedFont>,
}

impl SpriteFonts {
    /// The font whose glyph height is nearest `size` px.
    pub fn pick(&self, size: f32) -> Option<&LoadedFont> {
        self.fonts.values().min_by(|a, b| {
            (a.font.height() - size)
                .abs()
                .total_cmp(&(b.font.height() - size).abs())
        })
    }
}

/// The `Sprites/Font*.spr` files the VFS shows, parsed (no textures).
pub fn read_fonts(vfs: &Vfs) -> Vec<SpriteFont> {
    let mut out = Vec::new();
    for path in vfs.list("sprites") {
        let file = path.rsplit('/').next().unwrap_or(&path);
        let lower = file.to_ascii_lowercase();
        if !lower.starts_with("font") || !lower.ends_with(".spr") {
            continue;
        }
        let Some(Ok(bytes)) = vfs.read(&path) else {
            continue;
        };
        let stem = &file[..file.len() - 4];
        if let Some(f) = SpriteFont::parse(stem, &String::from_utf8_lossy(&bytes)) {
            out.push(f);
        }
    }
    out
}

/// Decode a page texture: its pixels and size.
fn decode_page(vfs: &Vfs, texture: &str) -> Option<(Vec<u8>, UVec2)> {
    let (path, fmt) = sta_assets::find_texture(vfs, texture)?;
    let rgba = sta_assets::decode_rgba(vfs, &path, fmt).ok()?;
    Some((rgba.pixels, UVec2::new(rgba.width, rgba.height)))
}

fn upload_page(pixels: Vec<u8>, size: UVec2, images: &mut Assets<Image>) -> (Handle<Image>, UVec2) {
    let mut img = Image::new(
        Extent3d {
            width: size.x,
            height: size.y,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = ImageSampler::nearest(); // "No filtering, ever" (the fonts' own note).
    (images.add(img), size)
}

/// (Re)load the fonts when the mounted install changes.
fn load_fonts(
    vfs: Option<Res<Mounted>>,
    images: Option<ResMut<Assets<Image>>>,
    mut fonts: ResMut<SpriteFonts>,
) {
    let (Some(vfs), Some(mut images)) = (vfs, images) else {
        return;
    };
    if !vfs.is_changed() {
        return;
    }
    fonts.fonts.clear();
    for font in read_fonts(&vfs.0) {
        let decoded: Vec<_> = font
            .pages
            .iter()
            .filter_map(|p| decode_page(&vfs.0, &p.texture))
            .collect();
        if decoded.len() != font.pages.len() {
            continue;
        }
        let fits = decoded
            .iter()
            .enumerate()
            .all(|(i, (px, size))| page_fit(&font, i, *size, px).is_none_or(|f| f >= MIN_FIT));
        if !fits {
            info!(
                "sprite font {} skipped: its strips do not fit its texture",
                font.name
            );
            continue;
        }
        let pages = decoded
            .into_iter()
            .map(|(px, size)| upload_page(px, size, &mut images))
            .collect();
        fonts
            .fonts
            .insert(font.name.clone(), LoadedFont { font, pages });
    }
    info!("sprite fonts: {} loaded", fonts.fonts.len());
}

/// Text to draw in the mod's font when it has one, in the TTF otherwise. `size` is the
/// pixel size the TTF would be given; the nearest sprite font is drawn scaled to it.
#[derive(Component, Clone, Debug)]
pub struct SpriteText {
    pub text: String,
    pub size: f32,
    pub color: Color,
}

/// The bundle for a [`SpriteText`] (resolved on the next `PostUpdate`).
pub fn sprite_text(text: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (
        SpriteText {
            text: text.into(),
            size,
            color,
        },
        Pickable::IGNORE,
    )
}

fn resolve_text(
    mut commands: Commands,
    fonts: Res<SpriteFonts>,
    q: Query<(Entity, &SpriteText), Changed<SpriteText>>,
) {
    for (e, t) in &q {
        let mut ec = commands.entity(e);
        ec.despawn_related::<Children>();
        let Some(f) = fonts.pick(t.size) else {
            ec.with_child((
                Text::new(t.text.clone()),
                TextFont {
                    font_size: bevy::text::FontSize::Px(t.size),
                    ..default()
                },
                TextColor(t.color),
                Pickable::IGNORE,
            ));
            continue;
        };
        let scale = t.size / f.font.height();
        let lay = f.font.layout(&t.text, scale);
        ec.insert(Node {
            width: Val::Px(lay.width.ceil()),
            height: Val::Px(lay.height.ceil()),
            flex_shrink: 0.0,
            ..default()
        });
        for q in &lay.quads {
            let Some((image, tex)) = f.pages.get(q.page) else {
                continue;
            };
            let rect = texel_rect(f.font.reference, *tex, q.src);
            ec.with_child((
                ImageNode {
                    image: image.clone(),
                    rect: Some(rect),
                    color: t.color,
                    ..default()
                },
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(q.rect[0]),
                    top: Val::Px(q.rect[1]),
                    width: Val::Px(q.rect[2]),
                    height: Val::Px(q.rect[3]),
                    ..default()
                },
                Pickable::IGNORE,
            ));
        }
    }
}

pub struct SpriteFontPlugin;

impl Plugin for SpriteFontPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpriteFonts>().add_systems(
            PostUpdate,
            (load_fonts, resolve_text)
                .chain()
                .before(bevy::ui::UiSystems::Prepare),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const HEROIC: &str = "/home/cedric/Games/Heroic";

    const SAMPLE: &str = "sprite_table\n@referenceWidth=256\n@referenceHeight=128\n\n@animation f_a_uv\noffset 3 0.0 step\n@keyframes\n0\t0\t# \n10\t0\t#!\n20\t0\t#\"\n\n@animation f_a_w\nwidth 3 0.0 step\n@keyframes\n5\n8.5\n6\n\n@animation f_b_uv\noffset 2 0.0 step\n@keyframes\n0\t30\n12\t30\n\n@animation f_b_w\nwidth 2 0.0 step\n@keyframes\n7\n9\n\npagea\ttex_a\t\t0\t0\t0\t12\t@anim=f_a_uv\t@anim=f_a_w\npageb\ttex_b\t\t0\t0\t0\t12\t@anim=f_b_uv\t@anim=f_b_w\n";

    #[test]
    fn parse_reads_pages_as_one_continuing_table() {
        let f = SpriteFont::parse("F", SAMPLE).unwrap();
        assert_eq!(
            (f.reference, f.pages.len(), f.len(), f.height()),
            (Some([256.0, 128.0]), 2, 5, 12.0)
        );
        assert_eq!(
            f.glyph(0x21),
            Some((
                0,
                GlyphEntry {
                    x: 10.0,
                    y: 0.0,
                    w: 8.5
                }
            ))
        );
        // Code 0x23 is the first of page b.
        assert_eq!(
            f.glyph(0x23),
            Some((
                1,
                GlyphEntry {
                    x: 0.0,
                    y: 30.0,
                    w: 7.0
                }
            ))
        );
        assert_eq!(f.glyph(0x24).map(|g| g.1.w), Some(9.0));
        assert_eq!((f.glyph(0x25), f.glyph(0x1f)), (None, None));
    }

    #[test]
    fn layout_advances_by_glyph_width_and_breaks_lines() {
        let f = SpriteFont::parse("F", SAMPLE).unwrap();
        let l = f.layout(" !\n\"", 2.0);
        let rects: Vec<[f32; 4]> = l.quads.iter().map(|q| q.rect).collect();
        assert_eq!(
            rects,
            [
                [0.0, 0.0, 10.0, 24.0],
                [10.0, 0.0, 17.0, 24.0],
                [0.0, 24.0, 12.0, 24.0]
            ]
        );
        assert_eq!((l.width, l.height), (27.0, 48.0));
        assert_eq!(l.quads[1].src, [10.0, 0.0, 8.5, 12.0]);
        // A code the table lacks falls back to `?`, which this table lacks too: skipped.
        assert_eq!(f.layout("~", 1.0).quads.len(), 0);
    }

    #[test]
    fn windows_1252_maps_the_games_bytes() {
        assert_eq!(
            (
                windows_1252('A'),
                windows_1252('\u{e9}'),
                windows_1252('\u{2122}'),
                windows_1252('\u{4e2d}')
            ),
            (b'A', 0xe9, 0x99, b'?')
        );
    }

    #[test]
    fn nearest_font_by_height() {
        let mut fonts = SpriteFonts::default();
        assert!(fonts.pick(12.0).is_none());
        for (n, h) in [("S", 14.0), ("L", 23.0)] {
            let mut f = SpriteFont::parse(n, SAMPLE).unwrap();
            f.pages[0].height = h;
            fonts.fonts.insert(
                n.into(),
                LoadedFont {
                    font: f,
                    pages: vec![],
                },
            );
        }
        assert_eq!(fonts.pick(12.0).unwrap().font.name, "S");
        assert_eq!(fonts.pick(22.0).unwrap().font.name, "L");
    }

    /// Resolves to glyph nodes with a font, to a plain `Text` without one.
    #[test]
    fn sprite_text_resolves_both_ways() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<SpriteFonts>()
            .add_systems(Update, resolve_text);
        let e = app
            .world_mut()
            .spawn(sprite_text("Hi", 12.0, Color::WHITE))
            .id();
        app.update();
        let kids = app.world().get::<Children>(e).unwrap();
        assert_eq!(
            app.world().get::<Text>(kids[0]).map(|t| t.0.as_str()),
            Some("Hi")
        );

        let mut f = SpriteFont::parse("F", SAMPLE).unwrap();
        f.reference = None;
        app.world_mut().resource_mut::<SpriteFonts>().fonts.insert(
            "F".into(),
            LoadedFont {
                font: f,
                pages: vec![
                    (Handle::default(), UVec2::new(256, 128)),
                    (Handle::default(), UVec2::new(256, 128)),
                ],
            },
        );
        let e = app
            .world_mut()
            .spawn(sprite_text("  ", 12.0, Color::WHITE))
            .id();
        app.update();
        let kids = app.world().get::<Children>(e).unwrap();
        assert_eq!(kids.len(), 2);
        assert!(
            app.world()
                .get::<ImageNode>(kids[0])
                .is_some_and(|i| i.rect.is_some())
        );
    }

    /// `(label, install root, active mod)` for every install this machine may have.
    fn installs() -> Vec<(String, std::path::PathBuf, Option<String>)> {
        let roots = Path::new(HEROIC).join("Star Trek Armada II Fleet Operations Roots");
        let mut v = vec![
            (
                "FO4".to_owned(),
                roots.clone(),
                Some("Fleet Ops 4.0".to_owned()),
            ),
            (
                "Classic".to_owned(),
                roots.clone(),
                Some("STA2 Classic Mod".to_owned()),
            ),
            ("Roots".to_owned(), roots, None),
            (
                "FO".to_owned(),
                Path::new(HEROIC).join("Star Trek Armada II Fleet Operations"),
                None,
            ),
            (
                "DW".to_owned(),
                Path::new(HEROIC).join("Star Trek Armada II Dominion War"),
                None,
            ),
            (
                "A2".to_owned(),
                Path::new(HEROIC).join("Star Trek Armada II (Stock)"),
                None,
            ),
        ];
        if let Ok(root) = std::env::var("STA64_TEST_INSTALL") {
            v.push((
                "env".into(),
                root.into(),
                std::env::var("STA64_TEST_MOD")
                    .ok()
                    .filter(|m| !m.is_empty()),
            ));
        }
        v
    }

    /// Install-gated: every mounted install's fonts parse, every page's texture
    /// resolves, and the tables of different mods are not the same.
    #[test]
    fn glyph_tables_are_read_per_mod_and_differ() {
        let mut tables: Vec<(String, Vec<Page>)> = Vec::new();
        for (label, root, m) in installs() {
            let Ok(vfs) = Vfs::mount_install(&root, m.as_deref()) else {
                eprintln!("{label}: skipped");
                continue;
            };
            let fonts = read_fonts(&vfs);
            eprintln!(
                "{label}: {} fonts: {}",
                fonts.len(),
                fonts
                    .iter()
                    .map(|f| format!(
                        "{}({}p,{}g,h{})",
                        f.name,
                        f.pages.len(),
                        f.len(),
                        f.height()
                    ))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            for f in &fonts {
                for p in &f.pages {
                    // Some mod fonts name textures the mod does not ship (Classic's
                    // fontverylarge24); the loader skips those fonts. FontSmall must resolve.
                    let found = sta_assets::find_texture(&vfs, &p.texture).is_some();
                    if !found {
                        eprintln!("{label}: {} names missing texture {}", f.name, p.texture);
                    }
                    assert!(
                        found || !f.name.eq_ignore_ascii_case("FontSmall"),
                        "{label}: FontSmall names missing texture {}",
                        p.texture
                    );
                }
                assert!(
                    f.glyph(b'A').is_some_and(|(_, g)| g.w > 0.0),
                    "{label}: {} has no A",
                    f.name
                );
            }
            if let Some(f) = fonts
                .into_iter()
                .find(|f| f.name.eq_ignore_ascii_case("FontSmall"))
            {
                tables.push((label, f.pages));
            }
        }
        if tables.len() < 2 {
            eprintln!("fewer than two installs with FontSmall: difference not checked");
            return;
        }
        let differing = tables
            .iter()
            .skip(1)
            .filter(|(_, p)| *p != tables[0].1)
            .count();
        assert!(
            differing > 0,
            "every install's FontSmall table is identical: {:?}",
            tables.iter().map(|t| &t.0).collect::<Vec<_>>()
        );
    }

    /// Glyph rectangles of "DILITHIUM 4000" in Fleet Ops 4.0's FontSmall, read off the
    /// strips and checked by eye against the 1024x256 `all_interface_font20` sheet
    /// (texture and reference sizes are equal, so source texels are texture pixels).
    #[test]
    fn fo4_fontsmall_lays_out_known_strings() {
        let roots = Path::new(HEROIC).join("Star Trek Armada II Fleet Operations Roots");
        let Ok(vfs) = Vfs::mount_install(&roots, Some("Fleet Ops 4.0")) else {
            eprintln!("FO4 not installed: skipped");
            return;
        };
        let font = read_fonts(&vfs)
            .into_iter()
            .find(|f| f.name.eq_ignore_ascii_case("FontSmall"))
            .expect("FontSmall");
        assert_eq!(
            (font.reference, font.height()),
            (Some([1024.0, 256.0]), 23.0)
        );
        let l = font.layout("DILITHIUM 4000", 1.0);
        let want: [(u8, [f32; 4]); 14] = [
            (b'D', [561.0, 2.0, 11.0, 23.0]),
            (b'I', [632.0, 2.0, 5.0, 23.0]),
            (b'L', [669.0, 2.0, 7.0, 23.0]),
            (b'I', [632.0, 2.0, 5.0, 23.0]),
            (b'T', [789.0, 2.0, 9.0, 23.0]),
            (b'H', [616.0, 2.0, 10.0, 23.0]),
            (b'I', [632.0, 2.0, 5.0, 23.0]),
            (b'U', [804.0, 2.0, 10.0, 23.0]),
            (b'M', [680.0, 2.0, 16.0, 23.0]),
            (b' ', [0.0, 102.0, 6.0, 23.0]),
            (b'4', [291.0, 2.0, 12.0, 23.0]),
            (b'0', [234.0, 2.0, 12.0, 23.0]),
            (b'0', [234.0, 2.0, 12.0, 23.0]),
            (b'0', [234.0, 2.0, 12.0, 23.0]),
        ];
        assert_eq!(l.quads.len(), want.len());
        for (q, (c, src)) in l.quads.iter().zip(want) {
            assert_eq!((q.code, q.src), (c, src), "glyph {}", c as char);
        }
        // The pen advances by the widths: D 11, I 5, L 7, I 5, T 9, H 10, I 5, U 10, M 16.
        assert_eq!(
            l.quads[9].rect[0],
            11.0 + 5.0 + 7.0 + 5.0 + 9.0 + 10.0 + 5.0 + 10.0 + 16.0
        );
        let digits = font.layout("0123456789", 1.0);
        assert_eq!(digits.quads.len(), 10);
        let zero = digits.quads[0].src;
        assert_eq!((zero[0], zero[1]), (234.0, 2.0));
        // Equal texture and reference size: the texel rect is the source rect.
        let r = texel_rect(font.reference, UVec2::new(1024, 256), l.quads[0].src);
        assert_eq!(
            (r.min.x, r.min.y, r.max.x, r.max.y),
            (561.0, 2.0, 572.0, 25.0)
        );
    }

    /// Install-gated: the mod's FontSmall and the FontFinal4 pages fit their sheets;
    /// FontMedium/FontLarge/FontVeryLarge (strips cut for other sizes) do not.
    #[test]
    fn fo4_fonts_fit_or_are_rejected() {
        let roots = Path::new(HEROIC).join("Star Trek Armada II Fleet Operations Roots");
        let Ok(vfs) = Vfs::mount_install(&roots, Some("Fleet Ops 4.0")) else {
            eprintln!("FO4 not installed: skipped");
            return;
        };
        for f in read_fonts(&vfs) {
            let Some((px, size)) = decode_page(&vfs, &f.pages[0].texture) else {
                continue;
            };
            let fit = page_fit(&f, 0, size, &px).unwrap();
            eprintln!("{}: fit {fit:.2}", f.name);
            let n = f.name.to_ascii_lowercase();
            let good = matches!(
                n.as_str(),
                "fontsmall" | "fontfinal4_10" | "fontfinal4_13" | "fontfinal4_16"
            );
            let bad = matches!(n.as_str(), "fontmedium" | "fontlarge" | "fontverylarge");
            assert!(!good || fit >= MIN_FIT, "{} should fit: {fit}", f.name);
            assert!(!bad || fit < MIN_FIT, "{} should not fit: {fit}", f.name);
        }
    }

    /// A synthetic sheet: glyphs where the strips say pass, a wrong reference fails.
    #[test]
    fn page_fit_tells_aligned_from_shifted() {
        let mut f = SpriteFont::parse("F", SAMPLE).unwrap();
        f.pages.truncate(1);
        f.pages[0].glyphs = (0..0x60u32 - 0x20)
            .map(|n| GlyphEntry {
                x: 20.0 * (n % 12) as f32 + 2.0,
                y: 20.0 * (n / 12) as f32,
                w: 8.0,
            })
            .collect();
        f.pages[0].height = 10.0;
        f.reference = Some([256.0, 128.0]);
        let tex = UVec2::new(256, 128);
        let mut px = vec![0u8; 256 * 128 * 4];
        for g in &f.pages[0].glyphs {
            for y in 0..10 {
                for x in 0..8 {
                    let i = ((g.y as usize + y) * 256 + g.x as usize + x) * 4;
                    px[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
        }
        assert_eq!(page_fit(&f, 0, tex, &px), Some(1.0));
        // A reference twice the sheet: every rectangle lands at the wrong place.
        f.reference = Some([512.0, 256.0]);
        assert!(page_fit(&f, 0, tex, &px).unwrap() < MIN_FIT);
    }
}
