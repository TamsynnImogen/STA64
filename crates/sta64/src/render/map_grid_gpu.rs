//! Optional real-GPU frame test of the map grid (#1033): `STA64_GPU_TESTS=1` and a hardware adapter; `STA64_GPU_OUT`
//! keeps the PNGs outside the repository. Authored bounds, an opaque cube standing on the plane, and a blended black
//! quad on the plane standing for the fog layers (the same sort bias and layer as the real ones). As in the map
//! view, the ground camera draws the ground layers (the grid and the fog quad) first and the scene camera draws the
//! cube over its picture. Both look straight down at the middle of the map.
//!
//! Asserted: in the Stock look with MSAA off a line is one pixel wide and adds the dim colour to black, the cube hides
//! the lines under it, the fog quad dims them, the Remaster look's tone mapping leaves them visible and not
//! saturated, and the action window's ground camera (the ground layer only) does not draw the grid. Measured and printed, not asserted: the line's
//! width and value under the default multisampling.
use super::*;
use crate::render::action_camera::{GRID_LAYER, GROUND_LAYER, OVERLAY_LAYER};
use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::render::RenderPlugin;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::ExitCondition;
use std::sync::{Arc, Mutex};


const SIZE: u32 = 512;
/// The row the vertical lines are read on: above the cube and clear of the horizontal lines.
const LINE_ROW: u32 = 80;
/// A row through the cube that no horizontal line crosses.
const CUBE_ROW: u32 = 240;

struct Setup {
    tonemapping: Tonemapping,
    msaa: Msaa,
    /// The ground camera's layers.
    layers: RenderLayers,
}

/// One line of the grid on `LINE_ROW`: where it starts, how wide it is, and its brightest pixel.
#[derive(Debug, Clone, PartialEq)]
struct Mark {
    start: u32,
    width: u32,
    peak: [u8; 3],
}

fn marks(img: &image::RgbaImage, row: u32) -> Vec<Mark> {
    let mut out: Vec<Mark> = Vec::new();
    let mut x = 0;
    while x < img.width() {
        let p = img.get_pixel(x, row).0;
        if p[0].max(p[1]).max(p[2]) > 0 {
            let mut m = Mark { start: x, width: 0, peak: [0; 3] };
            while x < img.width() && img.get_pixel(x, row).0[..3].iter().any(|c| *c > 0) {
                let q = img.get_pixel(x, row).0;
                if q[0] > m.peak[0] {
                    m.peak = [q[0], q[1], q[2]];
                }
                m.width += 1;
                x += 1;
            }
            out.push(m);
        } else {
            x += 1;
        }
    }
    out
}

fn shoot(app: &mut App, target: &Handle<Image>) -> image::RgbaImage {
    let out = Arc::new(Mutex::new(None::<Image>));
    let sink = Arc::clone(&out);
    app.world_mut().spawn(Screenshot::image(target.clone())).observe(move |shot: On<ScreenshotCaptured>| {
        *sink.lock().unwrap() = Some(shot.image.clone());
    });
    for _ in 0..240 {
        app.update();
        if out.lock().unwrap().is_some() {
            break;
        }
    }
    let image = out.lock().unwrap().clone().expect("a capture");
    image.try_into_dynamic().unwrap().to_rgba8()
}

/// One frame of the scene, or `None` without a hardware adapter.
fn frame(name: &str, s: Setup) -> Option<image::RgbaImage> {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .disable::<bevy::winit::WinitPlugin>(),
    );
    add_material(&mut app);
    app.insert_resource(ClearColor(Color::BLACK))
        .insert_resource(GridModes { rts: GridMode::Grid, tactical: GridMode::None })
        .insert_resource(FogBounds { min: Vec2::new(0.0, 0.0), max: Vec2::new(1000.0, 1000.0) })
        .add_systems(Update, update);
    let mut image = Image::new_target_texture(SIZE, SIZE, TextureFormat::Rgba8UnormSrgb, None);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    // Straight down over the middle of the map (render space: x 0..1000, z -1000..0), screen up is -Z.
    let pose = Transform::from_xyz(500.0, 1500.0, -500.0).looking_to(Vec3::NEG_Y, Vec3::NEG_Z);
    app.world_mut().spawn((
        Camera3d::default(),
        s.layers,
        Camera { order: 0, ..default() },
        RenderTarget::from(target.clone()),
        s.msaa,
        s.tonemapping,
        pose,
    ));
    app.world_mut().spawn((
        Camera3d::default(),
        RenderLayers::from_layers(&[0, OVERLAY_LAYER]),
        Camera { order: 1, clear_color: bevy::camera::ClearColorConfig::None, ..default() },
        RenderTarget::from(target.clone()),
        s.msaa,
        s.tonemapping,
        pose,
    ));
    let flat = |c: Color| StandardMaterial { base_color: c, unlit: true, ..default() };
    let cube = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cuboid::new(150.0, 40.0, 150.0));
    let grey = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(flat(Color::srgb(0.5, 0.5, 0.5)));
    app.world_mut().spawn((Mesh3d(cube), MeshMaterial3d(grey), Transform::from_xyz(500.0, 20.0, -500.0)));
    let quad = app.world_mut().resource_mut::<Assets<Mesh>>().add(Plane3d::default().mesh().size(300.0, 1200.0));
    let fog = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
        base_color: Color::srgba(0.0, 0.0, 0.0, 0.7),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        depth_bias: 1.0e6,
        ..default()
    });
    app.world_mut().spawn((Mesh3d(quad), MeshMaterial3d(fog), Transform::from_xyz(800.0, 0.0, -500.0), RenderLayers::layer(GROUND_LAYER)));
    app.finish();
    app.cleanup();
    for _ in 0..12 {
        app.update();
    }
    let adapter = app
        .world()
        .get_resource::<bevy::render::renderer::RenderAdapterInfo>()
        .cloned()
        .or_else(|| app.get_sub_app(bevy::render::RenderApp).and_then(|r| r.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned()))?;
    if crate::bench::frames::is_software_adapter(adapter.device_type, &adapter.name) {
        eprintln!("SKIP: only a software adapter is available ({})", adapter.name);
        return None;
    }
    eprintln!("adapter: {} ({:?})", adapter.name, adapter.device_type);
    let img = shoot(&mut app, &target);
    if let Ok(dir) = std::env::var("STA64_GPU_OUT") {
        std::fs::create_dir_all(&dir).unwrap();
        img.save(std::path::Path::new(&dir).join(format!("map-grid-{name}.png"))).unwrap();
    }
    Some(img)
}

/// The map's background camera draws the ground layers and the grid.
fn both_layers() -> RenderLayers {
    RenderLayers::from_layers(&[GROUND_LAYER, GRID_LAYER])
}

#[test]
fn the_grid_is_one_dim_additive_line_under_the_cube_and_the_fog_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1");
        return;
    }
    let _serial = crate::bench::frames::gpu_serial();
    // Stock look, MSAA off.
    let Some(stock) = frame("stock-msaa-off", Setup { tonemapping: Tonemapping::None, msaa: Msaa::Off, layers: both_layers() }) else { return };
    let lines = marks(&stock, LINE_ROW);
    eprintln!("stock, MSAA off: {lines:?}");
    assert_eq!(lines.len(), 11, "a line every 100 units across the 1000-unit map: {lines:?}");
    assert!(lines.iter().all(|m| m.width == 1), "one device pixel wide: {lines:?}");
    // The colour on black: the constant in framebuffer terms (8 bit), within a few steps.
    let want = GRID_COLOUR.map(|c| c * 255.0);
    // Lines at x = 0..600 are clear of the fog quad (x 650..950); 700, 800 and 900 are under it.
    let (clear, fogged) = lines.split_at(7);
    for m in clear {
        for c in 0..3 {
            assert!((m.peak[c] as f32 - want[c]).abs() <= 3.0, "line value {:?} vs the constant {want:?}", m.peak);
        }
    }
    // (c) The fog quad covers the lines: they are dimmer than the clear ones, and the line at 1000 (clear again) is not.
    for m in &fogged[..3] {
        assert!((m.peak[0] as f32) < clear[0].peak[0] as f32 * 0.8, "a fogged line {m:?} is as bright as a clear one {:?}", clear[0]);
    }
    assert!((fogged[3].peak[0] as i32 - clear[0].peak[0] as i32).abs() <= 3, "the line past the fog is clear: {:?}", fogged[3]);
    // (b) The cube hides the lines under it: one flat grey across its top, no line pixel.
    let top: Vec<[u8; 4]> = (232..280).map(|x| stock.get_pixel(x, CUBE_ROW).0).collect();
    assert!(top[0][0] > 100, "the cube is in view: {:?}", top[0]);
    assert!(top.iter().all(|p| *p == top[0]), "a line shows through or over the cube: {top:?}");
    // (the action window's ground camera) no grid in a camera that draws the ground layer only.
    let Some(action) = frame("action-layers", Setup { tonemapping: Tonemapping::None, msaa: Msaa::Off, layers: RenderLayers::layer(GROUND_LAYER) }) else { return };
    assert!(marks(&action, LINE_ROW).is_empty(), "the grid is on its own layer, which the action window's ground camera does not draw");
    // (e) The default multisampling, Stock look: measured, not asserted.
    let ms = frame("stock-msaa-default", Setup { tonemapping: Tonemapping::None, msaa: Msaa::default(), layers: both_layers() }).expect("same adapter");
    let ms_lines = marks(&ms, LINE_ROW);
    eprintln!("stock, default MSAA ({:?}): {ms_lines:?}", Msaa::default());
    assert!(!ms_lines.is_empty(), "the lines show under the default multisampling");
    // (d) The Remaster look's tone mapping and the default multisampling: visible, not saturated.
    let refit = frame("remaster", Setup { tonemapping: Tonemapping::default(), msaa: Msaa::default(), layers: both_layers() }).expect("same adapter");
    let refit_lines = marks(&refit, LINE_ROW);
    eprintln!("remaster tone mapping: {refit_lines:?}");
    assert!(!refit_lines.is_empty(), "the lines show in the Remaster look");
    assert!(refit_lines.iter().all(|m| m.peak.iter().all(|c| *c < 200)), "the lines stay dim: {refit_lines:?}");
}
