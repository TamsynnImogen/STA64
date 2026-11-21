//! The stock HUD's bitmap-font rule for the Armada II engine family: which font file and
//! glyph scale a text role gets at a given HUD width, and how its glyphs are sampled.
//! Pure functions; `SpriteFonts::stock_choice` applies them to the loaded fonts.
//!
//! The rule (grades per item):
//! - Three roles (default, title, big) with a base size each, and four widths ("tiers").
//!   For each tier a role has the font whose size is `floor(base * tier / 800 + 0.5)`,
//!   when that file exists; those are its loaded sizes, in tier order. Read from the
//!   executable, static (the base sizes and tiers); the font file name prefix is read from data.
//! - At an effective width `w` the target size is `floor(base * w / 800 + 0.5)`; the font is
//!   the largest loaded size not above the target, or the smallest when none is. Read from
//!   the executable, static.
//! - A glyph draws at `(sx, sy) * 1600 / tier[i]` times its texels, `(sx, sy)` being the HUD's
//!   own scale per axis against the 1600x1200 space and `i` the font's index in the loaded
//!   list (not the index among all four tiers: a missing size shifts the later ones down).
//!   Measured on two frames of different aspect; the index rule measured on one frame.
//! - The pen advances by the scaled glyph width rounded up to a whole pixel. Measured.
//! - Glyph texels are sampled nearest only when the chosen size equals the target, linear
//!   otherwise (clamp to edge). Measured in one frame, matching the executable (static).
//! - No drop shadow on HUD readouts. Measured on two frames.
//!
//! Interim, STA64's rule for a case the original does not have: the effective width `w`
//! (see [`effective_width`]) when the HUD is centred on a window wider than its sheet.

use bevy::prelude::*;

use crate::ui::hud::stock::layout::HudFit;

/// The font file stem prefix of the role sets (`<prefix><size>`, lower case). Read from data.
pub const STOCK_FONT_PREFIX: &str = "fontfinal4_";
/// The widths the engine loads one size of each role for. Read from the executable, static.
pub const STOCK_TIERS: [u32; 4] = [800, 1024, 1280, 1600];
/// The HUD's own coordinate space, whatever a sheet's canvas is. Measured.
pub const STOCK_SPACE: Vec2 = Vec2::new(1600.0, 1200.0);

/// A font role of the original's text: which base size it asks for. The caller picks it by
/// what the text is.
// Title and Big have no caller yet (notices and markup are not wired; see the PR).
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StockRole {
    /// Readouts, labels, button labels, tooltips, selection names.
    #[default]
    Default,
    /// Notices, headers, mission titles.
    Title,
    /// Only reached through text markup.
    Big,
}

impl StockRole {
    /// Base size; read from the executable, static.
    pub const fn base(self) -> u32 {
        match self {
            Self::Default => 10,
            Self::Title => 15,
            Self::Big => 24,
        }
    }
}

/// How a font's glyph texels are sampled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GlyphFilter {
    #[default]
    Nearest,
    Linear,
}

/// `floor(base * w / 800 + 0.5)`.
pub fn size_at(base: u32, w: f32) -> u32 {
    (f64::from(base) * f64::from(w.max(0.0)) / 800.0 + 0.5).floor() as u32
}

/// The sizes a role has loaded, with the tier each stands at: its index in this list picks
/// the tier. `has(size)` says whether the font file of that size exists.
pub fn loaded_sizes(role: StockRole, has: &dyn Fn(u32) -> bool) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for tier in STOCK_TIERS {
        let n = size_at(role.base(), tier as f32);
        if n > 0 && has(n) && !out.iter().any(|(s, _)| *s == n) {
            // The tier is that of the index in the loaded list, not of the width it came from.
            out.push((n, STOCK_TIERS[out.len()]));
        }
    }
    out
}

/// What the rule picked for a role at an effective width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StockChoice {
    pub size: u32,
    pub tier: u32,
    pub target: u32,
    pub filter: GlyphFilter,
}

/// The font size, tier and filter for `role` at effective width `w` (physical px).
/// `None` when the role has no loaded size.
pub fn choose(role: StockRole, w: f32, has: &dyn Fn(u32) -> bool) -> Option<StockChoice> {
    let loaded = loaded_sizes(role, has);
    let target = size_at(role.base(), w);
    let (size, tier) = loaded
        .iter()
        .copied()
        .rfind(|(s, _)| *s <= target)
        .or_else(|| loaded.first().copied())?;
    let filter = if size == target { GlyphFilter::Nearest } else { GlyphFilter::Linear };
    Some(StockChoice { size, tier, target, filter })
}

/// The width in physical px the size choice uses. With the Centred fit the HUD canvas's
/// width (STA64's rule for a case the original does not have: the original draws on a
/// window of the sheet's aspect, where canvas and window coincide); with the Stretched fit
/// the window's width, which is the original's rule.
pub fn effective_width(fit: HudFit, canvas_w: f32, window_w: f32) -> f32 {
    match fit {
        HudFit::Centred => canvas_w,
        HudFit::Stretched => window_w,
    }
}

/// The glyph scale per axis for a choice: the HUD canvas's own scale against the 1600x1200
/// space times `1600 / tier`. `canvas` is the HUD rectangle's size in physical px.
pub fn glyph_scale(choice: &StockChoice, canvas: Vec2) -> Vec2 {
    canvas / STOCK_SPACE * (STOCK_SPACE.x / choice.tier as f32)
}

/// The HUD the stock text is drawn in, in logical px: set by the stock HUD while it is
/// shown, absent otherwise (then stock text keeps the plain path).
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq)]
pub struct StockTextCanvas(pub Option<StockCanvas>);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StockCanvas {
    /// The HUD rectangle's size (the sheet's canvas as placed).
    pub hud: Vec2,
    /// The window's size in the same units.
    pub window: Vec2,
    pub fit: HudFit,
}

impl StockCanvas {
    /// Effective width in physical px at `px` physical px per logical px.
    pub fn width(&self, px: f32) -> f32 {
        effective_width(self.fit, self.hud.x * px, self.window.x * px)
    }

    /// The HUD rectangle in physical px.
    pub fn physical(&self, px: f32) -> Vec2 {
        self.hud * px
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An authored set shaped like the original's: sizes for every tier of the base, except
    /// one tier of the title role.
    fn has_all_but(missing: &'static [u32]) -> impl Fn(u32) -> bool {
        move |n| !missing.contains(&n)
    }

    #[test]
    fn sizes_come_from_the_tiers_and_the_missing_one_shifts_the_index() {
        let all = |_: u32| true;
        assert_eq!(loaded_sizes(StockRole::Default, &all), vec![(10, 800), (13, 1024), (16, 1280), (20, 1600)]);
        // The title role's 30 is absent: its 24 stands at the third tier, not the fourth.
        let no30 = has_all_but(&[30]);
        assert_eq!(loaded_sizes(StockRole::Title, &no30), vec![(15, 800), (19, 1024), (24, 1280)]);
        // A missing size in the middle shifts the later ones down.
        let no13 = has_all_but(&[13]);
        assert_eq!(loaded_sizes(StockRole::Default, &no13), vec![(10, 800), (16, 1024), (20, 1280)]);
        let none = |_: u32| false;
        assert!(loaded_sizes(StockRole::Big, &none).is_empty());
        assert_eq!(choose(StockRole::Big, 1600.0, &none), None);
    }

    #[test]
    fn target_is_the_rounded_size_and_choice_the_largest_not_above() {
        let all = |_: u32| true;
        let want = [(800.0, 10, 10), (1024.0, 13, 13), (1280.0, 16, 16), (1600.0, 20, 20), (1920.0, 24, 20), (3440.0, 43, 20), (640.0, 8, 10), (1152.0, 14, 13)];
        for (w, target, size) in want {
            let c = choose(StockRole::Default, w, &all).unwrap();
            assert_eq!((c.target, c.size), (target, size), "width {w}");
        }
    }

    #[test]
    fn filter_is_nearest_only_when_the_size_is_the_target() {
        let all = |_: u32| true;
        let f = |w| choose(StockRole::Default, w, &all).unwrap().filter;
        assert_eq!(f(800.0), GlyphFilter::Nearest);
        assert_eq!(f(1280.0), GlyphFilter::Nearest);
        assert_eq!(f(1600.0), GlyphFilter::Nearest);
        assert_eq!(f(1152.0), GlyphFilter::Linear);
        assert_eq!(f(1920.0), GlyphFilter::Linear);
    }

    /// The measured 1600x1200 frame, as an authored set of the same shape: the default role
    /// draws 1:1 and nearest; the title role, with no size at the top tier, draws its largest
    /// at the third tier's scale and linear.
    #[test]
    fn the_measured_square_frame_with_authored_sets() {
        let canvas = Vec2::new(1600.0, 1200.0);
        let all = |_: u32| true;
        let d = choose(StockRole::Default, 1600.0, &all).unwrap();
        assert_eq!((d.size, d.filter, glyph_scale(&d, canvas)), (20, GlyphFilter::Nearest, Vec2::ONE));
        let no30 = has_all_but(&[30]);
        let t = choose(StockRole::Title, 1600.0, &no30).unwrap();
        assert_eq!((t.size, t.target, t.filter, glyph_scale(&t, canvas)), (24, 30, GlyphFilter::Linear, Vec2::splat(1.25)));
    }

    /// The measured 16:9 frame: stretched over the window, the second axis scales on its own.
    #[test]
    fn the_measured_wide_frame_with_authored_sets() {
        let (w, h) = (1280.0, 720.0);
        let all = |_: u32| true;
        let w_eff = effective_width(HudFit::Stretched, w, w);
        let c = choose(StockRole::Default, w_eff, &all).unwrap();
        assert_eq!((c.size, c.filter), (16, GlyphFilter::Nearest));
        let s = glyph_scale(&c, Vec2::new(w, h));
        assert!((s.x - 1.0).abs() < 1e-6 && (s.y - 0.75).abs() < 1e-6, "{s:?}");
    }

    #[test]
    fn centred_width_is_the_canvas_not_the_window() {
        // 3440x1440 centred on a 4:3 sheet: the canvas is 1920 wide (scale 1.2 over 1600x1200).
        let canvas = Vec2::new(1920.0, 1440.0);
        assert_eq!(effective_width(HudFit::Centred, canvas.x, 3440.0), 1920.0);
        // 1920x1080: the canvas is 1440 wide.
        let canvas = Vec2::new(1440.0, 1080.0);
        assert_eq!(effective_width(HudFit::Centred, canvas.x, 1920.0), 1440.0);
        // Stretched: the window.
        assert_eq!(effective_width(HudFit::Stretched, canvas.x, 1920.0), 1920.0);
        let all = |_: u32| true;
        // The same two windows through the choice: size 24 target at 1920, 18 at 1440.
        assert_eq!(choose(StockRole::Default, 1920.0, &all).unwrap().target, 24);
        assert_eq!(choose(StockRole::Default, 1440.0, &all).unwrap().target, 18);
        let c = StockCanvas { hud: Vec2::new(1600.0, 1200.0), window: Vec2::new(2752.0, 1152.0), fit: HudFit::Centred };
        assert!((c.width(0.9) - 1440.0).abs() < 1e-2);
    }
}
