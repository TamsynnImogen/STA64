//! Opt-in, windowless hardware check of the real selection overlay renderer.

use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::ExitCondition;
use sta_sim::command::{Command, LocalRouter};
use sta_sim::{
    ContentKey, EntityId, Fixed, PlayerId, TickOutput, UnitView, WorldSource, WorldView,
};

use super::{BarFill, ModelBounds, OverlayCounts, OverlaysPlugin};
use crate::app::configure_sets;
use crate::render::units::{HullShare, SelectRadius, TeamColour};
use crate::sim_link::{Selection, SimEntities, SimLink};

const SIZE: UVec2 = UVec2::new(1280, 720);
const UNIT: EntityId = EntityId(1);

struct FrozenWorldSource(Arc<WorldView>);

impl WorldSource for FrozenWorldSource {
    fn view(&self) -> Arc<WorldView> {
        self.0.clone()
    }

    fn step(&mut self, _: &[Command]) -> TickOutput {
        TickOutput {
            view: self.0.clone(),
            events: Vec::new(),
        }
    }
}

#[derive(Debug)]
struct Band {
    top: u32,
    bottom: u32,
    width: u32,
}

fn gauge_band(image: &image::RgbaImage, blue: bool) -> Band {
    // The ring is below the actor. Restrict this independent framebuffer
    // measurement to the space above its centre; no projected geometry helper
    // is consulted here.
    let mut rows = Vec::new();
    for y in 260..355 {
        let mut first = None;
        let mut last = 0;
        for x in 400..880 {
            let p = image.get_pixel(x, y);
            let match_colour = if blue {
                p[2] > 95 && p[2] > p[0].saturating_add(45) && p[2] > p[1].saturating_add(20)
            } else {
                p[1] > 95 && p[1] > p[0].saturating_add(45) && p[1] > p[2].saturating_add(20)
            };
            if match_colour {
                first.get_or_insert(x);
                last = x;
            }
        }
        if let Some(first) = first
            && last - first >= 8
        {
            rows.push((y, last - first + 1));
        }
    }
    assert!(
        !rows.is_empty(),
        "missing visible {} gauge pixels",
        if blue { "shield" } else { "hull" }
    );
    let top = rows.first().unwrap().0;
    let bottom = rows.last().unwrap().0;
    assert!(
        bottom - top <= 3,
        "{} gauge is taller than about 2 pixels: {rows:?}",
        if blue { "shield" } else { "hull" }
    );
    Band {
        top,
        bottom,
        width: rows.iter().map(|(_, width)| *width).max().unwrap(),
    }
}

fn capture(app: &mut App, target: &Handle<Image>, label: &str) -> image::RgbaImage {
    let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
    let output = Arc::clone(&frames);
    app.world_mut()
        .spawn(Screenshot::image(target.clone()))
        .observe(move |shot: On<ScreenshotCaptured>| {
            output.lock().unwrap().push(shot.image.clone());
        });
    for _ in 0..60 {
        app.update();
        if !frames.lock().unwrap().is_empty() {
            break;
        }
    }
    let shot = frames
        .lock()
        .unwrap()
        .first()
        .cloned()
        .unwrap_or_else(|| panic!("missing {label} GPU image"));
    let dynamic = shot.try_into_dynamic().expect("GPU image conversion");
    if let Ok(dir) = std::env::var("STA64_OVERLAY_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        dynamic
            .save(std::path::Path::new(&dir).join(format!("overlay-{label}.png")))
            .unwrap();
    }
    dynamic.to_rgba8()
}

#[test]
fn selection_gauges_render_in_screen_pixels_on_hardware() {
    if std::env::var("STA64_OVERLAY_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_OVERLAY_GPU_TESTS=1 to run the overlay GPU test");
        return;
    }

    let unit = UnitView {
        id: UNIT,
        def: ContentKey::NONE,
        owner: PlayerId(1),
        pos: sta_sim::Vec3::ZERO,
        heading: (Fixed::ONE, Fixed::ZERO),
        speed: Fixed::ZERO,
        hull: [Fixed::from_int(1), Fixed::from_int(2)],
        shields: [[Fixed::from_int(1), Fixed::from_int(4)]; 4],
        crew: None,
        energy: None,
        flags: Default::default(),
        progress: None,
    };
    let view = Arc::new(WorldView {
        units: vec![unit],
        ..default()
    });
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .set(RenderPlugin {
                synchronous_pipeline_compilation: true,
                ..default()
            })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::log::LogPlugin>(),
    );
    configure_sets(&mut app);
    app.insert_resource(ClearColor(Color::BLACK))
        .init_resource::<SimEntities>()
        .insert_resource(Selection(vec![UNIT]))
        .insert_resource(SimLink::new(
            Box::new(FrozenWorldSource(view)),
            Box::new(LocalRouter::new()),
            PlayerId(1),
        ))
        .add_plugins(OverlaysPlugin);

    let mut image = Image::new_target_texture(
        SIZE.x,
        SIZE.y,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        None,
    );
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    let camera = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            Camera::default(),
            RenderTarget::from(target.clone()),
            Msaa::Off,
            Transform::from_xyz(0.0, 0.0, 180.0).looking_at(Vec3::ZERO, Vec3::Y),
        ))
        .id();
    let mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(40.0, 24.0, 20.0));
    let material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb(0.5, 0.5, 0.5),
            unlit: true,
            ..default()
        });
    let root = app
        .world_mut()
        .spawn((
            Transform::default(),
            Visibility::Inherited,
            ModelBounds {
                min: Vec3::new(-20.0, -12.0, -10.0),
                max: Vec3::new(20.0, 12.0, 10.0),
            },
            SelectRadius(24.0),
            HullShare(0.5),
            TeamColour(Color::srgb(1.0, 1.0, 0.0)),
        ))
        .with_children(|children| {
            children.spawn((Mesh3d(mesh), MeshMaterial3d(material), Transform::default()));
        })
        .id();
    app.world_mut()
        .resource_mut::<SimEntities>()
        .0
        .insert(UNIT, root);

    app.finish();
    app.cleanup();
    for _ in 0..16 {
        app.update();
    }
    let adapter = app
        .world()
        .get_resource::<bevy::render::renderer::RenderAdapterInfo>()
        .cloned()
        .or_else(|| {
            app.get_sub_app(bevy::render::RenderApp).and_then(|render| {
                render
                    .world()
                    .get_resource::<bevy::render::renderer::RenderAdapterInfo>()
                    .cloned()
            })
        })
        .expect("overlay GPU test requires a render adapter");
    assert!(
        !crate::bench::frames::is_software_adapter(adapter.device_type, &adapter.name),
        "overlay GPU test requires hardware; found {}",
        adapter.name
    );
    eprintln!("overlay GPU adapter: {}", adapter.name);

    assert_eq!(app.world().resource::<OverlayCounts>().bars, 2);
    let mut fills = app.world_mut().query::<&BarFill>();
    let mut shares: Vec<_> = fills.iter(app.world()).map(|fill| fill.share).collect();
    shares.sort_by(f32::total_cmp);
    assert_eq!(shares, [0.25, 0.5]);

    let mut widths = Vec::new();
    for (label, distance) in [("near", 180.0), ("far", 450.0)] {
        app.world_mut()
            .get_mut::<Transform>(camera)
            .unwrap()
            .translation
            .z = distance;
        for _ in 0..12 {
            app.update();
        }
        let rgba = capture(&mut app, &target, label);
        let hull = gauge_band(&rgba, false);
        let shield = gauge_band(&rgba, true);
        assert!(
            shield.bottom + 1 < hull.top,
            "{label}: shield and hull gauges lack a pixel gap: {shield:?} {hull:?}"
        );
        let actor = rgba.get_pixel(SIZE.x / 2, SIZE.y / 2);
        assert!(
            actor[0] > 55
                && actor[0] < 225
                && actor[0].abs_diff(actor[1]) < 12
                && actor[1].abs_diff(actor[2]) < 12,
            "{label}: gray actor missing at centre: {actor:?}"
        );
        for y in 355..365 {
            for x in 610..670 {
                let p = rgba.get_pixel(x, y);
                assert!(
                    p[1] <= p[0].saturating_add(45) && p[2] <= p[0].saturating_add(45),
                    "{label}: gauge crosses actor centre at ({x},{y})"
                );
            }
        }
        widths.push((hull.width, shield.width));
        eprintln!("{label}: hull={hull:?} shield={shield:?}");
    }
    assert!(
        widths[1].0 < widths[0].0 && widths[1].1 < widths[0].1,
        "gauge widths did not shrink with distance: {widths:?}"
    );
}
