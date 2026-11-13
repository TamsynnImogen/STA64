//! The Stock render profile's backgrounds (RND-23): nebulae, skies and planets
//! as the original games draw them. The Refit look of the same things (volumetric
//! nebulae) stays in `render/nebula.rs` and is chosen by `RenderProfile::Refit`.

pub mod nebula;

use super::profile::{Effect, ProfileEffects};
use bevy::prelude::*;

/// Registers the stock implementations of this package.
pub struct StockBackgroundsPlugin;

impl Plugin for StockBackgroundsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ProfileEffects>();
        app.world_mut().resource_mut::<ProfileEffects>().register_stock(Effect::Scenery);
    }
}
