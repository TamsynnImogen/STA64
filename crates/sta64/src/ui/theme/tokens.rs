//! Theme tokens: palette, sizes, spacing. See `STYLE.md` for the table.

use bevy::prelude::*;

/// Every colour, size and gap the widgets use. Insert a modified copy to
/// re-skin; widgets read it when they are built and when their state changes.
#[derive(Resource, Debug, Clone)]
pub struct Theme {
    pub background: Color,
    pub orange: Color,
    pub peach: Color,
    pub lilac: Color,
    pub sky: Color,
    pub tan: Color,
    pub red: Color,
    pub text: Color,
    pub text_dark: Color,
    pub disabled: Color,
    /// Multiplier on the base colour while hovered / pressed / switched off.
    pub hover_boost: f32,
    pub press_boost: f32,
    pub off_dim: f32,
    pub bar_height: f32,
    pub rail_width: f32,
    pub elbow_radius: f32,
    pub gap: f32,
    pub pad: f32,
    pub font_size: f32,
    pub title_size: f32,
}

impl Default for Theme {
    fn default() -> Self {
        let c = |r: u8, g: u8, b: u8| Color::srgb_u8(r, g, b);
        Theme {
            background: c(0, 0, 0),
            orange: c(0xFF, 0x99, 0x00),
            peach: c(0xFF, 0xCC, 0x99),
            lilac: c(0xCC, 0x99, 0xCC),
            sky: c(0x99, 0xCC, 0xFF),
            tan: c(0xFF, 0xAA, 0x90),
            red: c(0xCC, 0x66, 0x66),
            text: c(0xFF, 0xCC, 0x99),
            text_dark: c(0, 0, 0),
            disabled: c(0x55, 0x55, 0x66),
            hover_boost: 1.15,
            press_boost: 0.75,
            off_dim: 0.45,
            bar_height: 28.0,
            rail_width: 72.0,
            elbow_radius: 36.0,
            gap: 6.0,
            pad: 10.0,
            font_size: 16.0,
            title_size: 20.0,
        }
    }
}

/// Scale a colour's brightness, keeping alpha.
pub fn scale(c: Color, k: f32) -> Color {
    let s = c.to_srgba();
    Color::srgba((s.red * k).min(1.0), (s.green * k).min(1.0), (s.blue * k).min(1.0), s.alpha)
}
