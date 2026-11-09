//! Which plugins run in which mode (docs/WORKPLAN.md S10).
//!
//! Append-only: each package adds one `app.add_plugins(...)` line at the end
//! of its lane's block. On a rebase conflict, keep both lines. Render, UI and
//! audio plugins only run when the mode has a window; `--sim-only` adds none
//! of them.
//!
//! The map view and the model viewer are added here by mode; `main.rs` inserts
//! the resources they read first.

use bevy::prelude::*;

use crate::app::{LaunchConfig, configure_sets};
use crate::clock::AnimClock;
use crate::input::InputPlugin;
use crate::map::MapPlugin;
use crate::render::units::UnitsRenderPlugin;
use crate::sim_link::SimLinkPlugin;
use crate::viewer::ViewerPlugin;

/// Add every lane's plugins for the launch mode, and order the sets.
pub fn add_plugins(app: &mut App, cfg: &LaunchConfig) {
    configure_sets(app);
    // Steps the sim once something inserts a `SimLink`; idle until then.
    app.insert_resource(AnimClock::new(cfg.clock_mode(), 0)).add_plugins(SimLinkPlugin);

    // The map view or the model viewer, chosen by the flags as before the bench
    // flags existed (they do nothing yet): `--map --headless` still opens the map.
    let map_view = cfg.map.is_some();
    if map_view {
        app.add_plugins(MapPlugin);
        // The map's ships; the map view is always windowed, so not gated below.
        app.add_plugins(UnitsRenderPlugin);
        app.add_plugins(crate::render::overlays::OverlaysPlugin);
        app.add_plugins(crate::render::fx::weapons::WeaponFxPlugin);
        app.add_plugins(crate::render::fx::specials::SpecialsFxPlugin);
        app.add_plugins(crate::render::fx::impacts::ImpactFxPlugin);
    } else if cfg.ui_gallery {
        app.add_plugins(crate::ui::theme::GalleryPlugin);
    } else {
        app.add_plugins(ViewerPlugin);
    }

    // input — one line per package, append inside your lane's block
    if map_view {
        app.add_plugins(InputPlugin);
    }

    // bench —

    if cfg.mode().has_window() {
        // render —

        // ui —
        app.add_plugins(crate::ui::theme::ThemePlugin);

        // audio —
        app.add_plugins(crate::audio::AudioPlugin { mode: cfg.mode() });
    }
}
