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

use std::collections::{BTreeMap, BTreeSet};

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

/// Horizontal alignment of a block of text inside its width.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// Greedy word wrap: lines of `text` no wider than `max` by `measure`, breaking at
/// spaces (a word wider than `max` breaks between characters) and at `\n`. The measure
/// is a parameter so the sprite fonts and the TTF share one wrapping rule.
pub fn wrap_lines(text: &str, max: f32, measure: &dyn Fn(&str) -> f32) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        let mut first = true;
        for word in para.split(' ') {
            if first {
                line = word.to_owned();
                first = false;
            } else {
                let candidate = format!("{line} {word}");
                if measure(&candidate) <= max {
                    line = candidate;
                    continue;
                }
                out.push(std::mem::take(&mut line));
                line = word.to_owned();
            }
            if measure(&line) > max {
                // A single word wider than `max`: break it between characters.
                let mut part = String::new();
                for c in std::mem::take(&mut line).chars() {
                    let mut next = part.clone();
                    next.push(c);
                    if !part.is_empty() && measure(&next) > max {
                        out.push(std::mem::take(&mut part));
                    }
                    part.push(c);
                }
                line = part;
            }
        }
        out.push(line);
    }
    out
}

impl SpriteFont {
    /// Width of one line of `text` at `scale` (no `\n`).
    pub fn measure(&self, text: &str, scale: f32) -> f32 {
        self.layout(text, scale).width
    }

    /// A block of text: wrapped to `max_width` when given, each line aligned inside the
    /// block (`max_width`, else the widest line). The block's `width` is that width and
    /// its `height` is one glyph height per line.
    pub fn layout_block(&self, text: &str, scale: f32, max_width: Option<f32>, align: Align) -> Layout {
        let lines = match max_width {
            Some(w) => wrap_lines(text, w, &|s| self.measure(s, scale)),
            None => text.split('\n').map(str::to_owned).collect(),
        };
        let h = self.height() * scale;
        let laid: Vec<Layout> = lines.iter().map(|l| self.layout(l, scale)).collect();
        let widest = laid.iter().fold(0.0f32, |m, l| m.max(l.width));
        // Left aligned text takes the width it needs; centred and right aligned text is
        // aligned inside the whole box.
        let width = match (max_width, align) {
            (Some(w), Align::Center | Align::Right) => w,
            _ => widest,
        };
        let mut quads = Vec::new();
        for (i, l) in laid.iter().enumerate() {
            let dx = match align {
                Align::Left => 0.0,
                Align::Center => ((width - l.width) / 2.0).max(0.0),
                Align::Right => (width - l.width).max(0.0),
            };
            for q in &l.quads {
                let mut q = *q;
                q.rect[0] += dx;
                q.rect[1] += i as f32 * h;
                quads.push(q);
            }
        }
        Layout { quads, width, height: h * laid.len().max(1) as f32 }
    }
}

/// The game's named fonts. The GUI cfgs do not name font files (docs/STOCK_HUD_CFG.md);
/// the games name them by role: the `Font<Role>.spr` of the mounted mod chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum FontRole {
    Small,
    Medium,
    Large,
    VeryLarge,
}

#[allow(dead_code)]
impl FontRole {
    pub const ALL: [FontRole; 4] = [Self::Small, Self::Medium, Self::Large, Self::VeryLarge];

    /// The font file stem the VFS reports.
    pub fn stem(self) -> &'static str {
        match self {
            Self::Small => "fontsmall",
            Self::Medium => "fontmedium",
            Self::Large => "fontlarge",
            Self::VeryLarge => "fontverylarge",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.stem().eq_ignore_ascii_case(name.trim()))
    }
}

/// Codes of `page` the font draws as coloured pictures (resource symbols, the red
/// figures of Roots): glyphs whose ink is mostly not grey. Read from the page's pixels,
/// so each mod's table decides. These are drawn untinted.
pub fn coloured_glyphs(font: &SpriteFont, page: usize, tex: UVec2, rgba: &[u8]) -> Vec<u8> {
    let Some(p) = font.pages.get(page) else { return Vec::new() };
    let mut out = Vec::new();
    for code in FIRST_CODE..=255u8 {
        let Some((gp, g)) = font.glyph(code) else { continue };
        // Letters and digits are text even in a font drawn in colour; only the other
        // codes can be pictures.
        if gp != page || g.w <= 0.0 || code.is_ascii_alphanumeric() {
            continue;
        }
        let r = texel_rect(font.reference, tex, [g.x, g.y, g.w, p.height]);
        let (mut ink, mut colour) = (0u32, 0u32);
        for y in (r.min.y.round() as i64).max(0)..(r.max.y.round() as i64).min(i64::from(tex.y)) {
            for x in (r.min.x.round() as i64).max(0)..(r.max.x.round() as i64).min(i64::from(tex.x)) {
                let i = (y as usize * tex.x as usize + x as usize) * 4;
                let Some(px) = rgba.get(i..i + 4) else { continue };
                let (hi, lo) = (px[0].max(px[1]).max(px[2]), px[0].min(px[1]).min(px[2]));
                if px[3] > 128 && hi > 64 {
                    ink += 1;
                    colour += u32::from(hi - lo > 60);
                }
            }
        }
        if ink >= 4 && colour * 10 >= ink * 3 {
            out.push(code);
        }
    }
    out
}

/// A mod's `[Glyphs]` table (Refit's `ui.ini` form): the codes its fonts draw as
/// pictures, each with the word or figure to write where a font cannot draw it
/// (`85 = Dilithium`). Read from the mounted `ui.ini`, else `info.ini`.
#[derive(Resource, Default, Clone, Debug, PartialEq, Eq)]
pub struct GlyphWords(pub BTreeMap<u8, String>);

impl GlyphWords {
    pub fn parse(text: &str) -> GlyphWords {
        let ini = sta_vfs::mods::Ini::parse(text);
        GlyphWords(
            ini.section("glyphs")
                .iter()
                .filter_map(|(k, v)| Some((u8::from_str_radix(k.trim(), 16).ok()?, v.clone())))
                .collect(),
        )
    }

    /// `text` with every mapped code written as its word. A word is set apart by a
    /// space from letters or figures beside it.
    pub fn display(&self, text: &str) -> String {
        if self.0.is_empty() {
            return text.to_owned();
        }
        let mut out = String::new();
        let mut pending_space = false;
        for c in text.chars() {
            let code = windows_1252(c);
            let word = self.0.get(&code);
            match word {
                Some(w) => {
                    let alpha = w.chars().next().is_some_and(char::is_alphabetic);
                    if alpha && out.chars().last().is_some_and(char::is_alphanumeric) {
                        out.push(' ');
                    }
                    out.push_str(w);
                    pending_space = alpha;
                }
                None => {
                    if pending_space && !c.is_whitespace() {
                        out.push(' ');
                    }
                    pending_space = false;
                    out.push(c);
                }
            }
        }
        out
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
        .map_or_else(
            // The games' own picture codes arrive as the control characters of the same number.
            || if (0x80..0xa0).contains(&u) { u as u8 } else { b'?' },
            |i| 0x80 + i as u8,
        )
}

/// One loaded font: its table and, per page, the texture and its pixel size.
pub struct LoadedFont {
    pub font: SpriteFont,
    pub pages: Vec<(Handle<Image>, UVec2)>,
    /// Codes drawn as coloured pictures, untinted (see [`coloured_glyphs`]).
    pub coloured: BTreeSet<u8>,
}

impl LoadedFont {
    pub fn new(font: SpriteFont, pages: Vec<(Handle<Image>, UVec2)>) -> Self {
        LoadedFont { font, pages, coloured: BTreeSet::new() }
    }
}

/// Every sprite font of the mounted install, by file stem. Absent until a mod is
/// mounted and at least one font and its textures were read.
#[derive(Resource, Default)]
pub struct SpriteFonts {
    pub fonts: BTreeMap<String, LoadedFont>,
}

impl SpriteFonts {
    /// The font file `name` (a stem, any case), if the mounted install has it and it fits.
    pub fn named(&self, name: &str) -> Option<&LoadedFont> {
        self.fonts.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, f)| f)
    }

    /// The font of a role; the nearest by height to `size` when the mod has no (usable)
    /// file of that name (Fleet Ops 4.0's FontMedium and up do not fit their sheets).
    pub fn for_role(&self, role: Option<FontRole>, size: f32) -> Option<&LoadedFont> {
        role.and_then(|r| self.named(r.stem())).or_else(|| self.pick(size))
    }

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
        let coloured: BTreeSet<u8> = decoded
            .iter()
            .enumerate()
            .flat_map(|(i, (px, size))| coloured_glyphs(&font, i, *size, px))
            .collect();
        let pages = decoded
            .into_iter()
            .map(|(px, size)| upload_page(px, size, &mut images))
            .collect();
        let mut loaded = LoadedFont::new(font, pages);
        loaded.coloured = coloured;
        fonts.fonts.insert(loaded.font.name.clone(), loaded);
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

/// Opt in to scalable Bevy text even when the mounted install has sprite fonts.
/// Useful for authored UI slots whose bitmap font would shrink below readable ink size.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct ScalableText;

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

/// How a [`SpriteText`] is set, beyond its text, size and colour. Optional: without it
/// text is left aligned, unwrapped, in the font nearest `size`, as before.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct SpriteTextStyle {
    /// The game's named font; the nearest by height when the mod lacks it.
    pub role: Option<FontRole>,
    pub align: Align,
    /// Wrap to this width (px of the node's own space) and align inside it.
    pub wrap: Option<f32>,
    /// Draw the font at its own glyph height instead of scaling it to `size`.
    pub native: bool,
    /// Tint coloured glyphs (resource symbols) like the rest; they are drawn untinted
    /// by default.
    pub tint_all: bool,
    /// Wrap to the parent's width once the text is wider than it (a notice line in a
    /// box with a maximum width). Kept up to date by `fit_wrap` from the last layout.
    pub wrap_parent: bool,
}

#[allow(dead_code)]
impl SpriteTextStyle {
    pub fn role(role: FontRole) -> Self {
        SpriteTextStyle { role: Some(role), ..default() }
    }
    pub fn centered(mut self) -> Self {
        self.align = Align::Center;
        self
    }
    pub fn right(mut self) -> Self {
        self.align = Align::Right;
        self
    }
    pub fn wrap_to_parent(mut self) -> Self {
        self.wrap_parent = true;
        self
    }
    pub fn wrapped(mut self, width: f32) -> Self {
        self.wrap = Some(width);
        self
    }
}

/// What a [`SpriteText`] was last laid out as: a change of colour alone (a fade) then
/// recolours the glyphs instead of laying the text out again.
#[derive(Component, Clone, Debug)]
pub struct SpriteLaid {
    text: String,
    size: f32,
    /// Per glyph quad: drawn untinted.
    untinted: Vec<bool>,
}

/// A [`sprite_text`] bundle with a style (and the `Node` a UI entity needs).
pub fn sprite_text_styled(
    text: impl Into<String>,
    size: f32,
    color: Color,
    style: SpriteTextStyle,
) -> impl Bundle {
    (sprite_text(text, size, color), style, Node::default())
}

/// Load the mod's `[Glyphs]` table (see [`GlyphWords`]) when the install changes.
fn load_glyph_words(vfs: Option<Res<Mounted>>, mut words: ResMut<GlyphWords>) {
    let Some(vfs) = vfs else { return };
    if !vfs.is_changed() {
        return;
    }
    let found = ["ui.ini", "info.ini"].into_iter().find_map(|name| {
        let bytes = vfs.0.read(name)?.ok()?;
        let table = GlyphWords::parse(&String::from_utf8_lossy(&bytes));
        (!table.0.is_empty()).then_some(table)
    });
    // Roots' fonts draw its resource icons and unit classes as pictures: without a table
    // from the mod, the TTF shows the words Refit's fleetops set names for them.
    let builtin = || {
        (sta_content::Profile::detect(&vfs.0) == sta_content::Profile::Fo4)
            .then(|| GlyphWords::parse(include_str!("../../assets/ui/glyphs-fleetops.ini")))
    };
    *words = found.or_else(builtin).unwrap_or_default();
}

#[allow(clippy::type_complexity)]
fn resolve_text(
    mut commands: Commands,
    fonts: Res<SpriteFonts>,
    words: Option<Res<GlyphWords>>,
    q: Query<(
        Entity,
        Ref<SpriteText>,
        Option<Ref<SpriteTextStyle>>,
        Option<Ref<ScalableText>>,
        Option<&SpriteLaid>,
    )>,
    children: Query<&Children>,
    mut images: Query<&mut ImageNode>,
    mut texts: Query<&mut TextColor, With<Text>>,
) {
    let all = fonts.is_changed() || words.as_ref().is_some_and(|w| w.is_changed());
    for (e, t, style, scalable, laid) in &q {
        let dirty = all
            || t.is_changed()
            || style.as_ref().is_some_and(|s| s.is_changed())
            || scalable.as_ref().is_some_and(|s| s.is_added());
        if !dirty {
            continue;
        }
        // Only the colour changed: recolour what is there.
        let only_colour = !all
            && !style.as_ref().is_some_and(|s| s.is_changed())
            && !scalable.as_ref().is_some_and(|s| s.is_added())
            && laid.is_some_and(|l| l.text == t.text && l.size == t.size);
        if only_colour {
            let laid = laid.unwrap();
            for (i, kid) in children.get(e).into_iter().flat_map(|c| c.iter()).enumerate() {
                if let Ok(mut img) = images.get_mut(kid) {
                    let want = if laid.untinted.get(i).copied().unwrap_or(false) { Color::WHITE } else { t.color };
                    if img.color != want {
                        img.color = want;
                    }
                } else if let Ok(mut c) = texts.get_mut(kid)
                    && c.0 != t.color
                {
                    c.0 = t.color;
                }
            }
            continue;
        }
        // Plain `sprite_text` (the HUD's) keeps its old drawing: pictures tinted like the
        // rest and no glyph words. A `SpriteTextStyle` opts in to both.
        let styled = style.is_some();
        let style = style.as_deref().cloned().unwrap_or_default();
        let existing: Vec<Entity> = children
            .get(e)
            .map(|children| children.iter().collect())
            .unwrap_or_default();
        let font = if scalable.is_some() {
            None
        } else {
            fonts.for_role(style.role, t.size)
        };
        let Some(f) = font else {
            // The TTF cannot draw the pictures; write the mod's words for them.
            let shown = words.as_ref().filter(|_| styled).map_or_else(|| t.text.clone(), |w| w.display(&t.text));
            let justify = match style.align {
                Align::Left => bevy::text::Justify::Left,
                Align::Center => bevy::text::Justify::Center,
                Align::Right => bevy::text::Justify::Right,
            };
            let label = (
                Text::new(shown),
                TextFont {
                    font_size: bevy::text::FontSize::Px(t.size),
                    ..default()
                },
                TextColor(t.color),
                TextLayout::new(
                    justify,
                    if style.wrap.is_some() {
                        bevy::text::LineBreak::WordBoundary
                    } else {
                        bevy::text::LineBreak::NoWrap
                    },
                ),
                Pickable::IGNORE,
            );
            if let Some(w) = style.wrap {
                // The box is as wide as the text up to `w`; aligned text fills it.
                let node = if style.align == Align::Left {
                    Node { max_width: Val::Px(w), flex_shrink: 0.0, ..default() }
                } else {
                    Node { width: Val::Px(w), flex_shrink: 0.0, ..default() }
                };
                commands.entity(e).insert(node);
            }
            commands.entity(e).insert(SpriteLaid { text: t.text.clone(), size: t.size, untinted: Vec::new() });
            if let Some(&child) = existing.first().filter(|child| texts.contains(**child)) {
                commands.entity(child).insert(label);
                for &stale in &existing[1..] {
                    commands.entity(stale).despawn();
                }
            } else {
                for stale in existing {
                    commands.entity(stale).despawn();
                }
                commands.entity(e).with_child(label);
            }
            continue;
        };
        let scale = if style.native { 1.0 } else { t.size / f.font.height() };
        let lay = f.font.layout_block(&t.text, scale, style.wrap, style.align);
        commands.entity(e).insert(Node {
            width: Val::Px(lay.width.ceil()),
            height: Val::Px(lay.height.ceil()),
            flex_shrink: 0.0,
            ..default()
        });
        let mut used = 0;
        let mut untinted = Vec::new();
        for q in &lay.quads {
            let Some((image, tex)) = f.pages.get(q.page) else {
                continue;
            };
            let rect = texel_rect(f.font.reference, *tex, q.src);
            let picture = styled && !style.tint_all && f.coloured.contains(&q.code);
            untinted.push(picture);
            let colour = if picture { Color::WHITE } else { t.color };
            let glyph = (
                ImageNode {
                    image: image.clone(),
                    rect: Some(rect),
                    color: colour,
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
            );
            if let Some(&child) = existing.get(used).filter(|child| images.contains(**child)) {
                commands.entity(child).insert(glyph);
            } else {
                if let Some(&stale) = existing.get(used) {
                    commands.entity(stale).despawn();
                }
                commands.entity(e).with_child(glyph);
            }
            used += 1;
        }
        for &stale in existing.iter().skip(used) {
            commands.entity(stale).despawn();
        }
        commands.entity(e).insert(SpriteLaid { text: t.text.clone(), size: t.size, untinted });
    }
}

/// `wrap_parent` text wraps to its parent's content width once it is wider (from the
/// last frame's layout); a changed text starts unwrapped again.
fn fit_wrap(
    mut q: Query<(&ChildOf, &ComputedNode, &mut SpriteTextStyle, Ref<SpriteText>)>,
    parents: Query<&ComputedNode>,
) {
    for (parent, node, mut style, text) in &mut q {
        if !style.wrap_parent {
            continue;
        }
        let Ok(p) = parents.get(parent.parent()) else { continue };
        let k = p.inverse_scale_factor();
        let (pad, border) = (p.padding(), p.border());
        let avail = p.size().x * k - (pad.min_inset.x + pad.max_inset.x + border.min_inset.x + border.max_inset.x) * k;
        let own = node.size().x * node.inverse_scale_factor();
        let want = if avail > 1.0 && own > avail + 0.5 {
            Some(avail.floor())
        } else if text.is_changed() {
            None
        } else {
            style.wrap
        };
        if style.wrap != want {
            style.wrap = want;
        }
    }
}

pub struct SpriteFontPlugin;

impl Plugin for SpriteFontPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpriteFonts>()
            .init_resource::<GlyphWords>()
            .add_systems(Update, fit_wrap)
            .add_systems(
                PostUpdate,
                (load_fonts, load_glyph_words, resolve_text)
                    .chain()
                    .before(bevy::ui::UiSystems::Prepare)
                    .before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
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
                LoadedFont::new(f, vec![]),
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
            LoadedFont::new(
                f,
                vec![
                    (Handle::default(), UVec2::new(256, 128)),
                    (Handle::default(), UVec2::new(256, 128)),
                ],
            ),
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

    fn test_font() -> LoadedFont {
        LoadedFont::new(
            SpriteFont::parse("F", SAMPLE).unwrap(),
            vec![
                (Handle::default(), UVec2::new(256, 128)),
                (Handle::default(), UVec2::new(256, 128)),
            ],
        )
    }

    #[test]
    fn changing_sprite_text_reuses_glyphs_and_removes_only_surplus() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<SpriteFonts>()
            .add_systems(Update, resolve_text);
        app.world_mut()
            .resource_mut::<SpriteFonts>()
            .fonts
            .insert("F".into(), test_font());
        let label = app
            .world_mut()
            .spawn(sprite_text(" !", 12.0, Color::WHITE))
            .id();
        app.update();
        let original: Vec<Entity> = app
            .world()
            .get::<Children>(label)
            .unwrap()
            .iter()
            .collect();

        app.world_mut().get_mut::<SpriteText>(label).unwrap().text = "!!".into();
        app.update();
        let same: Vec<Entity> = app
            .world()
            .get::<Children>(label)
            .unwrap()
            .iter()
            .collect();
        assert_eq!(same, original);
        assert_eq!(app.world().get::<Node>(same[1]).unwrap().left, Val::Px(8.5));
        assert_eq!(
            app.world().get::<ImageNode>(same[0]).unwrap().rect,
            Some(Rect::new(10.0, 0.0, 18.5, 12.0))
        );

        app.world_mut().get_mut::<SpriteText>(label).unwrap().text = "!!#".into();
        app.update();
        let longer: Vec<Entity> = app
            .world()
            .get::<Children>(label)
            .unwrap()
            .iter()
            .collect();
        assert_eq!(&longer[..2], &original);
        assert_eq!(longer.len(), 3);
        assert!(app.world().get::<Pickable>(longer[2]).is_some());

        app.world_mut().get_mut::<SpriteText>(label).unwrap().text = "!".into();
        app.update();
        let shorter: Vec<Entity> = app
            .world()
            .get::<Children>(label)
            .unwrap()
            .iter()
            .collect();
        assert_eq!(shorter, vec![original[0]]);
        assert!(app.world().get_entity(original[1]).is_err());
        assert!(app.world().get_entity(longer[2]).is_err());
    }

    #[test]
    fn fallback_text_keeps_its_child_for_number_updates() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<SpriteFonts>()
            .add_systems(Update, resolve_text);
        let label = app
            .world_mut()
            .spawn(sprite_text("123", 12.0, Color::WHITE))
            .id();
        app.update();
        let child = app.world().get::<Children>(label).unwrap()[0];
        app.world_mut().get_mut::<SpriteText>(label).unwrap().text = "99".into();
        app.update();
        assert_eq!(app.world().get::<Children>(label).unwrap().len(), 1);
        assert_eq!(app.world().get::<Children>(label).unwrap()[0], child);
        assert_eq!(app.world().get::<Text>(child).unwrap().0, "99");
    }

    #[test]
    fn scalable_text_uses_ttf_with_valid_sprite_fonts_and_reuses_number_child() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<SpriteFonts>()
            .add_systems(Update, resolve_text);
        app.world_mut()
            .resource_mut::<SpriteFonts>()
            .fonts
            .insert("F".into(), test_font());
        let label = app
            .world_mut()
            .spawn(sprite_text("!!", 18.0, Color::WHITE))
            .id();
        app.update();
        assert_eq!(app.world().get::<Children>(label).unwrap().len(), 2);
        assert!(
            app.world()
                .get::<Children>(label)
                .unwrap()
                .iter()
                .all(|e| app.world().get::<ImageNode>(e).is_some())
        );

        // A late opt-in must resolve without changing SpriteText itself.
        app.world_mut().entity_mut(label).insert(ScalableText);
        app.update();
        let child = app.world().get::<Children>(label).unwrap()[0];
        assert_eq!(app.world().get::<Children>(label).unwrap().len(), 1);
        assert!(app.world().get::<ImageNode>(child).is_none());
        assert_eq!(app.world().get::<Text>(child).unwrap().0, "!!");
        assert_eq!(
            app.world().get::<TextFont>(child).unwrap().font_size,
            bevy::text::FontSize::Px(18.0)
        );

        app.world_mut().get_mut::<SpriteText>(label).unwrap().text = "99".into();
        app.update();
        assert_eq!(app.world().get::<Children>(label).unwrap()[0], child);
        assert_eq!(app.world().get::<Text>(child).unwrap().0, "99");
    }

    #[test]
    fn new_glyphs_inherit_visibility_in_their_first_frame() {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>()
            .add_plugins((MinimalPlugins, bevy::camera::visibility::VisibilityPlugin, SpriteFontPlugin));
        app.world_mut()
            .resource_mut::<SpriteFonts>()
            .fonts
            .insert("F".into(), test_font());
        let root = app.world_mut().spawn(Visibility::Visible).id();
        let label = app
            .world_mut()
            .spawn(sprite_text("!", 12.0, Color::WHITE))
            .id();
        app.world_mut().entity_mut(root).add_child(label);
        app.update();
        let first = app.world().get::<Children>(label).unwrap()[0];
        assert!(app.world().get::<InheritedVisibility>(first).unwrap().get());

        *app.world_mut().get_mut::<Visibility>(root).unwrap() = Visibility::Hidden;
        app.world_mut().get_mut::<SpriteText>(label).unwrap().text = "!!".into();
        app.update();
        let appended = app.world().get::<Children>(label).unwrap()[1];
        assert!(
            !app.world()
                .get::<InheritedVisibility>(appended)
                .unwrap()
                .get()
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

    /// A sprite font whose glyph widths are the baked TTF's advances at `h` px: what the
    /// two layouts are compared on.
    fn ttf_like(h: f32) -> SpriteFont {
        let ttf = crate::ui::theme::font::SdfFont::regular();
        let glyphs = (0x20u8..=0x7e)
            .map(|c| GlyphEntry {
                x: 0.0,
                y: 0.0,
                w: (ttf.glyph(c as char).unwrap().advance * h).round(),
            })
            .collect();
        SpriteFont {
            name: "T".into(),
            reference: None,
            pages: vec![Page { texture: "t".into(), height: h, glyphs }],
        }
    }

    const PARAGRAPH: &str = "Shields are down on the Defiant and the Cardassian fleet is closing fast, so hold the line until reinforcements arrive";

    #[test]
    fn wrapped_text_matches_the_ttf_layout_within_a_tolerance() {
        let ttf = crate::ui::theme::font::SdfFont::regular();
        let (h, w) = (16.0, 220.0);
        let font = ttf_like(h);
        let sprite = wrap_lines(PARAGRAPH, w, &|s| font.measure(s, 1.0));
        let baked = wrap_lines(PARAGRAPH, w, &|s| ttf.measure(s, h));
        // Same rule, widths within half a pixel a glyph: the breaks agree or differ by one word.
        assert!(sprite.len().abs_diff(baked.len()) <= 1, "{sprite:?} vs {baked:?}");
        for line in &sprite {
            assert!(font.measure(line, 1.0) <= w, "{line:?} overflows");
        }
        // Nothing is lost or reordered.
        assert_eq!(sprite.join(" "), PARAGRAPH);
        // Block bounds: width is the widest line, height a glyph height per line, each
        // within a tolerance of the TTF block.
        let block = font.layout_block(PARAGRAPH, 1.0, Some(w), Align::Left);
        let widest = baked.iter().map(|l| ttf.measure(l, h)).fold(0.0, f32::max);
        let longest = baked.iter().map(|l| l.chars().count()).max().unwrap() as f32;
        assert!((block.width - widest).abs() <= 0.5 * longest, "{} vs {widest}", block.width);
        assert_eq!(block.height, h * sprite.len() as f32);
    }

    #[test]
    fn centred_and_right_text_sit_where_the_ttf_puts_it() {
        let ttf = crate::ui::theme::font::SdfFont::regular();
        let (h, w) = (16.0, 300.0);
        let font = ttf_like(h);
        let text = "Federation";
        let at = |align| {
            let b = font.layout_block(text, 1.0, Some(w), align);
            (b.width, b.quads[0].rect[0])
        };
        let ink = ttf.measure(text, h);
        let (bw, centre) = at(Align::Center);
        assert_eq!(bw, w);
        // The TTF centres the same line at (w - ink) / 2; the sprite font's own widths differ
        // by rounding only.
        assert!((centre - (w - ink) / 2.0).abs() <= text.len() as f32 * 0.5);
        let (_, right) = at(Align::Right);
        assert!((right - (w - ink)).abs() <= text.len() as f32 * 0.5);
        assert_eq!(at(Align::Left).1, 0.0);
        // Left text takes the width it needs, not the box's.
        assert!(font.layout_block(text, 1.0, Some(w), Align::Left).width < w);
    }

    #[test]
    fn long_words_break_between_characters_and_newlines_hold() {
        let font = ttf_like(16.0);
        let lines = wrap_lines("Supercalifragilisticexpialidocious", 80.0, &|s| font.measure(s, 1.0));
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|l| font.measure(l, 1.0) <= 80.0 || l.chars().count() == 1));
        assert_eq!(lines.concat(), "Supercalifragilisticexpialidocious");
        assert_eq!(wrap_lines("a\nb", 500.0, &|s| font.measure(s, 1.0)), ["a", "b"]);
        assert_eq!(wrap_lines("", 500.0, &|s| font.measure(s, 1.0)), [""]);
    }

    #[test]
    fn named_fonts_win_over_the_nearest_height() {
        let mut fonts = SpriteFonts::default();
        for (n, h) in [("fontsmall", 14.0), ("fontmedium", 23.0), ("fontlarge", 30.0)] {
            let mut f = SpriteFont::parse(n, SAMPLE).unwrap();
            f.pages[0].height = h;
            fonts.fonts.insert(n.into(), LoadedFont::new(f, vec![]));
        }
        // Medium is asked for at a size nearer FontLarge: the name decides.
        let pick = |r, size| fonts.for_role(r, size).unwrap().font.name.clone();
        assert_eq!(pick(Some(FontRole::Medium), 29.0), "fontmedium");
        assert_eq!(pick(Some(FontRole::Small), 30.0), "fontsmall");
        // A role the mod has no font for falls back to the nearest height; no role, as before.
        assert_eq!(pick(Some(FontRole::VeryLarge), 29.0), "fontlarge");
        assert_eq!(pick(None, 29.0), "fontlarge");
        assert_eq!(FontRole::from_name("FontMedium"), Some(FontRole::Medium));
        assert_eq!(FontRole::from_name("FontFinal4_16"), None);
        assert!(fonts.named("FONTSMALL").is_some());
    }

    #[test]
    fn coloured_glyphs_are_found_from_the_pixels() {
        let mut f = SpriteFont::parse("F", SAMPLE).unwrap();
        f.pages.truncate(1);
        f.pages[0].glyphs = (0..8)
            .map(|n| GlyphEntry { x: 12.0 * n as f32, y: 0.0, w: 10.0 })
            .collect();
        f.pages[0].height = 10.0;
        f.reference = Some([128.0, 16.0]);
        let tex = UVec2::new(128, 16);
        let mut px = vec![0u8; 128 * 16 * 4];
        let fill = |px: &mut Vec<u8>, n: usize, rgba: [u8; 4]| {
            for y in 0..10 {
                for x in 0..10 {
                    let i = (y * 128 + 12 * n + x) * 4;
                    px[i..i + 4].copy_from_slice(&rgba);
                }
            }
        };
        fill(&mut px, 0, [255, 255, 255, 255]); // 0x20: white, a letter's kind
        fill(&mut px, 1, [220, 30, 30, 255]); // 0x21: red
        fill(&mut px, 2, [200, 200, 200, 255]); // 0x22: grey
        fill(&mut px, 3, [30, 200, 60, 255]); // 0x23: green
        fill(&mut px, 4, [255, 0, 0, 0]); // 0x24: transparent
        assert_eq!(coloured_glyphs(&f, 0, tex, &px), vec![0x21, 0x23]);
    }

    #[test]
    fn glyph_words_follow_the_mods_table() {
        let words = GlyphWords::parse("[Other]\n85 = Nope\n[Glyphs]\n; red figures\n7F = 0\n81 = 1\n85 = Dilithium\n87 = Supply\n80 = s\n24 = \" \"\n");
        assert_eq!(words.0.get(&0x85).map(String::as_str), Some("Dilithium"));
        let c = |code: u8| {
            // The game's byte as the char the text carries.
            char::from(code)
        };
        // The red figures are digits; a resource icon is its word, set apart by spaces.
        let text = format!("{}{}{}{}250{}", c(0x7f), c(0x81), c(0x7f), c(0x85), c(0x87));
        assert_eq!(words.display(&text), "010 Dilithium 250 Supply");
        // Plain text and an empty table pass through.
        assert_eq!(words.display("Hull 100%"), "Hull 100%");
        assert_eq!(GlyphWords::default().display(&text), text);
        // `$` is the cost lines' spacer.
        assert_eq!(words.display("250$125"), "250 125");
    }

    /// Resolves styled text: the named font, untinted coloured glyphs, centring in a box.
    #[test]
    fn styled_sprite_text_uses_the_role_font_and_leaves_pictures_untinted() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).init_resource::<SpriteFonts>().init_resource::<GlyphWords>().add_systems(Update, resolve_text);
        let mut big = test_font();
        big.font.name = "fontlarge".into();
        big.font.pages[0].height = 24.0;
        big.coloured.insert(b'!');
        app.world_mut().resource_mut::<SpriteFonts>().fonts.insert("fontsmall".into(), test_font());
        app.world_mut().resource_mut::<SpriteFonts>().fonts.insert("fontlarge".into(), big);
        // Size 12 is nearest FontSmall's 12; the role asks for FontLarge.
        let e = app
            .world_mut()
            .spawn(sprite_text_styled(" !", 24.0, Color::srgb(0.0, 1.0, 0.0), SpriteTextStyle::role(FontRole::Large)))
            .id();
        app.update();
        let kids: Vec<Entity> = app.world().get::<Children>(e).unwrap().iter().collect();
        assert_eq!(kids.len(), 2);
        let colour = |i: usize| app.world().get::<ImageNode>(kids[i]).unwrap().color;
        assert_eq!(colour(0), Color::srgb(0.0, 1.0, 0.0), "text is tinted");
        assert_eq!(colour(1), Color::WHITE, "a coloured picture is not");
        // Opting out tints everything.
        app.world_mut().entity_mut(e).insert(SpriteTextStyle { tint_all: true, ..SpriteTextStyle::role(FontRole::Large) });
        app.update();
        let kids: Vec<Entity> = app.world().get::<Children>(e).unwrap().iter().collect();
        assert_eq!(app.world().get::<ImageNode>(kids[1]).unwrap().color, Color::srgb(0.0, 1.0, 0.0));

        // Centred in a box wider than the text.
        let c = app
            .world_mut()
            .spawn(sprite_text_styled(" ", 12.0, Color::WHITE, SpriteTextStyle::role(FontRole::Small).centered().wrapped(100.0)))
            .id();
        app.update();
        let kid = app.world().get::<Children>(c).unwrap()[0];
        assert_eq!(app.world().get::<Node>(c).unwrap().width, Val::Px(100.0));
        assert_eq!(app.world().get::<Node>(kid).unwrap().left, Val::Px(47.5), "(100 - 5) / 2");
    }

    /// A label built before the install is mounted picks up the mod's font once it loads.
    #[test]
    fn text_resolved_without_fonts_is_redone_when_fonts_arrive() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).init_resource::<SpriteFonts>().init_resource::<GlyphWords>().add_systems(Update, resolve_text);
        let e = app.world_mut().spawn(sprite_text(" ", 12.0, Color::WHITE)).id();
        app.update();
        let first = app.world().get::<Children>(e).unwrap()[0];
        assert!(app.world().get::<Text>(first).is_some());
        app.world_mut().resource_mut::<SpriteFonts>().fonts.insert("F".into(), test_font());
        app.update();
        let kids = app.world().get::<Children>(e).unwrap();
        assert!(app.world().get::<ImageNode>(kids[0]).is_some());
    }

    /// The TTF cannot draw the games' picture codes: the mod's words stand in.
    #[test]
    fn ttf_fallback_writes_the_glyph_words() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).init_resource::<SpriteFonts>().insert_resource(GlyphWords::parse("[Glyphs]\n85 = Dilithium\n")).add_systems(Update, resolve_text);
        let e = app.world_mut().spawn(sprite_text_styled(format!("50{}", char::from(0x85u8)), 12.0, Color::WHITE, SpriteTextStyle::default())).id();
        app.update();
        let kid = app.world().get::<Children>(e).unwrap()[0];
        assert_eq!(app.world().get::<Text>(kid).unwrap().0, "50 Dilithium");
    }

    /// Install-gated: the mods' fonts, loaded as the client loads them, have letters in
    /// white and whatever pictures they carry flagged coloured.
    #[test]
    fn installed_fonts_keep_letters_untinted_free() {
        for (label, root, m) in installs() {
            let Ok(vfs) = Vfs::mount_install(&root, m.as_deref()) else {
                eprintln!("{label}: skipped");
                continue;
            };
            for font in read_fonts(&vfs) {
                let Some(decoded): Option<Vec<_>> = font.pages.iter().map(|p| decode_page(&vfs, &p.texture)).collect() else {
                    continue;
                };
                let mut codes = Vec::new();
                for (i, (px, size)) in decoded.iter().enumerate() {
                    if page_fit(&font, i, *size, px).is_some_and(|f| f < MIN_FIT) {
                        continue;
                    }
                    codes.extend(coloured_glyphs(&font, i, *size, px));
                }
                eprintln!("{label}: {} coloured codes {:02x?}", font.name, codes);
                assert!(
                    codes.iter().all(|c| !c.is_ascii_alphabetic()),
                    "{label}: {} flags a letter as a coloured picture",
                    font.name
                );
            }
        }
    }

    /// A fade changes only the colour: the glyph nodes are reused and recoloured.
    #[test]
    fn a_colour_change_recolours_without_laying_out_again() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).init_resource::<SpriteFonts>().init_resource::<GlyphWords>().add_systems(Update, resolve_text);
        app.world_mut().resource_mut::<SpriteFonts>().fonts.insert("F".into(), test_font());
        let e = app.world_mut().spawn(sprite_text(" !", 12.0, Color::WHITE)).id();
        app.update();
        let before: Vec<Entity> = app.world().get::<Children>(e).unwrap().iter().collect();
        app.world_mut().get_mut::<SpriteText>(e).unwrap().color = Color::srgb(1.0, 0.0, 0.0);
        app.update();
        let after: Vec<Entity> = app.world().get::<Children>(e).unwrap().iter().collect();
        assert_eq!(before, after);
        assert_eq!(app.world().get::<ImageNode>(after[1]).unwrap().color, Color::srgb(1.0, 0.0, 0.0));
        // A changed text still lays out.
        app.world_mut().get_mut::<SpriteText>(e).unwrap().text = " ".into();
        app.update();
        assert_eq!(app.world().get::<Children>(e).unwrap().len(), 1);
    }

    #[test]
    fn the_builtin_fleetops_glyph_table_parses() {
        let t = GlyphWords::parse(include_str!("../../assets/ui/glyphs-fleetops.ini"));
        assert_eq!(t.0.get(&0x85).map(String::as_str), Some("Dilithium"));
        assert_eq!(t.0.get(&0x7f).map(String::as_str), Some("0"));
        assert_eq!(t.0.get(&0x24).map(String::as_str), Some(" "));
        assert_eq!(t.0.len(), 32);
    }
}
