//! The part of the stock HUD's cinematic window both Armada I and Armada II share (UI-53, #1073):
//! the dark display, an optional still picture under the live one, the live action picture
//! ([`crate::render::action_camera`]) and the frame art over its edges, all inside the
//! `cinematicPanelArea` panel.
//!
//! Layout keys read (the interface layout MAY declare each; read from data, the install's gui
//! cfg): `cinematicPanelArea` (a screen place, so the panel moves with the sheet's group and takes
//! part in both fits), panel-relative `cinematicDisplayArea` (`cinematicBackgroundArea` when
//! absent), the frame pieces `cinematicBackgroundPanel` with `cinematicBackgroundPanelSize` and
//! `cinematicBackgroundPanel_n`, and the still picture `cinematicDefaultBackground`.
//!
//! Armada I adds its two buttons ([`super::a1::action`]). Armada II draws no buttons.
//!
//! **Declared off.** A layout that gives the panel no visible rectangle declares the window off:
//! nothing is drawn, whatever the game. The decision is read from the mounted layout at run time,
//! never from a list of games.
//!
//! **Subject and framing are interim for Armada II.** The picture is the existing action camera,
//! unchanged: the subject choice, framing and gating are the Armada I window's. The original's
//! rules for Armada II differ (read from the executable, static; not measured) and are a later
//! package; nothing of them is built here. The double click into another view is not built.
//!
//! **Fits.** The panel, its frame pieces, the still and the picture are placed through the one
//! [`StockLayout`] the rest of the HUD uses, so they move as a whole with the sheet: `Centred`
//! keeps the sheet's aspect and the window's picture keeps its rectangle's aspect (the camera
//! target is sized to the node, so its projection aspect matches the rectangle); `Stretched`
//! scales the rectangles per axis like every other panel and the picture follows its node.

use bevy::{picking::Pickable, prelude::*};

use super::buttons::PanelButton;
use super::layout::StockLayout;
use super::{a1::frame_pieces, a1::node, a1::panel_origin};
use crate::{
    render::action_camera::{ActionPicture, ActionView},
    ui::{
        gui_sprites::GuiImage,
        theme::{UiButton, WidgetState},
    },
};

/// Spawn the window's shared parts under `root`. Returns the panel's origin (for the caller's
/// own pieces), or `None` when the layout declares no window: then nothing was spawned.
/// `still` draws the layout's still picture (`cinematicDefaultBackground`) under the live one.
pub(in crate::ui::hud) fn spawn(
    commands: &mut Commands,
    root: Entity,
    layout: &StockLayout,
    view: Option<&ActionView>,
    still: bool,
) -> Option<Vec2> {
    let (_, origin) = panel_origin(layout, "cinematic")?;
    let display = layout.at(origin, "cinematicDisplayArea").or_else(|| layout.at(origin, "cinematicBackgroundArea"));
    if let Some(r) = display {
        commands.entity(root).with_child((node(r), BackgroundColor(Color::BLACK), Pickable::IGNORE));
        if still && let Some(name) = layout.name("cinematicDefaultBackground") {
            commands.entity(root).with_child((node(r), Pickable::IGNORE, GuiImage::new(name)));
        }
        if let Some(view) = view {
            commands.entity(root).with_child((
                ActionPicture,
                PanelButton::ActionView,
                UiButton,
                WidgetState::Normal,
                node(r),
                ImageNode::new(view.image.clone()),
            ));
        }
    }
    // The one call site of the frame pieces for the window.
    frame_pieces(commands, root, layout, "cinematic", origin);
    Some(origin)
}

/// The Armada II window: the shared parts with the still picture, and no buttons.
pub(in crate::ui::hud) fn spawn_a2(commands: &mut Commands, root: Entity, layout: &StockLayout, view: Option<&ActionView>) -> bool {
    spawn(commands, root, layout, view, true).is_some()
}

