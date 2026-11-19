//! Opt-in windowless pixel check of the bridge scene on authored art and an authored clip colour
//! (`STA64_GPU_TESTS=1`; `STA64_UI_CAPTURE_DIR` keeps the PNGs, which show authored test art only).
//! The scene is drawn through the real [`drive`] system at the window sizes of the player's playtests
//! (physical pixels, with the UI scale `height / 1080` the game uses) and read back.

use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::render::{RenderPlugin, render_resource::{Extent3d, TextureDimension, TextureFormat}};
use bevy::window::{ExitCondition, WindowResolution};

use super::*;

/// Authored cfg: five pieces around a window-coloured piece at the Federation clip rectangle.
const ART_CFG: &str = "\
f_bridgeBackground = \"f_bridge_panel\"
f_bridgeBackgroundSize = 7
f_bridgeBackground_0 = 0 0 640 10
f_bridgeBackground_1 = 0 10 107 200
f_bridgeBackground_2 = 107 10 113 200
f_bridgeBackground_3 = 220 10 400 200
f_bridgeBackground_4 = 620 10 20 200
f_bridgeBackground_5 = 0 210 213 270
f_bridgeBackground_6 = 213 210 427 270
";

const GREY: [u8; 3] = [200, 200, 200];
const CLIP: [u8; 3] = [0, 255, 0];

fn solid_tga(rgb: [u8; 3]) -> Vec<u8> {
    let mut bytes = vec![0u8; 18];
    bytes[2] = 2;
    bytes[12] = 1;
    bytes[14] = 1;
    bytes[16] = 24;
    bytes[17] = 0x20;
    bytes.extend_from_slice(&[rgb[2], rgb[1], rgb[0]]);
    bytes
}

/// Draws the scene at `physical` and returns the frame.
fn render(physical: UVec2, scale_factor: f32) -> bevy::image::Image {
    let spr: String = (0..7).map(|i| format!("f_bridge_panel.{i} {} 0 0 1 1\n", if i == 3 { "win" } else { "tile" })).collect();
    let (vfs, _tmp) = mount(&[
        ("sprites/gui_global.spr", spr.into_bytes()),
        ("textures/rgb/tile.tga", solid_tga(GREY)),
        ("textures/rgb/win.tga", solid_tga([255, 0, 255])),
    ]);
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .disable::<bevy::gilrs::GilrsPlugin>()
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::log::LogPlugin>(),
    );
    let logical = (physical.as_vec2() / scale_factor).round().as_uvec2();
    app.world_mut().spawn((
        Window { resolution: WindowResolution::new(logical.x, logical.y).with_scale_factor_override(scale_factor), ..default() },
        PrimaryWindow,
    ));
    let (gui, _) = sta_content::GuiCfg::parse("synthetic.cfg", ART_CFG);
    app.insert_resource(UiScale(physical.y as f32 / 1080.0))
        .insert_resource(Mounted(vfs))
        .insert_resource(HudConfig { gui: Some(gui.layout((640, 480))), ..default() })
        .insert_resource(BridgeState { race: Some(1), shown: true, ..default() })
        .init_resource::<Movies>()
        .init_resource::<BridgeArt>()
        .init_resource::<BridgeOverlay>();
    let mut images = app.world_mut().resource_mut::<Assets<Image>>();
    let clip = Image::new_fill(
        Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[CLIP[0], CLIP[1], CLIP[2], 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    let handle = images.add(clip);
    let mut target = Image::new_target_texture(physical.x, physical.y, TextureFormat::Rgba8UnormSrgb, None);
    target.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = images.add(target);
    app.insert_resource(BridgeScreen { image: handle.clone() });
    app.add_systems(Update, drive);
    app.world_mut().spawn((Camera2d, bevy::ui::IsDefaultUiCamera, Camera { clear_color: ClearColorConfig::Custom(Color::srgb(0.0, 0.0, 1.0)), ..default() }, RenderTarget::from(target.clone()), Msaa::Off));
    app.world_mut().resource_mut::<Movies>().queue_for_test(PlayMovie { id: MovieId(1), name: "x".into(), target: MovieTarget::Panel(handle), skippable: false });
    app.finish();
    app.cleanup();
    for _ in 0..12 {
        app.update();
    }
    let frames = Arc::new(Mutex::new(Vec::<bevy::image::Image>::new()));
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
    let frame = frames.lock().unwrap().first().cloned().expect("bridge scene GPU capture");
    frame
}

fn near(p: &image::Rgba<u8>, c: [u8; 3]) -> bool {
    (0..3).all(|i| (p[i] as i32 - c[i] as i32).abs() <= 3)
}

#[test]
fn the_scene_is_centred_seamless_and_the_clip_fills_only_the_opening() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 (and STA64_UI_CAPTURE_DIR to keep the frames)");
        return;
    }
    // The player's window (a 1.25 scale factor on his display), the sheet's aspect, 16:9, a fractional scale, a tall window.
    for (size, factor) in [
        (UVec2::new(3440, 1440), 1.25),
        (UVec2::new(1280, 960), 1.0),
        (UVec2::new(1920, 1080), 1.0),
        (UVec2::new(1001, 777), 1.0),
        (UVec2::new(800, 1280), 1.0),
    ] {
        let rgba = render(size, factor).try_into_dynamic().expect("frame").to_rgba8();
        if let Ok(dir) = std::env::var("STA64_UI_CAPTURE_DIR") {
            std::fs::create_dir_all(&dir).unwrap();
            rgba.save(std::path::Path::new(&dir).join(format!("bridge-scene-{}x{}.png", size.x, size.y))).unwrap();
        }
        let l = layout(size.as_vec2(), Some(1), CANVAS);
        let (g, m) = (l.group, l.movie);
        let (mut cmin, mut cmax, mut clip_n) = ((u32::MAX, u32::MAX), (0u32, 0u32), 0u32);
        for (x, y, p) in rgba.enumerate_pixels() {
            let inside = (x as f32) >= g.x && (x as f32) < g.right() && (y as f32) >= g.y && (y as f32) < g.bottom();
            let in_clip = (x as f32) >= m.x && (x as f32) < m.right() && (y as f32) >= m.y && (y as f32) < m.bottom();
            if near(p, CLIP) {
                clip_n += 1;
                cmin = (cmin.0.min(x), cmin.1.min(y));
                cmax = (cmax.0.max(x), cmax.1.max(y));
            }
            let want = if in_clip { CLIP } else if inside { GREY } else { [0, 0, 0] };
            assert!(near(p, want), "{size}: pixel ({x},{y}) is {p:?}, wanted {want:?} (group {g:?}, clip {m:?})");
        }
        // The clip's pixels are exactly the opening, drawn once.
        assert_eq!((cmin.0 as f32, cmin.1 as f32), (m.x, m.y), "{size}");
        assert_eq!(((cmax.0 + 1) as f32, (cmax.1 + 1) as f32), (m.right(), m.bottom()), "{size}");
        assert_eq!(clip_n as f32, m.w * m.h, "{size}");
    }
}
