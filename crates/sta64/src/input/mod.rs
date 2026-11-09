//! Player input: selection and orders (docs/WORKPLAN.md, UI lane).
//!
//! Input never touches the sim. It writes [`Selection`](crate::sim_link::Selection)
//! and pushes commands into [`CommandQueue`](crate::sim_link::CommandQueue);
//! the sim link stamps and routes them. Systems run in `GameSet::Input`.

pub mod keys;
pub mod orders;
pub mod select;

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

/// The cursor and the ray under it, if the window has a cursor.
fn cursor_ray(window: &Window, cam: (&Camera, &GlobalTransform)) -> Option<(Vec2, Ray3d)> {
    let c = window.cursor_position()?;
    cam.0.viewport_to_world(cam.1, c).ok().map(|r| (c, r))
}

/// The map camera and the primary window, as one system parameter.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Pointer<'w, 's> {
    window: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    camera: Query<'w, 's, (&'static Camera, &'static GlobalTransform), crate::map::MainCamera>,
}

impl Pointer<'_, '_> {
    /// The camera, and the cursor with its ray.
    fn get(&self) -> Option<((&Camera, &GlobalTransform), Vec2, Ray3d)> {
        let (window, cam) = (self.window.single().ok()?, self.camera.single().ok()?);
        let (cursor, ray) = cursor_ray(window, cam)?;
        Some((cam, cursor, ray))
    }
}

/// Registers selection and orders.
pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<select::Drag>()
            .init_resource::<select::ControlGroups>()
            .init_resource::<keys::KeyBindings>()
            .add_message::<select::CameraFocus>()
            .init_resource::<orders::OrderTargets>()
            .init_resource::<orders::Aim>()
            .add_systems(Startup, select::spawn_select_box)
            .add_systems(
                Update,
                (orders::aim, select::select_keys, select::select, orders::order, orders::prune_targets)
                    .chain()
                    .in_set(crate::app::GameSet::Input)
                    .run_if(resource_exists::<crate::sim_link::SimLink>),
            );
    }
}
