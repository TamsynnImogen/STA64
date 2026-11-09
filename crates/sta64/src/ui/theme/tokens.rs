//! Theme tokens: the LCARS look of Refit's `ui/STYLE.md` (`Form::Lcars` in its
//! `widgets/theme.rs`), ported with Refit's MIT notice (see `assets/ui/NOTICE.md`).
//! Colour roles keep Refit's names; sizes are Refit's units (pixels of a
//! 1080-line screen). See `STYLE.md` next to this file for the table.

use bevy::prelude::*;

/// Every colour, size and gap the widgets use. Insert a modified copy to
/// re-skin; widgets read it when they are built and when their state changes.
#[derive(Resource, Debug, Clone)]
pub struct Theme {
    // Colour roles.
    /// Panel body and the clear colour (LCARS panels are black).
    pub panel: Color,
    /// Pale lavender: the bottom elbow.
    pub panel_hi: Color,
    pub text: Color,
    pub text_dim: Color,
    /// Disabled text.
    pub text_off: Color,
    /// Text on a bright fill (every LCARS pill).
    pub text_on: Color,
    pub title: Color,
    /// Gold: selection, fills, the "on" state.
    pub accent: Color,
    /// Lavender: panel frames and buttons.
    pub accent2: Color,
    /// Sky blue: the title bar's cap.
    pub accent3: Color,
    pub button: Color,
    pub button_hot: Color,
    pub button_down: Color,
    pub button_on: Color,
    pub button_off: Color,
    /// The empty part of sliders and bars.
    pub track: Color,
    pub fill: Color,
    pub focus: Color,
    pub row_hot: Color,
    pub row_sel: Color,
    pub row_alt: Color,
    /// Status colours, shared by every theme.
    pub good: Color,
    pub warn: Color,
    pub bad: Color,
    // Type (units).
    pub display_size: f32,
    pub title_size: f32,
    pub heading_size: f32,
    pub text_size: f32,
    pub small_size: f32,
    pub micro_size: f32,
    /// Titles and (LCARS) buttons are set in capitals.
    pub caps_titles: bool,
    pub caps_buttons: bool,
    // Space (a 4-unit grid).
    pub gap: f32,
    pub pad: f32,
    pub row_h: f32,
    pub button_h: f32,
    pub list_row_h: f32,
    // Shape: the panel's elbows, the bars and the slider track.
    /// Outer radius of a panel's elbows.
    pub corners: f32,
    /// Radius of the elbow's inside corner.
    pub elbow_inner: f32,
    /// Width of the side bar (the elbow's vertical arm).
    pub side_bar: f32,
    /// Height of the top bar without a title; with one it is `title_size + 6`.
    pub top_bar: f32,
    pub bottom_bar: f32,
    /// Length of an elbow's horizontal arm past the side bar.
    pub elbow_arm: f32,
    /// Gap between the pieces of a frame.
    pub bar_gap: f32,
    /// Width of the title bar's sky cap.
    pub cap_w: f32,
    /// Slider and progress bar track height.
    pub track_h: f32,
    /// Hover blend `0..=1` of a toggle that is on: `mix(on, hot, hot * 0.5)`.
    pub on_hover_mix: f32,
}

impl Default for Theme {
    fn default() -> Self {
        // Refit writes 0xAARRGGBB; `argb` reads the same.
        let c = |v: u32| Color::srgba_u8((v >> 16) as u8, (v >> 8) as u8, v as u8, (v >> 24) as u8);
        Theme {
            panel: c(0xff000000),
            panel_hi: c(0xffc5cef1),
            text: c(0xffdcdcff),
            text_dim: c(0xff9a9ac8),
            text_off: c(0xff55556e),
            text_on: c(0xff000000),
            title: c(0xfff4b84c),
            accent: c(0xfff4b84c),
            accent2: c(0xff9c9cde),
            accent3: c(0xff7ea8d4),
            button: c(0xff9c9cde),
            button_hot: c(0xffc0c0ff),
            button_down: c(0xffeeeeff),
            button_on: c(0xfff4b84c),
            button_off: c(0xff2e2e44),
            track: c(0xff20203a),
            fill: c(0xfff4b84c),
            focus: c(0xffffe2a8),
            row_hot: c(0x349c9cde),
            row_sel: c(0x58f4b84c),
            row_alt: c(0x10ffffff),
            good: c(0xff5fd36a),
            warn: c(0xfff2c23b),
            bad: c(0xffe5483b),
            display_size: 44.0,
            title_size: 26.0,
            heading_size: 22.0,
            text_size: 20.0,
            small_size: 17.0,
            micro_size: 14.0,
            caps_titles: true,
            caps_buttons: true,
            gap: 8.0,
            pad: 12.0,
            row_h: 32.0,
            button_h: 40.0,
            list_row_h: 26.0,
            corners: 30.0,
            elbow_inner: 12.0,
            side_bar: 18.0,
            top_bar: 12.0,
            bottom_bar: 10.0,
            elbow_arm: 72.0,
            bar_gap: 5.0,
            cap_w: 30.0,
            track_h: 6.0,
            on_hover_mix: 0.5,
        }
    }
}

impl Theme {
    /// The text as a button or title sets it: capitals where the theme says so.
    pub fn caps(&self, s: &str, on: bool) -> String {
        if on { s.to_uppercase() } else { s.to_owned() }
    }
}

/// Linear blend of two colours in sRGB, `t` of the way from `a` to `b`
/// (Refit's `mix`).
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    let (a, b) = (a.to_srgba(), b.to_srgba());
    let l = |x: f32, y: f32| x + (y - x) * t;
    Color::srgba(l(a.red, b.red), l(a.green, b.green), l(a.blue, b.blue), l(a.alpha, b.alpha))
}

/// Scale a colour's brightness, keeping alpha.
pub fn scale(c: Color, k: f32) -> Color {
    let s = c.to_srgba();
    Color::srgba((s.red * k).min(1.0), (s.green * k).min(1.0), (s.blue * k).min(1.0), s.alpha)
}
