//! Which plugins run in which mode (docs/WORKPLAN.md S10).
//!
//! Append-only: each package adds one `app.add_plugins(...)` line at the end
//! of its lane's block. On a rebase conflict, keep both lines. Render, UI and
//! audio plugins only run when the mode has a window; `--sim-only` adds none
//! of them.
//!
//! The map view and the model viewer are still added by `main.rs` (they move
//! here in R0-3).

use bevy::prelude::*;

use crate::app::{LaunchConfig, configure_sets};
use crate::sim_link::SimLinkPlugin;

/// Add every lane's plugins for the launch mode, and order the sets.
pub fn add_plugins(app: &mut App, cfg: &LaunchConfig) {
    configure_sets(app);
    // Steps the sim once something inserts a `SimLink`; idle until then.
    app.add_plugins(SimLinkPlugin);

    // input — one line per package, append inside your lane's block

    // bench —

    if cfg.mode().has_window() {
        // render —

        // ui —

        // audio —
    }
}
