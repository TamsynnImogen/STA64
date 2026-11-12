//! The menu widget kit (UI-21): dropdown selects, scrolling lists, check boxes,
//! sliders, text fields, tabs and buttons, for the front end (UI-22), the in-game
//! options menu (UI-20) and the map editor (EDT-12, no egui). Our own implementation;
//! no stock art.
//!
//! **Look.** Every colour and size comes from the [`Theme`] resource, which the active
//! [`UiProfile`](crate::ui::profile::UiProfile) fills: LCARS, or the neutral skin of
//! Armada II / Fleet Operations (`UiProfile::theme`). Switching the profile switches
//! the theme; builders read it when they spawn, the paint systems when state changes.
//! **Text** is drawn with [`kit_text`]: the mod's own sprite font for the role the size
//! stands for (`FontSmall`/`FontMedium`/`FontLarge`/`FontVeryLarge`, UI-16), the baked
//! TTF where the mod has none. Row heights follow the theme, never a hard-coded value.
//!
//! **API.** Builders return bundles to `spawn` or nest in `children!`:
//! [`dropdown`], [`scroll_list`], [`checkbox`], [`text_field`], [`tab_bar`] with
//! [`tab_panel`], [`slider_focusable`] and, from the LCARS kit, `button`, `toggle`,
//! `slider`, `list`, `panel` (wrap with [`focusable`] for keyboard access). Widgets
//! report through messages: [`DropdownChanged`], [`CheckboxChanged`], [`TextChanged`],
//! [`TextSubmitted`], [`TabChanged`], and the LCARS kit's `ButtonClicked`,
//! `ToggleChanged`, `SliderChanged`, `ListSelected`. Read a widget's value from its
//! component (`Dropdown::selected`, `Checkbox::on`, `TextField::text`, ...).
//!
//! **Input.** Mouse through the picking observers; keyboard through [`KitFocus`]:
//! Tab/Shift+Tab walk the [`Focusable`] widgets, a press focuses the one under the
//! pointer, and each widget handles its own keys (see the module docs). [`kit_key`],
//! [`kit_click`] and [`press`] drive the same paths without a window, for tests and
//! the bench.

mod checkbox;
mod dropdown;
mod focus;
mod scroll;
mod tabs;
mod text_field;

#[cfg(test)]
mod gpu;
#[cfg(test)]
mod tests;

use bevy::picking::events::PointerClick;
use bevy::prelude::*;

pub use checkbox::{Checkbox, CheckboxChanged, checkbox};
pub use dropdown::{Dropdown, DropdownChanged, DropdownOption, DropdownPopup, MAX_ROWS, dropdown};
pub use focus::{FocusScope, Focusable, KitCapture, KitFocus, KitKey, focus_order, focusable, is_displayed, kit_key, move_focus, press};
pub use scroll::{ScrollArea, SliderStep, scroll_list, scroll_to_row, slider_focusable};
pub use tabs::{Tab, TabBar, TabChanged, TabPanel, tab_bar, tab_panel};
pub use text_field::{TextChanged, TextField, TextFilter, TextSubmitted, text_field};

use super::{Theme, WidgetPaint, WidgetState};
use crate::app::GameSet;
use crate::ui::sprite_font::{FontRole, SpriteTextStyle, sprite_text_styled};

/// The game's named font a text of `size` px stands for in this theme: the small
/// tokens use `FontSmall`, body text `FontMedium`, titles `FontLarge`, display text
/// `FontVeryLarge`. A mod without that file gets the nearest it has.
pub fn font_role(theme: &Theme, size: f32) -> FontRole {
    if size <= theme.small_size {
        FontRole::Small
    } else if size <= theme.text_size {
        FontRole::Medium
    } else if size <= theme.title_size {
        FontRole::Large
    } else {
        FontRole::VeryLarge
    }
}

/// Text in the theme's font for its size (UI-16).
pub fn kit_text(theme: &Theme, s: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    sprite_text_styled(s, size, color, SpriteTextStyle::role(font_role(theme, size)))
}

/// What a click on `e` does for the widgets of this module (the LCARS kit's `click`
/// handles buttons, toggles and list rows). Public so tests click without a pointer.
pub fn kit_click(w: &mut World, e: Entity) {
    if w.get_resource::<crate::platform::LifecycleGate>().is_some_and(|gate| !gate.is_foreground()) {
        return;
    }
    checkbox::click(w, e);
    dropdown::click(w, e);
    tabs::click(w, e);
}

/// `PointerClick` bubbles up the tree and global observers see every step: the first
/// kit widget on the way handles it and stops it, so a row's click never reaches the
/// dropdown that owns the list (which would toggle it open again).
fn on_click(
    mut ev: On<PointerClick>,
    kit: Query<(), Or<(With<Checkbox>, With<Dropdown>, With<DropdownOption>, With<Tab>)>>,
    mut commands: Commands,
) {
    let e = ev.entity;
    if kit.contains(e) {
        ev.propagate(false);
    }
    commands.queue(move |w: &mut World| kit_click(w, e));
}

/// Systems of the kit run in this set, after the LCARS kit's painter.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KitSet;

pub struct KitPlugin;

impl Plugin for KitPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KitFocus>()
            .init_resource::<KitCapture>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<bevy::picking::hover::HoverMap>()
            .add_message::<bevy::input::keyboard::KeyboardInput>()
            .add_message::<bevy::input::mouse::MouseWheel>()
            .add_message::<CheckboxChanged>()
            .add_message::<DropdownChanged>()
            .add_message::<TextChanged>()
            .add_message::<TextSubmitted>()
            .add_message::<TabChanged>()
            .add_observer(on_click)
            .add_systems(
                Update,
                (
                    focus::pointer_press,
                    focus::key_events,
                    focus::capture,
                    scroll::wheel,
                    dropdown::hover_highlight,
                    dropdown::sync,
                    text_field::sync,
                    (checkbox::paint, dropdown::paint, tabs::sync, dropdown::flip_popups, focus::paint_focus),
                )
                    .chain()
                    .in_set(KitSet)
                    .in_set(GameSet::Ui)
                    .after(WidgetPaint),
            );
    }
}
