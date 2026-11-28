//! Windowless real-adapter check for the mounted Armada II `warp_sprite`.
//! Run with `STA64_GPU_TESTS=1 STA64_EMISSIVE_INSTALL=/path/to/ArmadaII-or-mod`;
//! `STA64_GPU_OUT` optionally keeps the active and cleared frames.

use super::{EXIT_PULSE_SECONDS, SPRITE, register};
use crate::clock::{AnimClock, ClockMode};
use crate::render::sprites::SpriteTables;
use crate::render::textures::TextureCache;
use crate::sim_link::{ContentRes, SimLink};
use crate::viewer::Mounted;
use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::ExitCondition;
use sta_sim::command::LocalRouter;
use sta_sim::{
    EntityId, Fixed, PlayerId, TickOutput, UnitView, WorldSource, WorldView, view::{UnitFlags, WarpPhase, WarpRow},
};
use std::sync::{Arc, Mutex};

const ID: EntityId = EntityId(1);
const SIZE: u32 = 256;

/// Minimal live source: it exposes the same phase metadata as the engine while moving
/// the craft through the flash's native reach, then beyond it.
struct WarpSource {
    view: Arc<WorldView>,
    stage: u8,
}

impl WorldSource for WarpSource {
    fn view(&self) -> Arc<WorldView> {
        self.view.clone()
    }

    fn step(&mut self, _: &[sta_sim::command::Command]) -> TickOutput {
        self.stage = self.stage.saturating_add(1);
        let mut next = (*self.view).clone();
        next.tick += 1;
        // Clear the entry effect by moving well beyond its reach, then hold the
        // same pose for the cruise/normal comparison below.
        next.units[0].pos.z = Fixed::from_int(-80 * i32::from(self.stage.min(3)));
        next.units[0].speed = Fixed::from_int(if (2..=3).contains(&self.stage) {
            100
        } else {
            0
        });
        let phase = match self.stage {
            1..=3 => Some(WarpPhase::AtWarp),
            4 => Some(WarpPhase::Leaving),
            _ => None,
        };
        next.warp_rows =phase.map(|phase| WarpRow { id: ID, phase, ramp_start: next.tick, ramp_len: 10, stretch: Fixed::from_int(3) }).into_iter().collect();
        self.view = Arc::new(next);
        TickOutput {
            view: self.view.clone(),
            events: Vec::new(),
        }
    }
}

fn frame(app: &mut App, target: &Handle<Image>) -> image::RgbaImage {
    let capture = Arc::new(Mutex::new(None::<Image>));
    let sink = capture.clone();
    app.world_mut()
        .spawn(Screenshot::image(target.clone()))
        .observe(move |shot: On<ScreenshotCaptured>| {
            *sink.lock().unwrap() = Some(shot.image.clone());
        });
    for _ in 0..120 {
        app.update();
        if capture.lock().unwrap().is_some() {
            break;
        }
    }
    capture
        .lock()
        .unwrap()
        .clone()
        .expect("GPU screenshot")
        .try_into_dynamic()
        .unwrap()
        .to_rgba8()
}

fn lit_pixels(image: &image::RgbaImage) -> usize {
    image
        .pixels()
        .filter(|p| p[0] > 5 || p[1] > 5 || p[2] > 5)
        .count()
}

#[test]
fn mounted_warp_sprite_contributes_pixels_then_disappears() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 and STA64_EMISSIVE_INSTALL");
        return;
    }
    let Ok(root) = std::env::var("STA64_EMISSIVE_INSTALL") else {
        eprintln!("SKIP: STA64_EMISSIVE_INSTALL names no Armada II install or mod");
        return;
    };
    let path = std::path::Path::new(&root);
    let (root, active) = if path
        .parent()
        .and_then(std::path::Path::file_name)
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|name| name.eq_ignore_ascii_case("mods"))
    {
        (
            path.ancestors().nth(3).expect("install/Data/Mods/mod"),
            path.file_name().and_then(std::ffi::OsStr::to_str),
        )
    } else {
        (path, None)
    };
    let _serial = crate::bench::frames::gpu_serial();
    let vfs = sta_vfs::Vfs::mount_install(root, active).expect("mount Armada II art");
    let table = sta_assets::load_sprite_table(&vfs);
    let sprite = table.sprite(SPRITE).expect("mounted warp_sprite entry");
    let (texture, _) =
        sta_assets::find_texture(&vfs, &sprite.texture).expect("mounted warp sprite texture");
    eprintln!("Mounted warp_sprite from {texture}");
    assert_eq!(
        sta_assets::spr_anim::sprite_blend(&sprite.material),
        sta_assets::spr_anim::SpriteBlend::Add
    );

    let mut def = sta_content::UnitDef::new("warp_flash_fixture", "frigate");
    def.radius = Fixed::from_int(10);
    def.movement.max_speed = Fixed::from_int(10);
    def.movement.warp_speed = Fixed::from_int(100);
    let db = sta_content::ContentDb::builder().unit(def).build().unwrap();
    let key = db.key("warp_flash_fixture").unwrap();
    let unit = UnitView {
        id: ID,
        def: key,
        owner: PlayerId(0),
        pos: sta_core::Vec3::ZERO,
        heading: (Fixed::ZERO, Fixed::ONE),
        speed: Fixed::ZERO,
        hull: [Fixed::from_int(100); 2],
        shields: [[Fixed::from_int(100); 2]; 4],
        flags: UnitFlags::SELECTABLE,
        progress: None,
        crew: None,
        energy: None,
        autonomy: None,
    };
    let initial = Arc::new(WorldView {
        units: vec![unit],
        ..default()
    });
    let source = WarpSource {
        view: initial,
        stage: 0,
    };

    let mut app = App::new();
    app.set_error_handler(bevy::ecs::error::panic);
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
    crate::app::configure_sets(&mut app);
    app.insert_resource(ClearColor(Color::BLACK))
        .insert_resource(AnimClock::new(ClockMode::Bench { step: 0.0 }, 0))
        .insert_resource(ContentRes(Arc::new(db)))
        .insert_resource(SpriteTables(Arc::new(table)))
        .insert_resource(Mounted(vfs))
        .init_resource::<TextureCache>()
        .insert_resource(SimLink::new(
            Box::new(source),
            Box::new(LocalRouter::new()),
            PlayerId(0),
        ));
    register(&mut app);
    let mut target_image = Image::new_target_texture(
        SIZE,
        SIZE,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        None,
    );
    target_image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(target_image);
    let camera = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            RenderTarget::from(target.clone()),
            Msaa::Off,
            Tonemapping::None,
            Transform::from_xyz(0.0, 0.0, 180.0).looking_at(Vec3::ZERO, Vec3::Y),
        ))
        .id();
    app.finish();
    app.cleanup();

    // Let track remember the initial non-warp phase, then enter warp and capture its active frame.
    app.update();
    use bevy::render::renderer::RenderAdapterInfo;
    let adapter = app
        .world()
        .get_resource::<RenderAdapterInfo>()
        .or_else(|| {
            app.get_sub_app(bevy::render::RenderApp)
                .and_then(|app| app.world().get_resource::<RenderAdapterInfo>())
        })
        .expect("windowless GPU adapter");
    eprintln!(
        "GPU adapter: {} ({:?}, {:?})",
        adapter.name, adapter.device_type, adapter.backend
    );
    assert!(!crate::bench::frames::is_software_adapter(
        adapter.device_type,
        &adapter.name
    ));
    for _ in 0..2 {
        app.world_mut()
            .resource_mut::<SimLink>()
            .tick()
            .expect("scripted warp tick");
        app.update();
    }
    let active = frame(&mut app, &target);
    eprintln!("Active warp flash lit {} pixels", lit_pixels(&active));
    assert!(
        lit_pixels(&active) > 20,
        "active native warp flash contributed no pixels"
    );

    app.world_mut()
        .resource_mut::<SimLink>()
        .tick()
        .expect("scripted warp-out tick");
    for _ in 0..3 {
        app.update();
    }
    let gone = frame(&mut app, &target);
    assert_eq!(
        lit_pixels(&gone),
        0,
        "warp flash remained after leaving its reach"
    );
    assert_eq!(app.world().resource::<SimLink>().warp_phase(ID), Some(true));

    // The stage-3 pose remains fixed as the scripted phase drops out. A simple
    // unlit hull makes the real WarpHull transform visible without a mounted model.
    let root = app
        .world_mut()
        .spawn((
            crate::sim_link::SimEntity(ID),
            Transform::IDENTITY,
            Visibility::default(),
        ))
        .id();
    let mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(6.0, 4.0, 16.0));
    let material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb(0.2, 0.8, 1.0),
            unlit: true,
            ..default()
        });
    app.world_mut().spawn((
        super::WarpHull { pivot_z: 0.0 },
        Transform::IDENTITY,
        Visibility::Inherited,
        ChildOf(root),
        Mesh3d(mesh),
        MeshMaterial3d(material),
    ));
    *app.world_mut().get_mut::<Transform>(camera).unwrap() =
        Transform::from_xyz(100.0, 0.0, 0.0).looking_at(Vec3::ZERO, Vec3::Y);
    for _ in 0..3 {
        app.update();
    }
    let cruise = frame(&mut app, &target);
    app.world_mut()
        .resource_mut::<SimLink>()
        .tick()
        .expect("scripted warp-out tick");
    assert_eq!(
        app.world().resource::<SimLink>().warp_phase(ID),
        Some(false)
    );
    app.update();
    let exit_anchor = {
        let world = app.world_mut();
        let mut query = world.query::<&super::WarpFlash>();
        query
            .iter(world)
            .find(|flash| flash.exit_started.is_some())
            .expect("warp-out created an exit pulse")
            .anchor
    };
    // The exit pulse is anchored at the ship's exit point. Center the camera
    // there and hide the hull so its silhouette cannot contribute pixels.
    *app.world_mut().get_mut::<Visibility>(root).unwrap() = Visibility::Hidden;
    *app.world_mut().get_mut::<Transform>(camera).unwrap() =
        Transform::from_xyz(exit_anchor.x + 100.0, exit_anchor.y, exit_anchor.z)
            .looking_at(exit_anchor, Vec3::Y);
    let lifetime_ticks = (EXIT_PULSE_SECONDS * f64::from(sta_sim::TICK_HZ)).ceil() as u32;
    let pulse_capture_ticks = (lifetime_ticks / 4).max(1);
    for _ in 0..pulse_capture_ticks {
        app.world_mut()
            .resource_mut::<SimLink>()
            .tick()
            .expect("advance exit pulse");
        app.update();
    }
    let exit = frame(&mut app, &target);
    eprintln!("Exit warp flash lit {} pixels", lit_pixels(&exit));
    assert!(lit_pixels(&exit) > 20, "exit pulse contributed no pixels");

    // Keep the pulse visible only for its fixed duration, then pass at least
    // 15 world ticks before the normal hull silhouette capture.
    let normal_capture_ticks = 15_u32.max(lifetime_ticks + 1);
    for _ in pulse_capture_ticks..normal_capture_ticks {
        app.world_mut()
            .resource_mut::<SimLink>()
            .tick()
            .expect("expire exit pulse");
        app.update();
    }
    let exit_gone = frame(&mut app, &target);
    assert_eq!(
        lit_pixels(&exit_gone),
        0,
        "exit pulse outlived its fixed lifetime"
    );
    *app.world_mut().get_mut::<Visibility>(root).unwrap() = Visibility::Inherited;
    *app.world_mut().get_mut::<Transform>(camera).unwrap() =
        Transform::from_xyz(100.0, 0.0, 0.0).looking_at(Vec3::ZERO, Vec3::Y);
    for _ in 0..3 {
        app.update();
    }
    let normal = frame(&mut app, &target);
    assert_eq!(app.world().resource::<SimLink>().warp_phase(ID), None);
    let width = |image: &image::RgbaImage| {
        let xs = image
            .pixels()
            .enumerate()
            .filter_map(|(i, p)| (p[0] > 5 || p[1] > 5 || p[2] > 5).then_some((i as u32) % SIZE));
        let (min, max) = xs.fold((SIZE, 0), |(min, max), x| (min.min(x), max.max(x)));
        max.saturating_sub(min)
    };
    let cruise_width = width(&cruise);
    let normal_width = width(&normal);
    eprintln!("WarpHull side-view widths: cruise={cruise_width}px normal={normal_width}px");
    assert!(
        cruise_width > 0 && normal_width > 0,
        "WarpHull comparison was not visible"
    );
    assert!(
        cruise_width > 2 * normal_width,
        "cruise hull was not visibly elongated"
    );
    if let Ok(dir) = std::env::var("STA64_GPU_OUT") {
        std::fs::create_dir_all(&dir).unwrap();
        active
            .save(std::path::Path::new(&dir).join("warp-flash-active.png"))
            .unwrap();
        gone.save(std::path::Path::new(&dir).join("warp-flash-gone.png"))
            .unwrap();
        exit.save(std::path::Path::new(&dir).join("warp-flash-exit.png"))
            .unwrap();
        cruise
            .save(std::path::Path::new(&dir).join("warp-flash-cruise.png"))
            .unwrap();
        normal
            .save(std::path::Path::new(&dir).join("warp-flash-normal.png"))
            .unwrap();
    }
}
