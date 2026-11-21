//! The UI lane: the LCARS theme and widget kit (UI-03); the HUD (UI-04), menus
//! and the rest build on it.

pub mod presentation;
pub mod bridge_scene;
pub mod objectives_panel;
#[cfg(test)]
mod objectives_panel_gpu;
pub mod profile;
pub mod cursor;
pub mod gui_sprites;
pub mod hud;
pub mod ingame_menu;
pub mod pause_box;
pub mod i18n;
pub mod notices;
pub mod theme;
pub mod radar;
pub mod radar_fog;
pub mod radar_layers;
pub mod settings;
pub mod display;
pub mod graphics;
pub mod sprite_font;
pub mod stock_font;
pub mod stock_tooltip;
