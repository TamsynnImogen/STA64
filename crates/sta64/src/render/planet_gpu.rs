//! Opt-in, windowless frames of a colony's lights (RND-12): `STA64_GPU_TESTS=1` (a hardware adapter) and
//! `STA64_TEST_INSTALL` (an Armada II install; `STA64_TEST_MOD`, `STA64_TEST_MAP` optional). The map view's own systems
//! load a stock map; a planet of it is given an owner whose race has a `cityTextureName` and a colony row of a chosen
//! population through the link's view, the simulated clock runs until the drawn population has caught up, and the planet
//! is drawn from its lit and its dark hemisphere under the Stock look and the Refit look. Frames are written only when
//! `STA64_GPU_OUT` names a directory; nothing of the install is committed or kept.
use super::*;
use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::render::RenderPlugin;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::ExitCondition;
use std::sync::{Arc, Mutex};

const SIZE: u32 = 768;

/// Test switch for comparing the alloc picture's filters (the original's is not established).
pub static NEAREST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// What the driver feeds the link: the planet, its owner's race and the population its colony row says.
#[derive(Resource, Default)]
struct Scenario {
    race: String,
    pop: f32,
    /// Sim ticks the clock runs each frame.
    ticks: u32,
    /// Where the camera is: the planet's centre plus the key light's direction times `side` (1 toward the light,
    /// -1 away from it) times `dist` radii.
    side: f32,
    dist: f32,
    /// The planet under test (its sim entity), found once the map is built.
    planet: Option<sta_sim::EntityId>,
}

#[derive(Component)]
struct Done;

#[derive(Resource)]
struct Target(Handle<Image>);

#[allow(clippy::type_complexity)]
fn retarget(mut commands: Commands, target: Res<Target>, mut cameras: Query<(Entity, &mut RenderTarget), (With<Camera3d>, Without<Done>)>) {
    for (e, mut rt) in &mut cameras {
        if matches!(*rt, RenderTarget::Window(_)) {
            *rt = RenderTarget::from(target.0.clone());
            commands.entity(e).insert(Done);
        }
    }
}

/// Pick the planet (the first whose ground texture name contains `STA64_TEST_PLANET`), then every frame feed the link its view with that planet owned and populated.
fn drive(
    mut scenario: ResMut<Scenario>,
    link: Option<ResMut<crate::sim_link::SimLink>>,
    mut clock: ResMut<crate::clock::AnimClock>,
    planets: Query<(&PlanetColony, &PlanetBody)>,
) {
    let Some(mut link) = link else { return };
    if scenario.planet.is_none() {
        // The first planet with two ground hemispheres and a sim row.
        let view = link.curr().clone();
        let want = std::env::var("STA64_TEST_PLANET").unwrap_or_default().to_ascii_lowercase();
        scenario.planet = planets
            .iter()
            .find(|(c, b)| b.ground.len() == 2 && b.radius > 0.0 && view.unit(c.0).is_some() && b.ground_name.to_ascii_lowercase().contains(&want))
            .map(|(c, _)| c.0);
    }
    let Some(id) = scenario.planet else { return };
    let mut view = (**link.curr()).clone();
    let owner = sta_sim::PlayerId(0);
    if let Some(u) = view.units.iter_mut().find(|u| u.id == id) {
        u.owner = owner;
    }
    match view.attrs.seats.iter_mut().find(|s| s.player == owner) {
        Some(s) => s.race = Some(scenario.race.clone()),
        None => view.attrs.seats.push(sta_sim::attrs::SeatView { player: owner, race: Some(scenario.race.clone()), ..Default::default() }),
    }
    view.colonies.retain(|c| c.id != id);
    view.colonies.push(sta_sim::colony::ColonyView {
        id,
        population: sta_sim::Fixed::from_f64(scenario.pop as f64),
        max_population: sta_sim::Fixed::from_int(5000),
        colonizer: None,
    });
    link.feed(Arc::new(view));
    for _ in 0..scenario.ticks {
        clock.ticked();
    }
}

/// The camera: the planet's centre, `dist` radii out toward the key light (`side` 1) or away from it (-1).
#[allow(clippy::type_complexity)]
fn place_camera(
    scenario: Res<Scenario>,
    planets: Query<(&PlanetColony, &PlanetBody, &GlobalTransform)>,
    lights: Query<(&GlobalTransform, &DirectionalLight)>,
    mut camera: Query<(&mut Transform, &Camera), crate::map::MainCamera>,
) {
    let Some(id) = scenario.planet else { return };
    let Some((_, body, gt)) = planets.iter().find(|(c, _, _)| c.0 == id) else { return };
    let Some(key) = lights.iter().max_by(|a, b| a.1.illuminance.total_cmp(&b.1.illuminance)).map(|(t, _)| t.back().as_vec3()) else { return };
    let radius = body.radius * gt.compute_transform().scale.x;
    let centre = gt.translation();
    let eye = centre + key * scenario.side * radius * scenario.dist;
    let up = if key.y.abs() > 0.95 { Vec3::X } else { Vec3::Y };
    if let Some(mut best) = camera.iter_mut().max_by_key(|(_, c)| c.order) {
        *best.0 = Transform::from_translation(eye).looking_at(centre, up);
    }
}

/// Mean RGB and the share of pixels brighter than 24 in the central disc of an RGBA frame, for the log.
fn stats(w: u32, h: u32, rgba: &[u8]) -> ([f32; 3], f32) {
    let (cx, cy, r) = (w as f32 / 2.0, h as f32 / 2.0, w.min(h) as f32 * 0.3);
    let (mut sum, mut n, mut lit) = ([0f64; 3], 0u32, 0u32);
    for (i, p) in rgba.chunks_exact(4).enumerate() {
        let (x, y) = ((i as u32 % w) as f32, (i as u32 / w) as f32);
        if (x - cx).hypot(y - cy) <= r {
            for c in 0..3 {
                sum[c] += p[c] as f64;
            }
            n += 1;
            lit += (p[0].max(p[1]).max(p[2]) > 24) as u32;
        }
    }
    (sum.map(|v| (v / n.max(1) as f64) as f32), lit as f32 / n.max(1) as f32)
}

#[test]
fn colony_lights_frames_on_a_stock_map() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 and STA64_TEST_INSTALL to draw a colony's lights");
        return;
    }
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
        eprintln!("SKIP: STA64_TEST_INSTALL names no install");
        return;
    };
    let mod_name = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), mod_name.as_deref()).expect("mount");
    let map_name = std::env::var("STA64_TEST_MAP").unwrap_or_else(|_| "a2_fed01".into());

    // `STA64_TEST_NEAREST=1`: the alloc picture is filtered nearest instead of linear, for comparing the two.
    let nearest = std::env::var("STA64_TEST_NEAREST").as_deref() == Ok("1");
    NEAREST.store(nearest, std::sync::atomic::Ordering::Relaxed);
    let suffix = if nearest { "-nearest" } else { "" };
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .set(ImagePlugin::default_linear())
            .disable::<bevy::winit::WinitPlugin>(),
    );
    let info = app.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>().cloned();
    if let Some(info) = &info
        && crate::bench::frames::is_software_adapter(info.device_type, &info.name)
    {
        eprintln!("SKIP: only a software adapter is available ({})", info.name);
        return;
    }
    crate::app::configure_sets(&mut app);
    let rts = sta_content::layered::cfg_h(&vfs, "RTS_CFG.h");
    let table = sta_assets::load_sprite_table(&vfs);
    let maps = crate::map::browser(&vfs, Some(&map_name), "planet lights frame test".into());
    assert!(maps.maps.iter().any(|m| m.eq_ignore_ascii_case(&map_name)), "{map_name} is not in the install");
    app.insert_resource(crate::clock::AnimClock::new(crate::clock::ClockMode::Bench { step: 1.0 / 30.0 }, 0))
        .add_plugins(crate::sim_link::SimLinkPlugin)
        .insert_resource(crate::render::sprites::SpriteTables(Arc::new(table)))
        .insert_resource(crate::viewer::TeamColours::load(&vfs))
        .insert_resource(crate::map::MapSeat(None))
        .insert_resource(crate::map::CameraLimits::from_cfg(rts.as_ref()))
        .insert_resource(crate::viewer::Mounted(vfs))
        .insert_resource(maps)
        .insert_resource(crate::sim_link::BenchDriven)
        .insert_resource(Scenario { pop: 0.0, ticks: 0, side: 1.0, dist: std::env::var("STA64_TEST_DIST").ok().and_then(|v| v.parse().ok()).unwrap_or(2.6), ..default() });
    app.add_plugins(crate::map::MapPlugin)
        .add_plugins(crate::render::units::UnitsRenderPlugin)
        .add_plugins(crate::render::fx::FxMaterialsPlugin)
        .add_plugins(crate::render::hulls::HullPlugin)
        .add_plugins(crate::render::profile::ProfilePlugin);
    let mut target = Image::new_target_texture(SIZE, SIZE, bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, None);
    target.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(target);
    app.insert_resource(Target(target.clone()))
        .add_systems(Update, (retarget, drive.after(crate::app::GameSet::SimLink)))
        .add_systems(PostUpdate, place_camera.before(TransformSystems::Propagate));
    app.finish();
    app.cleanup();

    for _ in 0..3 {
        app.update();
    }
    // The race to give the planet: the first with a `cityTextureName` (by ODF stem).
    let races = app.world().resource::<CityRaces>().clone();
    let want = std::env::var("STA64_TEST_RACE").unwrap_or_default().to_ascii_lowercase();
    let race = races.by_stem.keys().find(|k| k.contains(&want)).cloned().expect("the install's races have a cityTextureName");
    app.world_mut().resource_mut::<Scenario>().race = race.clone();
    for _ in 0..120 {
        app.update();
        if app.world().resource::<Scenario>().planet.is_some() {
            break;
        }
    }
    let planet = app.world().resource::<Scenario>().planet.expect("the map has a planet with two ground hemispheres and a sim row");
    let heavy = app.world().get_resource::<crate::sim_link::ContentRes>().and_then(|c| c.0.rules().population()).map_or(5000.0, |p| p.levels[3].to_f32());
    let city = races.get(&race).cloned().unwrap();
    eprintln!("planet {planet:?}, race {race} ({} k={}), cfgPOP_HEAVY {heavy}", city.texture, city.count);
    for _ in 0..20 {
        app.update();
    }

    let out_dir = std::env::var("STA64_GPU_OUT").ok();
    let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
    let mut results: Vec<(String, [f32; 3], f32)> = Vec::new();
    let mut shoot = |app: &mut App, label: String| {
        for _ in 0..12 {
            app.update();
        }
        let before = frames.lock().unwrap().len();
        let out = Arc::clone(&frames);
        app.world_mut().spawn(Screenshot::image(target.clone())).observe(move |shot: On<ScreenshotCaptured>| out.lock().unwrap().push(shot.image.clone()));
        for _ in 0..60 {
            app.update();
            if frames.lock().unwrap().len() > before {
                break;
            }
        }
        assert_eq!(frames.lock().unwrap().len(), before + 1, "no capture for {label}");
        let image = frames.lock().unwrap().last().unwrap().clone().try_into_dynamic().unwrap();
        let rgba = image.to_rgba8();
        let (mean, lit) = stats(rgba.width(), rgba.height(), rgba.as_raw());
        eprintln!("{label}: centre mean {mean:?}, lit share {lit:.3}");
        if let Some(dir) = &out_dir {
            std::fs::create_dir_all(dir).unwrap();
            image.save(std::path::Path::new(dir).join(format!("{label}.png"))).unwrap();
        }
        results.push((label, mean, lit));
    };

    for (look, name) in [(crate::render::profile::RenderProfile::Stock, "stock"), (crate::render::profile::RenderProfile::Refit, "refit")] {
        *app.world_mut().resource_mut::<crate::render::profile::RenderProfile>() = look;
        // Population 0 first, then up to half and full, each caught up by running simulated time (200 a second:
        // 5000 people take 25 s = 25 frames of 30 ticks). Coming down snaps, so each look starts from 0 again.
        for (pop, pname) in [(0.0, "pop0"), (heavy * 0.5, "pophalf"), (heavy, "popfull")] {
            {
                let mut s = app.world_mut().resource_mut::<Scenario>();
                s.pop = pop;
                s.ticks = 30;
            }
            for _ in 0..((pop / 200.0).ceil() as usize + 4) {
                app.update();
            }
            app.world_mut().resource_mut::<Scenario>().ticks = 0;
            let drawn = app.world_mut().query::<&PlanetCities>().iter(app.world()).map(|c| c.drawn.pop).fold(f32::NAN, f32::max);
            eprintln!("{name} {pname}: drawn population {drawn}");
            // As the planet meshes are drawn (`CITY_RADIUS`) the camera sees the far wall of the sphere: from the key
            // light's side it sees the hemisphere that faces away from the key light.
            for (side, sname) in [(-1.0, "lit"), (1.0, "dark")] {
                app.world_mut().resource_mut::<Scenario>().side = side;
                shoot(&mut app, format!("{name}-{pname}-{sname}{suffix}"));
            }
        }
        // Back to nothing for the other look.
        app.world_mut().resource_mut::<Scenario>().pop = 0.0;
        for _ in 0..3 {
            app.update();
        }
    }

    let get = |label: &str| results.iter().find(|r| r.0 == format!("{label}{suffix}")).map(|r| r.1).unwrap();
    let diff = |a: [f32; 3], b: [f32; 3]| (0..3).map(|c| (a[c] - b[c]).abs()).sum::<f32>();
    // The pass reaches the screen: a full colony differs from an empty planet on the lit side under the Stock look.
    let stock_lit = diff(get("stock-pop0-lit"), get("stock-popfull-lit"));
    let stock_dark = diff(get("stock-pop0-dark"), get("stock-popfull-dark"));
    let refit_dark = diff(get("refit-pop0-dark"), get("refit-popfull-dark"));
    eprintln!("mean colour change, population 0 -> full: stock lit {stock_lit:.2}, stock dark {stock_dark:.2}, refit dark {refit_dark:.2}");
    assert!(stock_lit > 0.05, "the stock pass shows nothing on the lit hemisphere");
    assert!(stock_dark > 0.05, "the stock pass shows nothing on the dark hemisphere");
    assert!(refit_dark > 0.05, "the Refit look shows nothing on the dark hemisphere");
}
