//! The Armada I cinematic window (UI-53): the COMM and MENU buttons above a live picture of the
//! action camera ([`crate::render::action_camera`]), inside the `cinematic*` panel of the cfg.
//!
//! Places come from the cfg, as for every other panel: `cinematicPanelArea` (a screen place, so
//! the panel takes part in the bottom row like the others), then panel-relative
//! `cinematicDisplayArea` (`cinematicBackgroundArea` when absent), `cinematicBackgroundPanel_n`,
//! `cinematicMenuButtonArea` and `cinematicCommButtonArea`, and the sprites `cinematicMenuButton`
//! and `cinematicCommButton` (read from data, the install's gui cfg and sprite table).
//!
//! Rules and their grade:
//! - **MENU** opens the game's options menu (objectives, save and load, settings, abort; tooltip
//!   key `GUI_CV_MAIN_TOOLTIP`; read from data, the install's manual and string table). Here: the
//!   same in-game menu the Esc key opens (`OpenInGameMenu`). The Ctrl-X shortcut the manual names is
//!   not wired (TODO line).
//! - **COMM** opens the multiplayer options (alliances, chat, gifts; `GUI_CV_COMM_TOOLTIP`; the
//!   manual makes it multiplayer only; read from data). STA64 has no in-match alliance, chat or
//!   gift menu, so the button is drawn in its disabled state and does nothing. Seen in one frame of
//!   the original: a single-player game draws it dark, the MENU button lit.
//! - **Labels**: the two captions are literals of the executable, not string-table keys (read from
//!   the executable, static); drawn here as the same words.
//! - **States**: one sprite per button, tinted by state (the sprite table has no hover or pressed
//!   art; read from data). The tints and the label colour are approximated from one frame of the
//!   original, **not measured**. The `cinematicMenuBorder` and `cinematicCommBorder` sprites are
//!   not drawn: where the original uses them is not established.
//! - **Picture**: one click on it moves the map camera to what it shows (the manual; read from
//!   data). The double click into the
//!   cinematic action view, `Z`, and minimising the panel (`cinematicMinimizedPanel*`,
//!   Ctrl-F4) are not built (TODO lines).

use bevy::prelude::*;

use super::super::{buttons::PanelButton, info::readout, layout::StockLayout, window};
use super::node;
use crate::{
    render::action_camera::ActionView,
    ui::{
        gui_sprites::{GuiButton, GuiButtonColors},
        i18n::Strings,
        notices::Tooltip,
        theme::{Theme, UiButton, WidgetState},
    },
};

/// A button of the window: its cfg keys, what it does, its caption, and its tooltip key.
struct Key {
    area: &'static str,
    sprite: &'static str,
    act: PanelButton,
    label: &'static str,
    tip: &'static str,
    /// The English words used when the string table has none.
    fallback: &'static str,
}

const BUTTONS: [Key; 2] = [
    Key { area: "cinematicCommButtonArea", sprite: "cinematicCommButton", act: PanelButton::Unavailable, label: "COMM", tip: "GUI_CV_COMM_TOOLTIP", fallback: "Comm" },
    Key { area: "cinematicMenuButtonArea", sprite: "cinematicMenuButton", act: PanelButton::Menu, label: "MENU", tip: "GUI_CV_MAIN_TOOLTIP", fallback: "Menu" },
];

/// The tints of the two buttons' one sprite per state. Approximated from one frame; not measured.
fn colours() -> GuiButtonColors {
    GuiButtonColors {
        normal: Color::srgb(0.6, 0.7, 0.75),
        hovered: Color::WHITE,
        pressed: Color::srgb(0.45, 0.5, 0.55),
        disabled: Color::srgb(0.12, 0.12, 0.1),
        active: Color::srgb(0.6, 0.7, 0.75),
    }
}

/// The window's text for a tooltip key: the string table's, else the English fallback.
fn words(strings: Option<&Strings>, key: &str, fallback: &str) -> String {
    strings.map(|s| s.tooltip_text(key)).filter(|t| *t != key).unwrap_or(fallback).replace('^', "")
}

/// Spawn the window: the shared parts (dark display, action picture, frame art over its edges;
/// [`super::super::window`]) and the two buttons. Does nothing when the cfg has no `cinematicPanelArea`.
pub(in crate::ui::hud) fn spawn(commands: &mut Commands, root: Entity, layout: &StockLayout, theme: &Theme, strings: Option<&Strings>, view: Option<&ActionView>) {
    let Some(origin) = window::spawn(commands, root, layout, view, false) else { return };
    for k in &BUTTONS {
        let Some(r) = layout.at(origin, k.area) else { continue };
        let Some(sprite) = layout.name(k.sprite) else { continue };
        let state = if k.act == PanelButton::Unavailable { WidgetState::Disabled } else { WidgetState::Normal };
        commands.entity(root).with_child((
            k.act,
            UiButton,
            state,
            colours(),
            Tooltip::new(words(strings, k.tip, k.fallback)),
            GuiButton { names: vec![sprite.to_owned()], keep_label: false },
            node(r),
            BackgroundColor(theme.button),
        ));
        readout(commands, root, r, k.label.to_owned(), Color::srgb(0.08, 0.07, 0.02), theme.micro_size, None);
    }
}
