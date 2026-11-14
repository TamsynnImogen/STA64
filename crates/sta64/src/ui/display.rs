//! Applies the saved display choices to the primary window.

use bevy::prelude::*;
use bevy::window::{PresentMode, PrimaryWindow};
#[cfg(not(target_os = "android"))]
use bevy::window::{MonitorSelection, WindowMode};

use crate::frontend::screens::Settings;

#[derive(Clone, Copy, PartialEq, Eq)]
struct DisplayValues {
    fullscreen: bool,
    vsync: bool,
    size: Option<(u32, u32)>,
}

/// Window state is kept separately from `Settings` so unrelated edits never resize it.
#[derive(Resource, Default)]
struct AppliedDisplay {
    window: Option<Entity>,
    last: Option<DisplayValues>,
    windowed_size: Option<(u32, u32)>,
}

pub struct DisplayPlugin;

impl Plugin for DisplayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AppliedDisplay>()
            .add_systems(PostUpdate, apply_display);
    }
}

fn apply_display(
    settings: Option<Res<Settings>>,
    mut state: ResMut<AppliedDisplay>,
    mut windows: Query<(Entity, &mut Window), With<PrimaryWindow>>,
) {
    let Some(settings) = settings else { return };
    let Ok((entity, mut window)) = windows.single_mut() else { return };
    let values = DisplayValues { fullscreen: settings.fullscreen, vsync: settings.vsync, size: settings.window_size.dimensions() };
    let fresh = state.window != Some(entity);
    let old = if fresh { None } else { state.last };
    if fresh {
        state.window = Some(entity);
        state.windowed_size = Some((window.resolution.physical_width(), window.resolution.physical_height()));
    }
    if !fresh && old == Some(values) {
        return;
    }

    // Android's native window owns its mode and resolution. VSync remains configurable.
    #[cfg(not(target_os = "android"))]
    {
        let mode_changed = old.is_none_or(|previous| previous.fullscreen != values.fullscreen);
        let size_changed = old.is_none_or(|previous| previous.size != values.size);
        if mode_changed {
            if values.fullscreen {
                // Keep the user's latest manual window size for Automatic restoration.
                if !fresh {
                    state.windowed_size = Some((window.resolution.physical_width(), window.resolution.physical_height()));
                }
                window.mode = WindowMode::BorderlessFullscreen(MonitorSelection::Current);
            } else {
                window.mode = WindowMode::Windowed;
                let restore = if fresh { None } else { state.windowed_size };
                if let Some((width, height)) = values.size.or(restore) {
                    // A chosen preset must leave maximized mode; Automatic preserves it.
                    if values.size.is_some() {
                        window.set_maximized(false);
                    }
                    window.resolution.set_physical_resolution(width, height);
                }
            }
        } else if !values.fullscreen && size_changed
            && let Some((width, height)) = values.size
        {
            window.set_maximized(false);
            window.resolution.set_physical_resolution(width, height);
        }
    }
    if old.is_none_or(|previous| previous.vsync != values.vsync) {
        window.present_mode = if values.vsync { PresentMode::AutoVsync } else { PresentMode::AutoNoVsync };
    }
    state.last = Some(values);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::window::WindowResolution;
    #[cfg(not(target_os = "android"))]
    use crate::frontend::screens::WindowSize;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(DisplayPlugin);
        app
    }

    fn window(app: &mut App, width: u32, height: u32) -> Entity {
        app.world_mut().spawn((Window {
            resolution: WindowResolution::new(width, height),
            ..default()
        }, PrimaryWindow)).id()
    }

    fn size(app: &App, entity: Entity) -> (u32, u32) {
        let window = app.world().get::<Window>(entity).unwrap();
        (window.resolution.physical_width(), window.resolution.physical_height())
    }

    #[test]
    fn absent_settings_or_window_and_replaced_window_are_safe() {
        let mut app = app();
        app.update();
        let first = window(&mut app, 1300, 800);
        app.update();
        assert_eq!(size(&app, first), (1300, 800));
        app.insert_resource(Settings::default());
        app.update();
        assert_eq!(app.world().get::<Window>(first).unwrap().present_mode, PresentMode::AutoVsync);
        app.world_mut().despawn(first);
        app.update();
        let replacement = window(&mut app, 1400, 900);
        app.update();
        assert_eq!(size(&app, replacement), (1400, 900));
    }

    #[test]
    #[cfg(target_os = "android")]
    fn android_keeps_native_window_mode_and_size() {
        let mut app = app();
        let entity = window(&mut app, 1300, 800);
        let mut settings = Settings::default();
        settings.fullscreen = true;
        settings.window_size = crate::frontend::screens::WindowSize::R1920x1080;
        settings.vsync = false;
        app.insert_resource(settings);
        app.update();
        assert_eq!(size(&app, entity), (1300, 800));
        assert!(matches!(&app.world().get::<Window>(entity).unwrap().mode, bevy::window::WindowMode::Windowed));
        assert_eq!(app.world().get::<Window>(entity).unwrap().present_mode, PresentMode::AutoNoVsync);
    }

    #[test]
    #[cfg(not(target_os = "android"))]
    fn unrelated_edits_preserve_manual_size_and_fullscreen_restores_it() {
        let mut app = app();
        let entity = window(&mut app, 1280, 720);
        app.insert_resource(Settings::default());
        app.update();
        app.world_mut().get_mut::<Window>(entity).unwrap().resolution.set_physical_resolution(1310, 790);
        app.world_mut().resource_mut::<Settings>().master = 42.0;
        app.update();
        assert_eq!(size(&app, entity), (1310, 790));
        app.world_mut().get_mut::<Window>(entity).unwrap().set_maximized(true);
        let pending_maximize = app.world().get::<Window>(entity).unwrap().internal;
        app.world_mut().resource_mut::<Settings>().fullscreen = true;
        app.update();
        assert!(matches!(&app.world().get::<Window>(entity).unwrap().mode, WindowMode::BorderlessFullscreen(_)));
        app.world_mut().get_mut::<Window>(entity).unwrap().resolution.set_physical_resolution(1920, 1080);
        app.world_mut().resource_mut::<Settings>().fullscreen = false;
        app.update();
        assert!(matches!(&app.world().get::<Window>(entity).unwrap().mode, WindowMode::Windowed));
        assert_eq!(size(&app, entity), (1310, 790));
        assert_eq!(app.world().get::<Window>(entity).unwrap().internal, pending_maximize);
    }

    #[test]
    #[cfg(not(target_os = "android"))]
    fn preset_is_applied_when_windowed_and_vsync_changes_live() {
        let mut app = app();
        let entity = window(&mut app, 1100, 700);
        let mut settings = Settings::default();
        settings.fullscreen = true;
        settings.window_size = WindowSize::R1600x900;
        settings.vsync = false;
        app.insert_resource(settings);
        app.update();
        assert_eq!(app.world().get::<Window>(entity).unwrap().present_mode, PresentMode::AutoNoVsync);
        app.world_mut().resource_mut::<Settings>().fullscreen = false;
        app.update();
        assert_eq!(size(&app, entity), (1600, 900));
        app.world_mut().resource_mut::<Settings>().vsync = true;
        app.update();
        assert_eq!(app.world().get::<Window>(entity).unwrap().present_mode, PresentMode::AutoVsync);
        app.world_mut().resource_mut::<Settings>().window_size = WindowSize::R1280x720;
        app.update();
        assert_eq!(size(&app, entity), (1280, 720));
        app.world_mut().get_mut::<Window>(entity).unwrap().resolution.set_physical_resolution(1290, 730);
        app.world_mut().resource_mut::<Settings>().effects = 33.0;
        app.update();
        assert_eq!(size(&app, entity), (1290, 730));
    }
}
