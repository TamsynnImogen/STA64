//! LCARS theme and widget kit (UI-03). Token table and the font decision are
//! in `STYLE.md`. `ThemePlugin` registers the theme, the pointer observers and
//! the interaction systems in `GameSet::Ui`; `GalleryPlugin` (`--ui-gallery`)
//! shows every widget.

// The kit is for UI-04..08; the binary uses only the gallery for now.
#![allow(dead_code, unused_imports)]

mod gallery;
mod systems;
mod tokens;
mod widgets;

use bevy::prelude::*;

pub use gallery::{GalleryPlugin, GalleryRoot, spawn_gallery};
pub use systems::{ButtonClicked, ListSelected, SliderChanged, ToggleChanged, click, state_colour};
pub use tokens::{Theme, scale};
pub use widgets::*;

use crate::app::GameSet;

pub struct ThemePlugin;

impl Plugin for ThemePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Theme>()
            .add_message::<ButtonClicked>()
            .add_message::<ToggleChanged>()
            .add_message::<SliderChanged>()
            .add_message::<ListSelected>()
            .add_observer(systems::on_over)
            .add_observer(systems::on_out)
            .add_observer(systems::on_press)
            .add_observer(systems::on_release)
            .add_observer(systems::on_click)
            .add_systems(Update, (systems::drive_sliders, systems::paint_widgets, systems::fill_bars).chain().in_set(GameSet::Ui));
    }
}

#[cfg(test)]
mod tests;
