//! Opt-in windowless main-menu captures on synthetic content, with an optional
//! mount for the installed fonts. The background scene is not rendered here.

use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::{RenderPlugin, view::screenshot::{Screenshot, ScreenshotCaptured}};
use bevy::window::ExitCondition;

use super::{OptionsTab, StockBox, layout};
use crate::app::{AppState, configure_sets};
use crate::frontend::screens::{ActiveMod, Screen, ScreenData, ScreensCamera, ScreensConfig, ScreensPlugin};
use crate::ui::profile::{UiProfile, UiProfilePlugin};

fn capture(size: UVec2, name: &str, mod_title: Option<&str>, mounted: Option<sta_vfs::Vfs>, screen: Screen) {
    let ingame = name.starts_with("ingame");
    let mut app = App::new();
    app.add_plugins(DefaultPlugins
        .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
        .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
        .disable::<bevy::gilrs::GilrsPlugin>()
        .disable::<bevy::winit::WinitPlugin>()
        .disable::<bevy::log::LogPlugin>());
    configure_sets(&mut app);
    let scale = if name.contains("compact") { 1.0 } else { size.y as f32 / 1080.0 };
    let mut data = ScreenData::fake();
    if let Some(title) = mod_title {
        data.mods[0].name = title.into();
    }
    app.insert_resource(UiProfile::Armada2)
        .insert_resource(UiScale(scale))
        .insert_resource(StockBox(layout::fit_box(Vec2::new(size.x as f32, size.y as f32) / scale)))
        .insert_resource(data)
        .insert_resource(ActiveMod(mod_title.map(|_| "fx_balance".into())))
        .insert_resource(OptionsTab(if screen == Screen::Settings { 2 } else { 0 }))
        .insert_resource(ScreensConfig { own_camera: false, fit_scale: false, start: screen, ..default() })
        .insert_state(if ingame { AppState::InGame } else { AppState::FrontEnd })
        .add_plugins(UiProfilePlugin);
    if let Some(vfs) = mounted {
        app.insert_resource(crate::viewer::Mounted(vfs));
    }
    let mut image = Image::new_target_texture(size.x, size.y, bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, None);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    let camera = app.world_mut().spawn((Camera2d, bevy::ui::IsDefaultUiCamera,
        Camera { clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() },
        RenderTarget::from(target.clone()), Msaa::Off)).id();
    app.insert_resource(ScreensCamera(camera)).add_plugins(ScreensPlugin);
    if ingame {
        app.add_plugins(crate::ui::ingame_menu::InGameMenuPlugin)
            .insert_resource(crate::ui::ingame_menu::InGameMenu { open: true, page: crate::ui::ingame_menu::MenuPage::Options });
    }
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
    if screen == Screen::Main {
        // A short active-mod name has fewer gold glyph pixels than the old fixed title.
        assert!(gold > 800, "{name}: missing title/divider ({gold} gold pixels)");
        if size == UVec2::new(1920, 1080) {
            let title_ink = rgba.enumerate_pixels().filter(|(x, y, p)|
                (150..1780).contains(x) && (150..334).contains(y)
                    && p[0] > 150 && p[1] > 80 && p[2] < 120).count();
            assert!(title_ink > 500, "{name}: missing active-mod title ({title_ink} gold pixels)");
            assert!(!rgba.enumerate_pixels().any(|(x, y, p)|
                (1784..1920).contains(&x) && (150..342).contains(&y)
                    && p[0] > 150 && p[1] > 80 && p[2] < 120),
                "{name}: title ink extends past the title region");
            // Check the visible ink, not only its parent rectangle: the reference
            // puts the first caption near y=376, below a clear title/divider gap.
            let first_ink = rgba.enumerate_pixels().filter(|(x, y, p)|
                (165..700).contains(x) && (360..430).contains(y)
                    && p[0] > 140 && p[1] > 140 && p[2] > 140)
                .map(|(_, y, _)| y).min().expect("first main-menu caption");
            assert!((370..=385).contains(&first_ink), "{name}: first caption ink starts at {first_ink}");
            assert!(!rgba.enumerate_pixels().any(|(x, y, p)|
                (150..1780).contains(&x) && (334..342).contains(&y)
                    && p[0] > 150 && p[1] > 80 && p[2] < 120),
                "{name}: title ink reaches the divider gap");
        }
    } else {
        assert_eq!(app.world().resource::<OptionsTab>().0, 2);
        let controls = app.world_mut().query::<&crate::frontend::screens::view::SettingToggle>()
            .iter(app.world()).count();
        assert_eq!(controls, 5);
        let mut areas = app.world_mut().query::<(&super::GraphicsPart, &ComputedNode, &bevy::ui::UiGlobalTransform)>();
        let rects: Vec<_> = areas.iter(app.world()).map(|(p, node, transform)| (*p, Rect::from_center_size(transform.translation, node.size()))).collect();
        let bounds = |part| rects.iter().find(|(p, _)| *p == part).expect("graphics region").1;
        let toggles = bounds(super::GraphicsPart::Toggles);
        let detail = bounds(super::GraphicsPart::Detail);
        let footer = bounds(super::GraphicsPart::Footer);
        assert!(toggles.max.y <= detail.min.y + 1.0, "{name}: options overlap detail controls");
        assert!(detail.max.y <= footer.min.y + 1.0, "{name}: detail controls overlap footer");
        let bottom = if ingame { 532.0 } else { 560.0 } / 600.0 * size.y as f32;
        assert!(footer.max.y < bottom, "{name}: graphics controls extend into Previous Menu ({footer:?}, {bottom})");

        let body = layout::region(layout::Slot::OptionsBody).px(app.world().resource::<StockBox>().0);
        let inset = (Vec2::new(size.x as f32, size.y as f32) / scale - app.world().resource::<StockBox>().0) * 0.5;
        let start = (body.min + inset) * scale;
        let end = (body.max + inset) * scale;
        let ink = rgba.enumerate_pixels().filter(|(x,y,p)|
            *x >= start.x as u32 && *x < end.x as u32 && *y >= start.y as u32 && *y < end.y as u32
                && p[0] > 140 && p[1] > 140 && p[2] > 140).count();
        assert!(ink > 200, "{name}: graphics controls have no visible label ink ({ink})");
    }
    if let Ok(dir) = std::env::var("STA64_UI_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        let prefix = if screen == Screen::Main { "main-menu" } else { "graphics-options" };
        dynamic.save(std::path::Path::new(&dir).join(format!("{prefix}-{name}.png"))).unwrap();
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
        capture(size, name, None, None, Screen::Main);
    }
    capture(UVec2::new(1920, 1080), "short-mod", Some("Roots"), None, Screen::Main);
    capture(UVec2::new(1920, 1080), "long-mod", Some("Expanded Fleet Operations and Starship Construction Project"), None, Screen::Main);
    if let Ok(install) = std::env::var("STA64_TEST_INSTALL") {
        let active_mod = std::env::var("STA64_TEST_MOD").ok().filter(|name| !name.is_empty());
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), active_mod.as_deref())
            .expect("main-menu capture install mount");
        capture(UVec2::new(1920, 1080), "mounted", None, Some(vfs), Screen::Main);
    }
}


#[test]
fn graphics_options_render_without_a_window() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 and optionally STA64_UI_CAPTURE_DIR");
        return;
    }
    for (size, name) in [
        (UVec2::new(1920, 1080), "16x9"),
        (UVec2::new(800, 600), "compact"),
        (UVec2::new(800, 600), "ingame-compact"),
        (UVec2::new(1920, 1080), "ingame"),
        (UVec2::new(1280, 960), "4x3"),
        (UVec2::new(3440, 1440), "ultrawide"),
    ] {
        capture(size, name, None, None, Screen::Settings);
    }
    if let Ok(install) = std::env::var("STA64_TEST_INSTALL") {
        let active_mod = std::env::var("STA64_TEST_MOD").ok().filter(|name| !name.is_empty());
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&install), active_mod.as_deref())
            .expect("graphics capture install mount");
        capture(UVec2::new(1920, 1080), "mounted", None, Some(vfs), Screen::Settings);
    }
}
