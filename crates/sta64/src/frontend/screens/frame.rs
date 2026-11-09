//! The screens in the bench's headless frames (`--headless --frames`), for the
//! GPU frame test: an environment switch, not a flag (`app.rs` is INT-M3's).
//!
//! - `STA64_UI_SCREEN=<name>`: draw that screen over the bench's frame, on
//!   [`ScreenData::fake`]: `main`, `single`, `setup`, `map`, `settings`, `mods`.
//!
//! The screen's root is printed once as a `screen:` line.

use bevy::camera::RenderTarget;
use bevy::prelude::*;

use super::{Screen, ScreenData, ScreenRoot, ScreensCamera, ScreensConfig, ScreensPlugin};
use crate::app::{AppState, GameSet};

/// The screen `STA64_UI_SCREEN` names.
pub fn requested() -> Option<Screen> {
    parse(std::env::var("STA64_UI_SCREEN").ok()?.trim())
}

pub fn parse(name: &str) -> Option<Screen> {
    Some(match name {
        "main" => Screen::Main,
        "single" => Screen::SinglePlayer,
        "setup" => Screen::Setup,
        "map" => Screen::MapSelect,
        "settings" => Screen::Settings,
        "mods" => Screen::ModSelect,
        _ => return None,
    })
}

/// Add the screens to a frame run: a 2D camera into the capture image
/// `target`, the UI scaled to the 1080-unit screen, the front end entered.
pub fn add_overlay(app: &mut App, screen: Screen, target: Handle<Image>, frame_height: u32) {
    app.insert_resource(UiScale(frame_height as f32 / 1080.0))
        .insert_resource(ScreenData::fake())
        .insert_resource(ScreensConfig { own_camera: false, fit_scale: false, start: screen })
        .insert_state(AppState::FrontEnd)
        .add_plugins(ScreensPlugin)
        .add_systems(Startup, move |mut commands: Commands| {
            let cam = commands
                .spawn((Camera2d, Camera { order: 1, clear_color: ClearColorConfig::None, ..default() }, RenderTarget::from(target.clone()), Msaa::Off))
                .id();
            commands.insert_resource(ScreensCamera(cam));
        })
        .add_systems(Update, announce.in_set(GameSet::Ui));
}

fn announce(roots: Query<&ScreenRoot, Added<ScreenRoot>>) {
    for r in &roots {
        println!("screen: {:?}", r.0);
    }
}
