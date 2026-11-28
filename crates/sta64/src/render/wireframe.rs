//! `--wireframe`: draw the models as wireframe, for debugging and for promotional renders.
//!
//! Bevy's own [`WireframePlugin`] draws the lines: a separate pass after the main pass that
//! takes the entity's [`Mesh3d`] and its own pipeline, so it does not depend on the material
//! (the hull look, `StandardMaterial` and the rest all work the same). It needs the wgpu
//! features `POLYGON_MODE_LINE` and `IMMEDIATES`. Bevy's default renderer settings already
//! turn on every feature the adapter has, so nothing is requested here; [`WireframeViewPlugin`]
//! only checks the device when the renderer is up and stops with a plain message if the
//! adapter lacks them. Without the flag this module adds nothing: no plugin, no system.
//!
//! The lines belong to the parts of loaded models ([`ModelPart`]) that hang under a unit
//! ([`SimEntity`]: ships, stations and the other objects the sim has), never to the sky (a
//! model too), nebulae, planets or effects. Three
//! modes, for compositing renders of the same frame:
//!
//! - **overlay** (`--wireframe`): the models stay filled and the lines are drawn over them,
//!   hidden lines removed by the depth buffer;
//! - **lines** (`--wireframe=lines`): the models' fill is not drawn, only their lines, over
//!   the unchanged background (back faces are culled as in the fill);
//! - **only** (`--wireframe=only`): as lines, with everything else hidden on a black
//!   background, so the line pixels can be isolated exactly.
//!
//! The camera, the animation clock and the scene are untouched, so the passes of one scene
//! and tick are pixel-aligned wherever they show the same thing.

use bevy::camera::visibility::VisibilitySystems;
use bevy::pbr::wireframe::{Wireframe, WireframeColor, WireframeLineWidth, WireframePlugin};
use bevy::prelude::*;
use bevy::render::render_resource::WgpuFeatures;
use bevy::render::renderer::RenderDevice;
use bevy::render::RenderApp;

use crate::app::LaunchConfig;
use crate::render::appearance::ModelPart;
use crate::render::hulls::HullMaterial;
use crate::sim_link::SimEntity;

/// What `--wireframe` draws (module docs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WireframeMode {
    /// Lines over the filled models.
    #[default]
    Overlay,
    /// Lines only, background as usual.
    Lines,
    /// Lines only, on black, everything else hidden.
    Only,
}

impl WireframeMode {
    /// The value after `--wireframe`: `overlay`, `lines` or `only`.
    pub fn parse(s: &str) -> Option<WireframeMode> {
        match s {
            "overlay" => Some(WireframeMode::Overlay),
            "lines" => Some(WireframeMode::Lines),
            "only" => Some(WireframeMode::Only),
            _ => None,
        }
    }
}

/// Line width when `--wireframe-width` gives none, in pixels. Bevy draws widths above 1 as quads
/// pulled from the vertex buffer, which break when an edge crosses the camera's near plane
/// (a ship seen close up); 1 uses the GPU's own lines and is always right.
pub const DEFAULT_WIDTH: f32 = 1.0;

/// The wireframe view's settings, from the launch flags.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct WireframeView {
    pub mode: WireframeMode,
    /// sRGB.
    pub color: [u8; 3],
    pub width: f32,
}

impl WireframeView {
    /// `Some` when `--wireframe` was given.
    pub fn from_cfg(cfg: &LaunchConfig) -> Option<WireframeView> {
        Some(WireframeView { mode: cfg.wireframe?, color: cfg.wireframe_color.unwrap_or([255, 255, 255]), width: cfg.wireframe_width.unwrap_or(DEFAULT_WIDTH) })
    }
}

/// Adds Bevy's wireframe pass and the systems that mark the models' parts.
pub struct WireframeViewPlugin(pub WireframeView);

impl Plugin for WireframeViewPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(WireframePlugin::default()).insert_resource(self.0);
        if self.0.mode == WireframeMode::Only {
            app.insert_resource(ClearColor(Color::BLACK));
        }
        app.add_systems(PostUpdate, (mark_parts, hide_others).chain().before(VisibilitySystems::VisibilityPropagate));
    }

    fn finish(&self, app: &mut App) {
        let Some(device) = app.get_sub_app(RenderApp).and_then(|r| r.world().get_resource::<RenderDevice>()) else {
            return;
        };
        let need = WgpuFeatures::POLYGON_MODE_LINE | WgpuFeatures::IMMEDIATES;
        let missing = need - device.features();
        if !missing.is_empty() {
            eprintln!("--wireframe needs a GPU that supports {need:?}; this adapter lacks {missing:?}");
            std::process::exit(78);
        }
    }
}

/// Marks a mesh that belongs to a unit's model and has its lines.
#[derive(Component)]
struct Lined;

/// Marks a model part that is not a unit's (the map's sky, for one): no lines.
#[derive(Component)]
struct Backdrop;

/// Give every part of a unit's model its lines, and in `lines`/`only` mode take its fill away
/// (the materials are removed; the wireframe pass needs only the mesh). A model part with no
/// unit above it (the map's sky is a model too) gets none. Runs every frame: parts are
/// spawned and the hull look puts its material on after the part is spawned.
#[allow(clippy::type_complexity)]
fn mark_parts(
    mut commands: Commands,
    view: Res<WireframeView>,
    fresh: Query<Entity, (With<ModelPart>, With<Mesh3d>, Without<Lined>, Without<Backdrop>)>,
    parents: Query<&ChildOf>,
    units: Query<(), With<SimEntity>>,
    filled: Query<Entity, (With<Lined>, Or<(With<MeshMaterial3d<StandardMaterial>>, With<MeshMaterial3d<HullMaterial>>)>)>,
) {
    let [r, g, b] = view.color;
    for e in &fresh {
        if parents.iter_ancestors(e).any(|a| units.contains(a)) {
            commands.entity(e).try_insert((Lined, Wireframe, WireframeColor { color: Color::srgb_u8(r, g, b) }, WireframeLineWidth { width: view.width }));
        } else {
            commands.entity(e).try_insert(Backdrop);
        }
    }
    if view.mode != WireframeMode::Overlay {
        for e in &filled {
            commands.entity(e).try_remove::<MeshMaterial3d<StandardMaterial>>().try_remove::<MeshMaterial3d<HullMaterial>>();
        }
    }
}

/// `only`: every mesh that is not part of a unit's model is hidden (sky, nebulae, planets, effects).
#[allow(clippy::type_complexity)]
fn hide_others(view: Res<WireframeView>, mut others: Query<&mut Visibility, (With<Mesh3d>, Without<Lined>)>) {
    if view.mode != WireframeMode::Only {
        return;
    }
    for mut v in &mut others {
        v.set_if_neq(Visibility::Hidden);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_parse() {
        assert_eq!(WireframeMode::parse("only"), Some(WireframeMode::Only));
        assert_eq!(WireframeMode::parse("lines"), Some(WireframeMode::Lines));
        assert_eq!(WireframeMode::parse("overlay"), Some(WireframeMode::Overlay));
        assert_eq!(WireframeMode::parse("fill"), None);
    }
}
