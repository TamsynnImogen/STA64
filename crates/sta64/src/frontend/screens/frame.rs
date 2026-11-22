//! The screens in the bench's headless frames (`--headless --frames`), for the
//! GPU frame test: an environment switch, not a flag (`app.rs` is INT-M3's).
//!
//! - `STA64_UI_SCREEN=<name>`: draw that screen over the bench's frame, on
//!   [`ScreenData::fake`]: `main`, `single`, `setup`, `advanced`, `map`, `settings`, `mods`.
//! - `STA64_UI_MAP=<stem>`: show that installed map's preview and the mounted
//!   palette on the setup frame.
//!
//! The screen's root is printed once as a `screen:` line.

use bevy::camera::RenderTarget;
use bevy::prelude::*;

use super::{MapEntry, Screen, ScreenData, ScreenRoot, ScreensCamera, ScreensConfig, ScreensPlugin};
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
        "advanced" => Screen::Advanced,
        "map" => Screen::MapSelect,
        "settings" => Screen::Settings,
        "mods" => Screen::ModSelect,
        _ => return None,
    })
}

/// Add the screens to a frame run: a 2D camera into the capture image
/// `target`, the UI scaled to the 1080-unit screen, the front end entered.
pub fn add_overlay(app: &mut App, screen: Screen, target: Handle<Image>, frame_size: (u32, u32), cfg: &crate::app::LaunchConfig) {
    let mut data = ScreenData::fake();
    if screen == Screen::ModSelect && let Some(root) = &cfg.install_root {
        let install_data = root.join("Data");
        data.mods = ScreenData::mods_in(if install_data.is_dir() { &install_data } else { root }).0;
        data.base_name = root.file_name().map(|name| name.to_string_lossy().into_owned());
        let folder = cfg.active_mod.clone();
        let mod_row = folder.as_deref().and_then(|name| data.mods.iter().position(|m| m.folder.eq_ignore_ascii_case(name))).map_or(0, |index| index + 1);
        app.insert_resource(super::ActiveMod(folder.clone()))
            .insert_resource(super::SelectedMod(folder))
            .insert_resource(super::Browse { mod_row, ..default() });
    }
    let (frame_width, frame_height) = frame_size;
    let scale = frame_height as f32 / 1080.0;
    app.insert_resource(UiScale(scale))
        .insert_resource(super::stock::StockBox(super::stock::layout::fit_box(Vec2::new(frame_width as f32 / scale, frame_height as f32 / scale))))
        .insert_resource(data)
        .insert_resource(ScreensConfig { own_camera: false, fit_scale: false, start: screen, ..default() })
        .insert_state(AppState::FrontEnd)
        .add_plugins(ScreensPlugin)
        .add_systems(PreStartup, use_installed_preview_fixture)
        .add_systems(Startup, move |mut commands: Commands| {
            let cam = commands
                .spawn((Camera2d, Camera { order: 1, clear_color: ClearColorConfig::None, ..default() }, RenderTarget::from(target.clone()), Msaa::Off))
                .id();
            commands.insert_resource(ScreensCamera(cam));
        })
        .add_systems(Update, announce.in_set(GameSet::Ui));
}

fn use_installed_preview_fixture(mut commands: Commands, mounted: Option<Res<crate::viewer::Mounted>>, mut data: ResMut<ScreenData>) {
    let (Some(mounted), Ok(stem)) = (mounted, std::env::var("STA64_UI_MAP")) else { return };
    let Some(Ok(bytes)) = mounted.0.read(&format!("bzn/{stem}.bzn")) else { return };
    let Ok(source) = sta_bzn::parse(&bytes) else { return };
    let (mut map, _) = sta_content::MapDef::from_bzn(&source, &data.content);
    map.name = stem;
    data.maps = vec![MapEntry::new(map)];
    data.colours = crate::viewer::TeamColours::load(&mounted.0);
    let profile = crate::ui::profile::resolve_profile(Some(&mounted.0), Default::default(), &Default::default());
    commands.insert_resource(profile.theme());
    commands.insert_resource(profile);
}

fn announce(roots: Query<&ScreenRoot, Added<ScreenRoot>>) {
    for r in &roots {
        println!("screen: {:?}", r.0);
    }
}
