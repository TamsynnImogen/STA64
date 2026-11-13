//! Quality settings for what trades look for speed (RND-31). Defaults are the look the
//! renderer had before the setting existed; the performance run (`bench/perf.rs`)
//! switches them to measure what each costs.

use bevy::prelude::*;

#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct RenderQuality {
    /// The key-light shadow pass (`shadows.rs`) may run (it still runs only when something
    /// needs it).
    pub shadows: bool,
}

impl Default for RenderQuality {
    fn default() -> Self {
        RenderQuality { shadows: true }
    }
}
