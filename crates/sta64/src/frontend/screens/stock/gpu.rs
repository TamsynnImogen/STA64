//! Opt-in windowless main-menu captures on synthetic content, with an optional
//! mount for the installed fonts. The background scene is not rendered here.

use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::{RenderPlugin, view::screenshot::{Screenshot, ScreenshotCaptured}};
use bevy::window::ExitCondition;

use super::{StockBox, layout};
use crate::app::{AppState, configure_sets};
use crate::frontend::screens::{Screen, ScreenData, ScreensCamera, ScreensConfig, ScreensPlugin};
use crate::ui::profile::{UiProfile, UiProfilePlugin};

fn capture(size: UVec2, name: &str, mounted: Option<sta_vfs::Vfs>) {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins
        .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
        .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
        .disable::<bevy::gilrs::GilrsPlugin>()
        .disable::<bevy::winit::WinitPlugin>()
        .disable::<bevy::log::LogPlugin>());
    configure_sets(&mut app);
    let scale = size.y as f32 / 1080.0;
    app.insert_resource(UiProfile::Armada2)
        .insert_resource(UiScale(scale))
        .insert_resource(StockBox(layout::fit_box(Vec2::new(size.x as f32, size.y as f32) / scale)))
        .insert_resource(ScreenData::fake())
        .insert_resource(ScreensConfig { own_camera: false, fit_scale: false, start: Screen::Main, ..default() })
        .insert_state(AppState::FrontEnd)
        .add_plugins(UiProfilePlugin);
    if let Some(vfs) = mounted {
        app.insert_resource(crate::viewer::Mounted(vfs));
    }
    let mut image = Image::new_target_texture(size.x, size.y, bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, None);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    let camera = app.world_mut().spawn((Camera2d,
        Camera { clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() },
        RenderTarget::from(target.clone()), Msaa::Off)).id();
    app.insert_resource(ScreensCamera(camera)).add_plugins(ScreensPlugin);
    app.finish();
    app.cleanup();
    for _ in 0..24 {
        app.update();
    }
    let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
    let output = Arc::clone(&frames);
    app.world_mut().spawn(Screenshot::image(target)).observe(move |shot: On<ScreenshotCaptured>| {
        output.lock().unwrap().push(shot.image.clone());
    });
    for _ in 0..90 {
        app.update();
        if !frames.lock().unwrap().is_empty() {
            break;
        }
    }
    let image = frames.lock().unwrap().first().cloned().expect("main-menu GPU capture");
    let dynamic = image.try_into_dynamic().expect("main-menu GPU image conversion");
    let rgba = dynamic.to_rgba8();
    let lit = rgba.pixels().filter(|p| p[0] > 30 || p[1] > 30 || p[2] > 30).count();
    let gold = rgba.pixels().filter(|p| p[0] > 150 && p[1] > 80 && p[2] < 120).count();
    assert!(lit > 2500, "{name}: missing menu text ({lit} visible pixels)");
    assert!(gold > 300, "{name}: missing title/divider ({gold} gold pixels)");
    if let Ok(dir) = std::env::var("STA64_UI_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        dynamic.save(std::path::Path::new(&dir).join(format!("main-menu-{name}.png"))).unwrap();
    }
}

#[test]
fn main_menu_renders_without_a_window() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 and optionally STA64_UI_CAPTURE_DIR");
        return;
    }
    for (size, name) in [
        (UVec2::new(1920, 1080), "16x9"),
        (UVec2::new(1280, 960), "4x3"),
        (UVec2::new(3440, 1440), "ultrawide"),
    ] {
        capture(size, name, None);
    }
    if let Ok(install) = std::env::var("STA64_TEST_INSTALL") {
        let active_mod = std::env::var("STA64_TEST_MOD").ok().filter(|name| !name.is_empty());
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), active_mod.as_deref())
            .expect("main-menu capture install mount");
        capture(UVec2::new(1920, 1080), "mounted", Some(vfs));
    }
}
