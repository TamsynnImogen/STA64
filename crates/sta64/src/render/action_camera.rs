//! The action camera (UI-53): the live 3D view of the Armada I stock HUD's cinematic window.
//!
//! What the Armada I manual says of the window (read from data, the install's manual): it shows what
//! happens elsewhere on the map; one click on it goes to the event, a double click switches the
//! main view to the cinematic action view, `Z` returns. The picture seen
//! in one frame of the original: a close, oblique view of the selected ship against the sky, no
//! health bar, no selection ring, no grid.
//!
//! What is implemented here, and its grade:
//! - A second camera pair renders the scene into [`ActionView::image`]; the HUD shows it inside
//!   `cinematicDisplayArea` (`ui::hud::stock::a1::action`). The pair is a sky camera (the sky is a
//!   direction-only picture, so it sits at the main camera's place with the action camera's
//!   orientation) and a scene camera. Neither is the map's [`crate::map::MainCamera`].
//! - The subject is the first selected unit; with no selection the last subject is held until it
//!   is gone. Whether the original also picks the latest event (combat, a completed build) is
//!   **not measured**.
//! - The framing, a fixed oblique angle at a distance that fits the unit's selection radius, is
//!   **not measured** either, and so is how the original moves the camera.
//! - Selection rings, hull and shield bars (the [`Overlay`] pieces) and the strategic grid are on
//!   [`OVERLAY_LAYER`], which only the main camera draws: seen in one frame that the original's
//!   window has none of them.
//! - Not drawn in the window: nebulae (their march belongs to the main camera) and, while the
//!   main view shows a ship as a symbol (zoomed far out), its model, because the model is hidden
//!   by that handoff for every camera. Billboarded effects face the main camera. All **not
//!   measured** against the original; the fog layer is not treated specially.

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use sta_sim::EntityId;

use crate::map::{BackgroundCamera, MatchEntity};
use crate::render::overlays::Overlay;
use crate::render::units::SelectRadius;
use crate::sim_link::{Selection, SimEntities};

/// The layer of what only the map's own camera draws: selection rings and bars, the strategic grid.
pub const OVERLAY_LAYER: usize = 3;

/// Vertical field of view of the action camera (the map camera's default).
const FOV: f32 = std::f32::consts::FRAC_PI_4;
/// Where the camera looks from, relative to the subject: yaw and pitch of a fixed oblique angle.
/// Not measured.
const YAW: f32 = 0.6;
const PITCH: f32 = 0.45;
/// The distance in units of the selection radius, so that the whole unit fits with a margin.
/// Not measured.
const FIT: f32 = 1.5;
/// Smallest and largest picture the camera renders, in pixels per side.
const MIN_SIDE: u32 = 16;
const MAX_SIDE: u32 = 2048;

/// The picture the action camera renders into.
#[derive(Resource, Clone)]
pub struct ActionView {
    pub image: Handle<Image>,
}

/// The node that shows [`ActionView::image`]; its size decides the picture's.
#[derive(Component)]
pub struct ActionPicture;

/// The scene camera of the action view (not the map's [`crate::map::MainCamera`]).
#[derive(Component)]
pub struct ActionCamera;

/// The unit the action camera looks at.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionSubject(pub Option<EntityId>);

/// The subject after a selection: the first selected unit, else the old one while it exists.
pub fn next_subject(selected: &[EntityId], held: Option<EntityId>, exists: impl Fn(EntityId) -> bool) -> Option<EntityId> {
    selected.iter().copied().find(|&id| exists(id)).or(held.filter(|&id| exists(id)))
}

/// Where the camera stands and what it looks at, for a subject at `at` of selection radius `radius`.
pub fn frame(at: Vec3, radius: f32) -> Transform {
    let dist = radius.max(1.0) * FIT / (FOV * 0.5).sin();
    let rot = Quat::from_rotation_y(YAW) * Quat::from_rotation_x(-PITCH);
    let eye = at + rot * Vec3::new(0.0, 0.0, dist);
    Transform::from_translation(eye).looking_at(at, Vec3::Y)
}

/// The side of the picture for a node of `size` physical pixels.
pub fn picture_size(size: Vec2) -> UVec2 {
    let side = |v: f32| (v.round() as u32).clamp(MIN_SIDE, MAX_SIDE);
    UVec2::new(side(size.x), side(size.y))
}

#[derive(Component)]
struct ActionSky;

pub struct ActionCameraPlugin;

impl Plugin for ActionCameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActionSubject>()
            .add_systems(Startup, make_view)
            .add_systems(PostUpdate, grid_layer.run_if(resource_exists::<bevy::gizmos::config::GizmoConfigStore>))
            .add_systems(
                PostUpdate,
                (layer_overlays, subject, sync_cameras)
                    .chain()
                    .after(bevy::transform::TransformSystems::Propagate)
                    .before(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
            );
    }
}

/// The strategic grid is a gizmo: it goes to [`OVERLAY_LAYER`] too (once).
fn grid_layer(mut store: ResMut<bevy::gizmos::config::GizmoConfigStore>, mut done: Local<bool>) {
    if !*done {
        store.config_mut::<bevy::gizmos::config::DefaultGizmoConfigGroup>().0.render_layers = RenderLayers::layer(OVERLAY_LAYER);
        *done = true;
    }
}

fn make_view(mut commands: Commands, images: Option<ResMut<Assets<Image>>>) {
    let Some(mut images) = images else { return };
    let mut image = Image::new_target_texture(MIN_SIDE, MIN_SIDE, TextureFormat::Rgba8UnormSrgb, None);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    // A resized picture is drawn afresh every frame, so the old texture is not worth copying; the
    // copy also needs `COPY_SRC`, which a render target made here does not have, and the failed
    // copy is a validation error that makes the renderer quit.
    image.copy_on_resize = false;
    commands.insert_resource(ActionView { image: images.add(image) });
}

/// Rings, bars and lines of the selection overlays go to [`OVERLAY_LAYER`], with their children.
fn layer_overlays(mut commands: Commands, added: Query<Entity, Added<Overlay>>) {
    for e in &added {
        // `Propagate` reaches the descendants only: the piece itself needs the layer too.
        commands.entity(e).try_insert((RenderLayers::layer(OVERLAY_LAYER), bevy::app::Propagate(RenderLayers::layer(OVERLAY_LAYER))));
    }
}

fn subject(selection: Res<Selection>, entities: Option<Res<SimEntities>>, mut subject: ResMut<ActionSubject>) {
    let exists = |id: EntityId| entities.as_ref().is_some_and(|e| e.0.contains_key(&id));
    // A match ended (its entities are gone): no id carries into the next one.
    if entities.as_ref().is_none_or(|e| e.0.is_empty()) {
        if subject.0.is_some() {
            subject.0 = None;
        }
        return;
    }
    let next = next_subject(&selection.0, subject.0, exists);
    if subject.0 != next {
        subject.0 = next;
    }
}

/// Keep the camera pair while the match runs, switched on only while the HUD shows the picture, and aim it at the subject.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn sync_cameras(
    mut commands: Commands,
    view: Option<Res<ActionView>>,
    mut images: ResMut<Assets<Image>>,
    pictures: Query<(&ComputedNode, &InheritedVisibility), With<ActionPicture>>,
    subject: Res<ActionSubject>,
    entities: Option<Res<SimEntities>>,
    bodies: Query<(&GlobalTransform, Option<&SelectRadius>), (Without<ActionCamera>, Without<ActionSky>)>,
    main: Query<(&GlobalTransform, &Projection), (crate::map::MainCamera, Without<ActionSky>)>,
    mut scene: Query<(&mut Transform, &mut GlobalTransform, &mut Camera), (With<ActionCamera>, Without<ActionSky>)>,
    mut sky: Query<(&mut Transform, &mut GlobalTransform, &mut Camera), (With<ActionSky>, Without<ActionCamera>)>,
) {
    // Only while the picture can be seen: a hidden match UI (loading, a cineractive) costs no passes.
    let size = pictures.iter().filter(|(_, v)| v.get()).map(|(n, _)| n.size()).find(|s| s.x > 0.0 && s.y > 0.0);
    let (Some(view), Some(size), Ok((main_tf, projection))) = (view, size, main.single()) else {
        // The cameras stay, switched off: showing the picture again costs neither a black frame
        // nor a new target.
        for (.., mut cam) in scene.iter_mut().chain(sky.iter_mut()) {
            if cam.is_active {
                cam.is_active = false;
            }
        }
        return;
    };
    for (.., mut cam) in scene.iter_mut().chain(sky.iter_mut()) {
        if !cam.is_active {
            cam.is_active = true;
        }
    }
    let want = picture_size(size);
    if let Some(img) = images.get(&view.image)
        && (img.width(), img.height()) != (want.x, want.y)
        && let Some(mut img) = images.get_mut(&view.image)
    {
        img.resize(bevy::render::render_resource::Extent3d { width: want.x, height: want.y, depth_or_array_layers: 1 });
    }
    let target = subject.0.and_then(|id| entities.as_ref()?.0.get(&id)).and_then(|&e| bodies.get(e).ok());
    let pose = target.map(|(gt, radius)| frame(gt.translation(), radius.map_or(20.0, |r| r.0)));
    if scene.is_empty() {
        let projection = match projection {
            Projection::Perspective(p) => Projection::Perspective(PerspectiveProjection { fov: FOV, ..p.clone() }),
            other => other.clone(),
        };
        let eye = pose.unwrap_or_else(|| Transform::from_translation(main_tf.translation()));
        commands.spawn((
            MatchEntity,
            ActionCamera,
            Camera3d::default(),
            // An intermediate picture: no tonemapper, so no dithering either.
            bevy::core_pipeline::tonemapping::DebandDither::Disabled,
            Camera { order: -9, clear_color: ClearColorConfig::None, ..default() },
            RenderTarget::from(view.image.clone()),
            projection.clone(),
            eye,
            GlobalTransform::from(eye),
        ));
        commands.spawn((
            MatchEntity,
            ActionSky,
            BackgroundCamera,
            Camera3d::default(),
            bevy::core_pipeline::tonemapping::DebandDither::Disabled,
            Camera { order: -10, ..default() },
            RenderTarget::from(view.image.clone()),
            RenderLayers::layer(crate::map::BACKGROUND_LAYER),
            projection,
            eye,
            GlobalTransform::from(eye),
        ));
        return;
    }
    // With no subject the camera keeps its last pose.
    if let Some(pose) = pose {
        // `GlobalTransform` too: this runs after propagation, and the render of this frame reads it.
        for (mut t, mut g, _) in &mut scene {
            *t = pose;
            *g = GlobalTransform::from(pose);
        }
        // The sky is a direction-only picture drawn from the map camera's place.
        for (mut t, mut g, _) in &mut sky {
            *t = Transform { translation: main_tf.translation(), ..pose };
            *g = GlobalTransform::from(*t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_subject_is_the_first_selected_unit_and_is_held_when_the_selection_empties() {
        let alive = |id: EntityId| id.0 != 9;
        assert_eq!(next_subject(&[EntityId(3), EntityId(4)], None, alive), Some(EntityId(3)));
        assert_eq!(next_subject(&[], Some(EntityId(3)), alive), Some(EntityId(3)), "held");
        assert_eq!(next_subject(&[EntityId(9), EntityId(4)], Some(EntityId(3)), alive), Some(EntityId(4)), "a gone unit is skipped");
        assert_eq!(next_subject(&[], Some(EntityId(9)), alive), None, "a gone subject is dropped");
        assert_eq!(next_subject(&[], None, alive), None);
    }

    #[test]
    fn the_frame_looks_at_the_subject_from_a_distance_that_grows_with_its_radius() {
        let at = Vec3::new(100.0, 5.0, -40.0);
        let near = frame(at, 10.0);
        let far = frame(at, 40.0);
        assert!((near.forward().as_vec3().dot((at - near.translation).normalize()) - 1.0).abs() < 1e-4);
        assert!((far.translation - at).length() > 3.9 * (near.translation - at).length());
        assert!(near.translation.y > at.y, "from above");
    }

    #[test]
    fn the_picture_follows_the_node_and_stays_within_bounds() {
        assert_eq!(picture_size(Vec2::new(226.4, 160.0)), UVec2::new(226, 160));
        assert_eq!(picture_size(Vec2::new(1.0, 99999.0)), UVec2::new(MIN_SIDE, MAX_SIDE));
    }
}

#[cfg(test)]
#[path = "action_camera_gpu.rs"]
mod gpu;
