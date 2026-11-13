//! Optional windowless framebuffer check, using only synthetic geometry and ODFs.

use super::*;
use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::render::RenderPlugin;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::ExitCondition;
use sta_sim::command::{Command, LocalRouter};
use sta_sim::{Fixed, PlayerId, TickOutput, WorldSource, WorldView};

struct Frozen(Arc<WorldView>);

impl WorldSource for Frozen {
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

fn link(key: ContentKey, shields: i32) -> SimLink {
    let unit = UnitView {
        id: EntityId(1),
        def: key,
        owner: PlayerId(0),
        pos: sta_core::Vec3::ZERO,
        heading: (Fixed::ZERO, Fixed::ONE),
        speed: Fixed::ZERO,
        hull: [Fixed::from_int(100); 2],
        shields: [[Fixed::from_int(shields), Fixed::from_int(100)]; 4],
        flags: UnitFlags::SELECTABLE,
        progress: None,
        crew: None,
        energy: None,
        autonomy: None,
    };
    SimLink::new(
        Box::new(Frozen(Arc::new(WorldView {
            units: vec![unit],
            ..default()
        }))),
        Box::new(LocalRouter::new()),
        PlayerId(0),
    )
}

#[test]
#[ignore = "requires a windowless Vulkan adapter; no game install or GUI"]
fn synthetic_outline_draws_disappears_and_returns_in_framebuffer() {
    let mut def = sta_content::UnitDef::new("outline_fixture", "craft");
    def.raw = Arc::new(
        sta_odf::resolve("outline_fixture", &mut |_| {
            Some("alwaysShowShields = 1".into())
        })
        .unwrap(),
    );
    let db = Arc::new(sta_content::ContentDb::builder().unit(def).build().unwrap());
    let key = db.key("outline_fixture").unwrap();

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
            .disable::<bevy::winit::WinitPlugin>(),
    );
    crate::app::configure_sets(&mut app);
    app.add_plugins(PersistentShieldsPlugin);
    app.insert_resource(ClearColor(Color::BLACK));
    app.insert_resource(ContentRes(db));
    app.insert_resource(link(key, 100));
    let mut image = Image::new_target_texture(
        256,
        256,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        None,
    );
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::from(target.clone()),
        Msaa::Off,
        Transform::from_xyz(0.0, 0.0, 160.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    let root = app
        .world_mut()
        .spawn((
            crate::sim_link::SimEntity(EntityId(1)),
            Transform::IDENTITY,
            Visibility::Inherited,
            ModelBounds {
                min: Vec3::new(-20.0, -15.0, -40.0),
                max: Vec3::new(30.0, 15.0, 40.0),
            },
        ))
        .id();
    let hull_mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(24.0, 12.0, 40.0));
    let hull_material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb(0.5, 0.0, 0.0),
            unlit: true,
            ..default()
        });
    app.world_mut().spawn((
        Mesh3d(hull_mesh),
        MeshMaterial3d(hull_material),
        Transform::IDENTITY,
        ChildOf(root),
    ));
    app.insert_resource(SimEntities(BTreeMap::from([(EntityId(1), root)])));
    app.finish();
    app.cleanup();

    let frames = Arc::new(Mutex::new(Vec::new()));
    for (name, shields) in [("positive", 100), ("depleted", 0), ("recharged", 1)] {
        app.insert_resource(link(key, shields));
        for _ in 0..12 {
            app.update();
        }
        let sink = frames.clone();
        app.world_mut()
            .spawn(Screenshot::image(target.clone()))
            .observe(move |shot: On<ScreenshotCaptured>| {
                sink.lock().unwrap().push((name, shot.image.clone()));
            });
        let before = frames.lock().unwrap().len();
        for _ in 0..120 {
            app.update();
            if frames.lock().unwrap().len() > before {
                break;
            }
        }
        assert!(frames.lock().unwrap().len() > before, "no {name} frame");
    }
    let mut counts = BTreeMap::new();
    for (name, image) in frames.lock().unwrap().iter() {
        let dynamic = image.clone().try_into_dynamic().unwrap();
        let rgba = dynamic.to_rgba8();
        let blue = rgba
            .pixels()
            .filter(|p| p[2] > 25 && p[2] > p[0].saturating_add(10))
            .count();
        let red = rgba
            .pixels()
            .filter(|p| p[0] > 40 && p[0] > p[2].saturating_add(10))
            .count();
        eprintln!("persistent shields {name}: blue={blue} hull={red}");
        assert!(red > 100, "synthetic hull lost in {name}");
        counts.insert(*name, blue);
        if let Ok(dir) = std::env::var("STA64_GPU_OUT") {
            std::fs::create_dir_all(&dir).unwrap();
            dynamic
                .save(std::path::Path::new(&dir).join(format!("persistent-shields-{name}.png")))
                .unwrap();
        }
    }
    assert!(counts["positive"] > 100, "outline never drew: {counts:?}");
    assert_eq!(counts["depleted"], 0, "outline survives depletion");
    assert_eq!(
        counts["positive"], counts["recharged"],
        "recharge changed the static outline"
    );
}
