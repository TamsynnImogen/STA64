//! The Armada I stock HUD (UI-19): the panels of `gui_<race>.cfg` in Armada I's own key family.
//!
//! What is generic (the cfg reader, `StockLayout`, the radar, the resource strip, the sprite
//! tables) is reused from the Armada II stock HUD. What is Armada I's own lives here:
//! - [`control`]: the control panel, a fixed grid of twelve slots (`controlButton1..12`) whose
//!   commands the install's `game_cfg.h` places (`BUTTON_LOCATION_*`, ADR 0004 D9, D11), with the
//!   Orders page behind the Orders button, and the build page.
//! - [`costs`]: the `needs:` line of the build and research tooltips and the named refusal (UI-42).
//! - [`action`]: the cinematic window (UI-53): the COMM and MENU buttons above the action camera's
//!   picture.
//! - [`info`]: the single-selection info panel (`infoSingle*` keys: name, class, shield bar,
//!   construction and energy bars, crew, race icon).
//!
//! The whole 640x480 sheet scales as one piece (`StockLayout`, ADR 0008 H1 to H3, Centred or
//! Stretched); panel groups never separate. Evidence for the keys: read from data (the Armada I
//! install's `gui_<race>.cfg`). Not drawn yet, each a later package (TODO lines in the PR):
//! the speed rail's contents (its empty panel is drawn from the `speed*` frame pieces),
//! multi-selection icons and the system icons (`infoMulti*`, `infoSystem*`), and the dialogs.
//! While a mission clip plays in the cinematic window, `hud::cinematic` (UI-39) draws over it.

pub(in crate::ui::hud) mod action;
pub(super) mod control;
pub(in crate::ui::hud) mod costs;
pub(super) mod info;

use bevy::{picking::Pickable, prelude::*};

use super::layout::StockLayout;
use crate::ui::{gui_sprites::GuiImage, hud::layout::R, profile::UiProfile};

/// The Armada I look: its own panels and command grid.
pub(super) fn is_a1(profile: Option<&UiProfile>) -> bool {
    profile.is_some_and(|p| *p == UiProfile::Armada1)
}

pub(super) fn node(r: R) -> Node {
    Node { position_type: PositionType::Absolute, left: Val::Px(r.x), top: Val::Px(r.y), width: Val::Px(r.w), height: Val::Px(r.h), ..default() }
}

/// A panel's rectangle in screen units, or `None` when the sheet has none.
pub(super) fn panel_origin(layout: &StockLayout, name: &str) -> Option<(R, Vec2)> {
    let area = layout.panel(name)?;
    Some((area, Vec2::new(area.x, area.y)))
}

/// A black rectangle (`<panel>BlackArea`, panel-relative): the panel's dark interior.
fn black_area(commands: &mut Commands, root: Entity, layout: &StockLayout, name: &str, origin: Vec2) {
    if let Some(r) = layout.at(origin, &format!("{name}BlackArea")) {
        commands.entity(root).with_child((node(r), BackgroundColor(Color::BLACK), Pickable::IGNORE));
    }
}

/// A panel's background pieces (`<panel>BackgroundPanel_n`) at their native frames.
pub(super) fn frame_pieces(commands: &mut Commands, root: Entity, layout: &StockLayout, name: &str, origin: Vec2) {
    for piece in layout.pieces(&format!("{name}BackgroundPanel")) {
        let r = R::new(origin.x + piece.rect.x, origin.y + piece.rect.y, piece.rect.w, piece.rect.h);
        commands.entity(root).with_child((node(r), Pickable::IGNORE, GuiImage::new(piece.name).with_frame(piece.frame)));
    }
}

/// The static frames: the control and info panels' dark interiors and background pieces. Spawned
/// with the body, below everything dynamic. The minimap frame is the radar's, the resource
/// strip's is drawn with its readouts.
pub(super) fn spawn_frames(commands: &mut Commands, root: Entity, layout: &StockLayout) {
    for name in ["control", "info", "speed"] {
        let Some((_, origin)) = panel_origin(layout, name) else { continue };
        black_area(commands, root, layout, name, origin);
        frame_pieces(commands, root, layout, name, origin);
    }
}

/// The cinematic panel's rectangle: its pointer shield is ours; its window is [`action`]'s while
/// the HUD is idle and `hud::cinematic`'s (UI-39) while a clip plays in it.
pub(super) fn cinematic_rect(layout: &StockLayout) -> Option<R> {
    layout.panel("cinematic")
}

/// Rectangles that take the pointer (not the world's): the control, info and cinematic panels.
/// The minimap and resource panels are added by the caller, as for Armada II.
pub(super) fn solid(layout: &StockLayout) -> Vec<R> {
    ["control", "info", "speed"].into_iter().filter_map(|name| layout.panel(name)).chain(cinematic_rect(layout)).collect()
}

#[cfg(test)]
mod tests;
