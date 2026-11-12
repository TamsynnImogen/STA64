//! Optional real-GPU regression for the strategic model/symbol handoff.
use super::*;
use bevy::asset::RenderAssetUsages;
use bevy::camera::{RenderTarget, Viewport};
use bevy::render::RenderPlugin;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::ui::UiScale;
use bevy::window::ExitCondition;
use std::sync::{Arc, Mutex};

const TARGET: UVec2 = UVec2::new(384, 304);
const VIEW_POS: UVec2 = UVec2::new(32, 24);
const VIEW_SIZE: UVec2 = UVec2::new(320, 256);
const LOCAL_CENTRE: Vec3 = Vec3::new(24.0, 0.0, -8.0);
const ROOT_POS: Vec3 = Vec3::new(-30.0, 0.0, 10.0);
const ROOT_SCALE: f32 = 1.25;

#[derive(Clone, Copy)]
struct Stage {
    name: &'static str,
    height: f32,
    scale: f32,
    symbol: bool,
}

fn dominant_pixels(image: &image::RgbaImage, green: bool) -> Vec<Vec2> {
    let mut points = Vec::new();
    for y in VIEW_POS.y..VIEW_POS.y + VIEW_SIZE.y {
        for x in VIEW_POS.x..VIEW_POS.x + VIEW_SIZE.x {
            let p = image.get_pixel(x, y);
            let selected = if green {
                p[1] > 100 && p[1] > p[0].saturating_mul(2) && p[1] > p[2].saturating_mul(2)
            } else {
                // Default scene tonemapping raises magenta's green channel (about
                // 75/255 on NVIDIA), so classify hue by dominance, not a fixed cutoff.
                p[0] > 100 && p[2] > 100 && p[1].saturating_mul(2) < p[0] && p[1].saturating_mul(2) < p[2]
            };
            if selected {
                points.push(Vec2::new(x as f32 + 0.5, y as f32 + 0.5));
            }
        }
    }
    points
}

fn centroid(points: &[Vec2]) -> Vec2 {
    points.iter().copied().sum::<Vec2>() / points.len() as f32
}

#[test]
fn static_offset_model_and_symbol_are_exclusive_on_gpu() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1")
        || std::env::var("STA64_STRATEGIC_TESTS").as_deref() != Ok("1")
    {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 STA64_STRATEGIC_TESTS=1");
        return;
    }

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
            .disable::<bevy::winit::WinitPlugin>(),
    );
    app.init_resource::<Selection>()
        .init_resource::<ButtonInput<KeyCode>>()
        .insert_resource(ClearColor(Color::BLACK))
        .add_plugins(StrategicPlugin);

    let mut image = Image::new_target_texture(
        TARGET.x,
        TARGET.y,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        None,
    );
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    let camera = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            Camera {
                viewport: Some(Viewport {
                    physical_position: VIEW_POS,
                    physical_size: VIEW_SIZE,
                    ..default()
                }),
                ..default()
            },
            RenderTarget::from(target.clone()),
            Msaa::Off,
            Transform::from_xyz(0.0, 170.0, 0.0).looking_to(Vec3::NEG_Y, Vec3::NEG_Z),
        ))
        .id();
    let mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(12.0, 12.0, 12.0));
    let material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.0, 1.0),
            unlit: true,
            ..default()
        });
    let root = app
        .world_mut()
        .spawn((
            Transform::from_translation(ROOT_POS).with_scale(Vec3::splat(ROOT_SCALE)),
            Visibility::Inherited,
            Strategic::new(Shape::Square, Color::srgb(0.0, 1.0, 0.0), 10.0).with_centre(LOCAL_CENTRE),
        ))
        .with_children(|children| {
            children.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::from_translation(LOCAL_CENTRE),
            ));
        })
        .id();

    app.finish();
    app.cleanup();
    for _ in 0..12 {
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
        });
    let Some(adapter) = adapter else {
        eprintln!("SKIP: no render adapter available");
        return;
    };
    if crate::bench::frames::is_software_adapter(adapter.device_type, &adapter.name) {
        eprintln!("SKIP: only a software adapter is available ({})", adapter.name);
        return;
    }
    eprintln!("strategic GPU adapter: {}", adapter.name);

    let captures = Arc::new(Mutex::new(Vec::<Image>::new()));
    let stages = [
        Stage {
            name: "near-small-ui",
            height: 170.0,
            scale: 0.66,
            symbol: false,
        },
        Stage {
            name: "far-small-ui",
            height: 650.0,
            scale: 0.66,
            symbol: true,
        },
        Stage {
            name: "back-small-ui",
            height: 170.0,
            scale: 0.66,
            symbol: false,
        },
        Stage {
            name: "near-large-ui",
            height: 170.0,
            scale: 1.5,
            symbol: false,
        },
        Stage {
            name: "far-large-ui",
            height: 650.0,
            scale: 1.5,
            symbol: true,
        },
        Stage {
            name: "back-large-ui",
            height: 170.0,
            scale: 1.5,
            symbol: false,
        },
    ];
    let mut icon_widths = Vec::new();
    for (index, stage) in stages.iter().enumerate() {
        app.world_mut().resource_mut::<UiScale>().0 = stage.scale;
        app.world_mut().get_mut::<Transform>(camera).unwrap().translation.y = stage.height;
        // Check the first handoff frame as well as later frames used for readback.
        for frame in 0..12 {
            app.update();
            let strategic = app.world().get::<Strategic>(root).unwrap();
            let model_visible = *app.world().get::<Visibility>(root).unwrap() != Visibility::Hidden;
            assert_eq!(model_visible, !stage.symbol, "{} frame {frame}: model gate", stage.name);
            assert_eq!(
                strategic.shown > 0.0,
                stage.symbol,
                "{} frame {frame}: symbol gate",
                stage.name
            );
            assert_eq!(*app.world().resource::<Mode>(), Mode::Auto);
        }
        let output = Arc::clone(&captures);
        app.world_mut()
            .spawn(Screenshot::image(target.clone()))
            .observe(move |shot: On<ScreenshotCaptured>| {
                output.lock().unwrap().push(shot.image.clone());
            });
        for _ in 0..60 {
            app.update();
            if captures.lock().unwrap().len() > index {
                break;
            }
        }
        assert_eq!(
            captures.lock().unwrap().len(),
            index + 1,
            "missing {} capture",
            stage.name
        );
        let captured = captures.lock().unwrap()[index].clone();
        let dynamic = captured.try_into_dynamic().unwrap();
        if let Ok(dir) = std::env::var("STA64_GPU_OUT") {
            std::fs::create_dir_all(&dir).unwrap();
            dynamic
                .save(std::path::Path::new(&dir).join(format!("strategic-{}.png", stage.name)))
                .unwrap();
        }
        let rgba = dynamic.to_rgba8();
        let mut colours = std::collections::HashMap::<[u8; 4], usize>::new();
        for p in rgba.pixels() {
            *colours.entry(p.0).or_default() += 1;
        }
        let mut colours: Vec<_> = colours.into_iter().collect();
        colours.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        eprintln!("{} dominant RGBA: {:?}", stage.name, &colours[..colours.len().min(5)]);
        let green = dominant_pixels(&rgba, true);
        let magenta = dominant_pixels(&rgba, false);
        let (expected, pivot) = {
            let camera_ref = app.world().get::<Camera>(camera).unwrap();
            let camera_tf = app.world().get::<GlobalTransform>(camera).unwrap();
            let root_tf = app.world().get::<GlobalTransform>(root).unwrap();
            (
                camera_ref
                    .world_to_viewport(camera_tf, root_tf.transform_point(LOCAL_CENTRE))
                    .unwrap(),
                camera_ref.world_to_viewport(camera_tf, root_tf.translation()).unwrap(),
            )
        };
        if stage.symbol {
            assert!(
                green.len() > 80,
                "{}: missing green symbol ({})",
                stage.name,
                green.len()
            );
            assert!(
                magenta.is_empty(),
                "{}: model and symbol overlap ({} magenta pixels)",
                stage.name,
                magenta.len()
            );
            let centre = centroid(&green);
            assert!(
                (centre - expected).length() < 3.0,
                "{}: symbol at {centre:?}, expected {expected:?}",
                stage.name
            );
            assert!(
                (centre - pivot).length() > 8.0,
                "{}: symbol followed root pivot",
                stage.name
            );
            let min_x = green.iter().map(|p| p.x).fold(f32::MAX, f32::min);
            let max_x = green.iter().map(|p| p.x).fold(f32::MIN, f32::max);
            icon_widths.push(max_x - min_x);
        } else {
            assert!(
                magenta.len() > 80,
                "{}: missing magenta model ({})",
                stage.name,
                magenta.len()
            );
            assert!(
                green.is_empty(),
                "{}: model and symbol overlap ({} green pixels)",
                stage.name,
                green.len()
            );
            assert!(
                (centroid(&magenta) - expected).length() < 4.0,
                "{}: model centre displaced",
                stage.name
            );
        }
        eprintln!(
            "{}: green={} magenta={} expected={expected:?}",
            stage.name,
            green.len(),
            magenta.len()
        );
    }
    assert_eq!(icon_widths.len(), 2);
    assert!(
        (icon_widths[0] - icon_widths[1]).abs() <= 2.0,
        "UI scale changed logical icon width: {icon_widths:?}"
    );
}
