//! Opt-in, windowless frame test of the bubble (`STA64_GPU_TESTS=1`, needs a hardware adapter):
//! the inner pass masks the outer one, nothing is added over hull pixels, and the ring is the
//! 1.0 to 1.1 annulus. Authored scene, the stand-in ellipsoid; no game files.

use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::ExitCondition;
use sta_sim::command::{Command, LocalRouter};
use sta_sim::{ContentKey, EntityId, Fixed, PlayerId, TickOutput, UnitView, WorldSource, WorldView};

use crate::app::configure_sets;
use crate::input::hover::HoveredUnit;
use crate::render::overlays::{ModelBounds, OverlaysPlugin};
use crate::render::units::{HullShare, SelectRadius, TeamColour};
use crate::sim_link::{Selection, SimEntities, SimLink};
use crate::ui::profile::UiProfile;

const SIZE: UVec2 = UVec2::new(1280, 720);
const UNIT: EntityId = EntityId(1);

struct Frozen(Arc<WorldView>);

impl WorldSource for Frozen {
    fn view(&self) -> Arc<WorldView> {
        self.0.clone()
    }
    fn step(&mut self, _: &[Command]) -> TickOutput {
        TickOutput { view: self.0.clone(), events: Vec::new() }
    }
}

fn capture(app: &mut App, target: &Handle<Image>) -> image::RgbaImage {
    let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
    let output = Arc::clone(&frames);
    app.world_mut().spawn(Screenshot::image(target.clone())).observe(move |shot: On<ScreenshotCaptured>| {
        output.lock().unwrap().push(shot.image.clone());
    });
    for _ in 0..60 {
        app.update();
        if !frames.lock().unwrap().is_empty() {
            break;
        }
    }
    let shot = frames.lock().unwrap().first().cloned().expect("a GPU image");
    shot.try_into_dynamic().expect("GPU image conversion").to_rgba8()
}

/// The first and last x of the row whose red channel is at least `min`.
fn extent(img: &image::RgbaImage, y: u32, min: u8) -> (u32, u32) {
    let lit: Vec<u32> = (0..SIZE.x).filter(|x| img.get_pixel(*x, y)[0] >= min).collect();
    (*lit.first().expect("lit pixels on the row"), *lit.last().unwrap())
}

#[test]
fn the_inner_pass_masks_the_ring_and_nothing_is_added_over_the_hull() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 to run the bubble frame test");
        return;
    }
    let unit = UnitView {
        id: UNIT,
        def: ContentKey::NONE,
        owner: PlayerId(1),
        pos: sta_sim::Vec3::ZERO,
        heading: (Fixed::ONE, Fixed::ZERO),
        speed: Fixed::ZERO,
        hull: [Fixed::from_int(1), Fixed::from_int(1)],
        shields: [[Fixed::ZERO, Fixed::ZERO]; 4],
        crew: None,
        energy: None,
        autonomy: None,
        flags: Default::default(),
        progress: None,
    };
    let view = Arc::new(WorldView { units: vec![unit], ..default() });
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::log::LogPlugin>(),
    );
    configure_sets(&mut app);
    app.insert_resource(ClearColor(Color::BLACK))
        .insert_resource(UiProfile::Armada2)
        .init_resource::<SimEntities>()
        .init_resource::<HoveredUnit>()
        .insert_resource(Selection(vec![]))
        .insert_resource(SimLink::new(Box::new(Frozen(view)), Box::new(LocalRouter::new()), PlayerId(1)))
        .add_plugins(OverlaysPlugin);
    let mut image = Image::new_target_texture(SIZE.x, SIZE.y, bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, None);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    app.world_mut().spawn((
        Camera3d::default(),
        Camera::default(),
        RenderTarget::from(target.clone()),
        Msaa::Off,
        Transform::from_xyz(0.0, 0.0, 180.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cuboid::new(40.0, 24.0, 20.0));
    let material = app.world_mut().resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial { base_color: Color::srgb(0.5, 0.5, 0.5), unlit: true, ..default() });
    let root = app
        .world_mut()
        .spawn((
            Transform::default(),
            Visibility::Inherited,
            ModelBounds { min: Vec3::new(-20.0, -12.0, -10.0), max: Vec3::new(20.0, 12.0, 10.0) },
            SelectRadius(24.0),
            HullShare(1.0),
            TeamColour(Color::srgb(0.9, 0.9, 0.9)),
        ))
        .with_children(|c| {
            c.spawn((Mesh3d(mesh), MeshMaterial3d(material), Transform::default()));
        })
        .id();
    app.world_mut().resource_mut::<SimEntities>().0.insert(UNIT, root);
    app.finish();
    app.cleanup();
    for _ in 0..16 {
        app.update();
    }
    let adapter = app
        .world()
        .get_resource::<bevy::render::renderer::RenderAdapterInfo>()
        .cloned()
        .or_else(|| app.get_sub_app(bevy::render::RenderApp).and_then(|r| r.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned()))
        .expect("a render adapter");
    if crate::bench::frames::is_software_adapter(adapter.device_type, &adapter.name) {
        eprintln!("SKIP: the bubble frame test needs a hardware adapter; found {}", adapter.name);
        return;
    }
    eprintln!("bubble GPU adapter: {}", adapter.name);

    let frame = |app: &mut App, selected: bool, hovered: bool, team: Color| {
        app.world_mut().resource_mut::<Selection>().0 = if selected { vec![UNIT] } else { vec![] };
        app.world_mut().insert_resource(HoveredUnit(hovered.then_some(UNIT)));
        app.world_mut().entity_mut(root).insert(TeamColour(team));
        for _ in 0..8 {
            app.update();
        }
        capture(app, &target)
    };
    let base = frame(&mut app, false, false, Color::WHITE);
    // The disc alone: a black team colour leaves the ring black.
    let disc = frame(&mut app, true, false, Color::BLACK);
    let selected = frame(&mut app, true, false, Color::srgb(0.9, 0.9, 0.9));
    let hover = frame(&mut app, false, true, Color::srgb(0.9, 0.9, 0.9));
    let both = frame(&mut app, true, true, Color::srgb(0.9, 0.9, 0.9));

    // Nothing is added over hull pixels, in any state.
    // The hull and shield gauges of today's overlay (replaced by the hover readout in another part
    // of #1046) sit on the top rows of the hull's box: those rows are left out.
    let top = base.enumerate_pixels().filter(|(_, _, p)| p[0] > 30).map(|(_, y, _)| y).min().unwrap();
    let mut hull_pixels = 0;
    for (x, y, p) in base.enumerate_pixels() {
        if p[0] > 30 && y > top + 14 {
            hull_pixels += 1;
            for (name, f) in [("disc", &disc), ("selected", &selected), ("hover", &hover), ("both", &both)] {
                assert_eq!(f.get_pixel(x, y), p, "{name}: the bubble added light over a hull pixel at ({x},{y})");
            }
        }
    }
    assert!(hull_pixels > 5000, "the hull is in the frame: {hull_pixels}");

    let y = SIZE.y / 2;
    let inner_probe = SIZE.x / 2 + 150;
    // The disc is a uniform grey about the factor of the ally colour: 0.6 * 0.25 * 0.625 in display units.
    let d = disc.get_pixel(inner_probe, y);
    assert!((18..=30).contains(&d[0]) && d[0].abs_diff(d[2]) < 3, "disc grey: {d:?}");
    // The inner pass hides the ring inside it: selected equals disc-only there, and the hover-only
    // frame is black there although its shell is full white.
    assert_eq!(selected.get_pixel(inner_probe, y), d, "the inner pass masks the shell");
    assert_eq!(both.get_pixel(inner_probe, y), d);
    assert_eq!(hover.get_pixel(inner_probe, y)[0], 0, "hovered only: nothing inside the ring");
    // The widths: the ring's outer edge over the disc's edge is the 1.1 scale.
    let (dl, dr) = extent(&disc, y, 8);
    let (hl, hr) = extent(&hover, y, 100);
    let ratio = (hr - hl) as f32 / (dr - dl) as f32;
    assert!((1.07..=1.13).contains(&ratio), "outer over inner width {ratio} ({hl}..{hr} against {dl}..{dr})");
    // Selected: the ring is brighter than the disc, dim; hovered: white.
    let ring = selected.get_pixel(dr + 2, y);
    assert!(ring[0] > d[0] && ring[0] < 70, "the dim ring: {ring:?} against {d:?}");
    assert!(hover.get_pixel(hr - 2, y)[0] > 180, "the hover ring is white");
}
