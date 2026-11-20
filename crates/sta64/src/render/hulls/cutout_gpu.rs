//! Headless GPU check that a cut-out hull part writes no depth at the texels its alpha test rejects, under
//! a camera with a `DepthPrepass` (the map camera's). Everything is authored: a texture whose left half is
//! opaque and right half has alpha 0, drawn on two hull quads (one a cut-out part, one not) in front of a
//! bright unlit surface. Rule (measured on the original, Armada II 1.1: a pixel rejected by the alpha test
//! writes no depth): the bright surface shows through the cut-out part's rejected texels and nowhere else.
//! Needs `STA64_GPU_TESTS=1` and a hardware adapter; skips otherwise. `STA64_GPU_OUT` keeps the frame.
use super::*;
use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::render::RenderPlugin;
use bevy::window::ExitCondition;
use std::sync::{Arc, Mutex};

const SIZE: u32 = 128;

fn shoot(app: &mut App, target: &Handle<Image>) -> Vec<u8> {
    let out = Arc::new(Mutex::new(None::<Image>));
    let sink = Arc::clone(&out);
    app.world_mut().spawn(Screenshot::image(target.clone())).observe(move |shot: On<ScreenshotCaptured>| {
        *sink.lock().unwrap() = Some(shot.image.clone());
    });
    for _ in 0..60 {
        app.update();
        if out.lock().unwrap().is_some() {
            break;
        }
    }
    let image = out.lock().unwrap().clone().expect("a capture");
    image.data.expect("pixels")
}

/// Left half opaque grey, right half alpha 0.
fn half_cut_texture() -> Image {
    let mut px = Vec::new();
    for _ in 0..2 {
        for x in 0..8u32 {
            px.extend_from_slice(&if x < 4 { [180, 180, 180, 255] } else { [180, 180, 180, 0] });
        }
    }
    Image::new(Extent3d { width: 8, height: 2, depth_or_array_layers: 1 }, TextureDimension::D2, px, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD)
}

fn pixel(frame: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
}

#[test]
fn a_cut_out_part_shows_what_is_behind_it_under_a_depth_prepass() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 to run the GPU test");
        return;
    }
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::log::LogPlugin>(),
    );
    app.add_plugins(HullPlugin);
    app.insert_resource(ClearColor(Color::BLACK));
    app.insert_resource(GlobalAmbientLight { color: Color::WHITE, brightness: 100.0, ..default() });
    let mut img = Image::new_target_texture(SIZE, SIZE, TextureFormat::Rgba8UnormSrgb, None);
    img.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(img);
    let texture = app.world_mut().resource_mut::<Assets<Image>>().add(half_cut_texture());
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::from(target.clone()),
        Msaa::Off,
        Tonemapping::None,
        DepthPrepass,
        Transform::from_xyz(0.0, 0.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    app.world_mut().spawn((DirectionalLight { illuminance: 6000.0, ..default() }, Transform::from_xyz(2.0, 3.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y)));
    // The bright surface: a large unlit green plane behind both parts.
    let plane = app.world_mut().resource_mut::<Assets<Mesh>>().add(Rectangle::new(8.0, 6.0));
    let green = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial { base_color: Color::srgb(0.0, 1.0, 0.0), unlit: true, ..default() });
    app.world_mut().spawn((Mesh3d(plane), MeshMaterial3d(green), Transform::from_xyz(0.0, 0.0, -1.0)));
    // Two hull quads with the same texture: the upper one a cut-out part, the lower one not.
    let quad = app.world_mut().resource_mut::<Assets<Mesh>>().add(Rectangle::new(2.0, 1.0));
    let view = HullView::default();
    let source = StandardMaterial { base_color: Color::WHITE, base_color_texture: Some(texture), cull_mode: None, double_sided: true, ..default() };
    for (y, cut_alpha) in [(0.6, true), (-0.6, false)] {
        let part = HullPart { cut_alpha, ..default() };
        let material = app.world_mut().resource_mut::<Assets<HullMaterial>>().add(hull_material(&source, &part, &view));
        app.world_mut().spawn((Mesh3d(quad.clone()), MeshMaterial3d(material), Transform::from_xyz(0.0, y, 0.0)));
    }
    app.finish();
    app.cleanup();
    for _ in 0..12 {
        app.update();
    }
    let adapter = app
        .world()
        .get_resource::<bevy::render::renderer::RenderAdapterInfo>()
        .cloned()
        .or_else(|| app.get_sub_app(bevy::render::RenderApp).and_then(|r| r.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned()));
    let Some(adapter) = adapter else {
        eprintln!("SKIP: no render adapter available");
        return;
    };
    if crate::bench::frames::is_software_adapter(adapter.device_type, &adapter.name) {
        eprintln!("SKIP: only a software adapter is available ({})", adapter.name);
        return;
    }
    let frame = shoot(&mut app, &target);
    if let Ok(dir) = std::env::var("STA64_GPU_OUT") {
        let _ = std::fs::create_dir_all(&dir);
        let img = Image::new(Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 }, TextureDimension::D2, frame.clone(), TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::default());
        if let Ok(d) = img.try_into_dynamic() {
            let _ = d.to_rgb8().save(format!("{dir}/cutout-prepass.png"));
        }
    }
    // The quads span x -1..1 (pixels 38..90) and y 0.1..1.1 / -1.1..-0.1; the texture's opaque half is
    // the left one. Sample well inside each half.
    let is_green = |p: [u8; 4]| p[1] > 200 && p[0] < 30 && p[2] < 30;
    let (cut_left, cut_right) = (pixel(&frame, 50, 41), pixel(&frame, 78, 41));
    let (opaque_left, opaque_right) = (pixel(&frame, 50, 87), pixel(&frame, 78, 87));
    eprintln!("cut-out part: opaque half {cut_left:?}, rejected half {cut_right:?}; plain part: left {opaque_left:?}, right {opaque_right:?}");
    assert!(!is_green(cut_left), "the opaque half of the cut-out part shows what is behind it: {cut_left:?}");
    assert!(cut_left[0] > 8, "the opaque half of the cut-out part was not drawn: {cut_left:?}");
    assert!(is_green(cut_right), "the rejected half of the cut-out part hides the surface behind it: {cut_right:?}");
    assert!(!is_green(opaque_left) && opaque_left[0] > 8, "a part that is not a cut-out is drawn: {opaque_left:?}");
    assert!(!is_green(opaque_right) && opaque_right[0] > 8, "a part that is not a cut-out ignores the texture's alpha: {opaque_right:?}");
}
