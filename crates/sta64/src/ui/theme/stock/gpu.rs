//! Opt-in windowless GPU frames of the widget gallery, one per UI profile. Rendered to
//! an image target in-process: no window, no screen capture.
//!
//! `STA64_GPU_TESTS=1 STA64_UI_CAPTURE_DIR=/tmp/ui cargo test -p sta64 gallery_renders_on_gpu`
//! writes `gallery-<profile>[-<mount>].png`. With `STA64_TEST_INSTALL` (and optionally
//! `STA64_TEST_MOD`) the install is mounted first, so the kit's text is drawn in the
//! mod's own sprite fonts.

use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::ui::{UiScale, UiTargetCamera};
use bevy::window::ExitCondition;

use super::*;
use crate::app::{GameSet, configure_sets};
use crate::ui::profile::UiProfile;
use crate::ui::theme::{GalleryPlugin, GalleryRoot};

#[derive(Resource)]
struct CaptureCamera(Entity);

fn aim_roots(mut commands: Commands, camera: Res<CaptureCamera>, roots: Query<Entity, (With<GalleryRoot>, Without<UiTargetCamera>)>) {
    for root in &roots {
        commands.entity(root).insert(UiTargetCamera(camera.0));
    }
}

fn capture(profile: UiProfile, vfs: Option<sta_vfs::Vfs>, open_dropdown: bool, label: &str, size: UVec2) {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::log::LogPlugin>(),
    );
    configure_sets(&mut app);
    app.insert_resource(ClearColor(Color::BLACK)).insert_resource(profile).insert_resource(profile.theme()).insert_resource(UiScale(size.y as f32 / 1080.0));
    if let Some(vfs) = vfs {
        app.insert_resource(crate::viewer::Mounted(vfs));
    }
    let mut image = Image::new_target_texture(size.x, size.y, bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, None);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    let camera = app
        .world_mut()
        .spawn((Camera2d, Camera { order: 1, clear_color: ClearColorConfig::Custom(profile.theme().panel), ..default() }, RenderTarget::from(target.clone()), Msaa::Off))
        .id();
    app.insert_resource(CaptureCamera(camera));
    app.add_plugins(GalleryPlugin).add_systems(Update, aim_roots.before(GameSet::Ui));
    app.finish();
    app.cleanup();
    for _ in 0..8 {
        app.update();
    }
    if app.get_sub_app(bevy::render::RenderApp).is_none() {
        eprintln!("SKIP {label}: no render app");
        return;
    }
    if open_dropdown {
        let d = app.world_mut().query_filtered::<Entity, With<Dropdown>>().single(app.world()).unwrap();
        super::dropdown::set_open(app.world_mut(), d, true);
        // Focus the text field too, to show the caret and the focus ring.
        let f = app.world_mut().query_filtered::<Entity, With<TextField>>().single(app.world()).unwrap();
        app.world_mut().resource_mut::<KitFocus>().0 = Some(f);
    }
    for _ in 0..8 {
        app.update();
    }
    let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
    let output = Arc::clone(&frames);
    app.world_mut().spawn(Screenshot::image(target)).observe(move |shot: On<ScreenshotCaptured>| {
        output.lock().unwrap().push(shot.image.clone());
    });
    for _ in 0..60 {
        app.update();
        if !frames.lock().unwrap().is_empty() {
            break;
        }
    }
    let shot = frames.lock().unwrap().first().cloned().unwrap_or_else(|| panic!("{label}: no GPU capture"));
    let dynamic = shot.try_into_dynamic().expect("GPU image conversion");
    let rgba = dynamic.to_rgba8();
    if let Ok(dir) = std::env::var("STA64_UI_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        dynamic.save(std::path::Path::new(&dir).join(format!("gallery-{label}.png"))).unwrap();
    }
    let lit = rgba.pixels().filter(|p| p[0] > 20 || p[1] > 20 || p[2] > 20).count();
    eprintln!("{label}: {lit} non-black pixels");
    assert!(lit > 2000, "{label}: the gallery did not visibly render");
}

#[test]
fn gallery_renders_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 (and STA64_UI_CAPTURE_DIR to keep the frames)");
        return;
    }
    let mounted = || {
        let install = std::env::var("STA64_TEST_INSTALL").ok()?;
        let active_mod = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
        sta_vfs::Vfs::mount_install(std::path::Path::new(&install), active_mod.as_deref()).ok()
    };
    let size = UVec2::new(1600, 900);
    for profile in [UiProfile::Lcars, UiProfile::Armada2, UiProfile::FleetOps] {
        capture(profile, None, true, &format!("{profile:?}").to_lowercase(), size);
    }
    if let Some(vfs) = mounted() {
        let profile = if std::env::var("STA64_UI_PROFILE").as_deref() == Ok("fo") { UiProfile::FleetOps } else { UiProfile::Armada2 };
        capture(profile, Some(vfs), true, &format!("{profile:?}-mounted").to_lowercase(), size);
    }
}
