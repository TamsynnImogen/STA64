//! Player input: selection and orders (docs/WORKPLAN.md, UI lane).
//!
//! Input never touches the sim. It writes [`Selection`](crate::sim_link::Selection)
//! and pushes commands into [`CommandQueue`](crate::sim_link::CommandQueue);
//! the sim link stamps and routes them. Systems run in `GameSet::Input`.

pub mod camera_cinematic;
pub mod context;
pub mod hover;
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
            .init_resource::<select::MaxSelection>()
            .add_message::<select::CameraFocus>()
            .init_resource::<orders::OrderTargets>()
            .init_resource::<orders::Aim>()
            .add_systems(Startup, select::spawn_select_box)
            .add_systems(
                Update,
                (
                    orders::aim,
                    select::select_keys,
                    // A click on the HUD, or the one that places a build, is not the map's.
                    select::select.run_if(crate::ui::hud::map_input_free),
                    orders::order.run_if(crate::ui::hud::map_input_free),
                    orders::prune_targets,
                    context::cursor,
                )
                    .chain()
                    .in_set(crate::app::GameSet::Input)
                    .distributive_run_if(resource_exists::<crate::sim_link::SimLink>)
                    // The front end's scene links a sim of ships for show: a click on a menu
                    // button (or the backdrop) must not select or order them.
                    .distributive_run_if(not(resource_exists::<crate::frontend::scene::MenuScene>))
                    // The in-game menu (UI-34) is over the match: nothing under it takes a click or a key.
                    .distributive_run_if(crate::ui::ingame_menu::menu_closed),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim_link::{SimLink, SimLinkPlugin};
    use sta_sim::command::LocalRouter;
    use sta_sim::{PlayerId, Sim};

    /// The match's link goes while the input systems are between two of them (the
    /// menu scene's `close` and the map's `rebuild` remove it with commands, in the
    /// same frame): a run condition checked once for the whole chain would let the
    /// rest run without it and panic on the missing resource.
    #[test]
    fn the_link_going_mid_chain_skips_the_rest() {
        let mut app = App::new();
        app.set_error_handler(bevy::ecs::error::panic)
            .add_plugins(MinimalPlugins)
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>();
        crate::app::configure_sets(&mut app);
        app.add_plugins((SimLinkPlugin, InputPlugin))
            .insert_resource(crate::clock::AnimClock::new(crate::clock::ClockMode::Wall, 0))
            .insert_resource(SimLink::new(Box::new(Sim::new(1)), Box::new(LocalRouter::new()), PlayerId(0)))
            .add_systems(
                Update,
                (|mut commands: Commands| commands.remove_resource::<SimLink>()).after(orders::aim).before(orders::prune_targets),
            );
        app.update();
        app.update();
        assert!(!app.world().contains_resource::<SimLink>());
    }
}
