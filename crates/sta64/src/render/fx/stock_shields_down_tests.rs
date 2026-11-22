//! Tests of the Stock look's shields-down effect. Models, sheets and content are invented here (no install); time is
//! stepped by hand (one sim tick per frame), never read from the clock of the machine.
use super::*;
use crate::app::configure_sets;
use crate::render::fx::impacts::{ImpactFx, ImpactFxPlugin, ImpactKind};
use crate::render::profile::RenderProfile;
use crate::sim_link::SimLinkPlugin;
use bevy::time::TimeUpdateStrategy;
use sta_assets::a2fmt::Matrix34;
use sta_assets::a2fmt::spr::SpriteTable;
use sta_assets::model::{Channel, Group, Keys, LightingMaterial, Maps, Mesh as ModelMesh, Node, SourceFormat};
use sta_content::{ContentBuilder, ContentDb, Features, UnitDef, WeaponDef, WeaponKind};
use sta_sim::command::LocalRouter;
use sta_sim::fake::ScriptedSource;
use sta_sim::{Fixed, PlayerId, TICK_HZ, TickOutput, WorldSource, WorldView};
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------------------------

/// A rotation about y by `deg` degrees and then a translation, as a node matrix (SOD row-vector convention).
fn about_y(deg: f32, t: [f32; 3]) -> Matrix34 {
    let (s, c) = deg.to_radians().sin_cos();
    Matrix34 { m: [c, 0.0, -s, 0.0, 1.0, 0.0, s, 0.0, c, t[0], t[1], t[2]] }
}

/// A square of half size `half` in the node's own xy plane, its texture coordinates the top left quarter of a sheet.
fn square(half: f32) -> ModelMesh {
    let positions = vec![[-half, -half, 0.0], [half, -half, 0.0], [half, half, 0.0], [-half, half, 0.0]];
    let uvs = vec![[0.0, 0.5], [0.5, 0.5], [0.5, 0.0], [0.0, 0.0]];
    ModelMesh {
        positions,
        uvs,
        groups: vec![Group { material: "m".into(), blend: "additive".into(), maps: Maps { diffuse: Some("authored_sheet".into()), ..Default::default() }, faces: vec![[(0, 0), (1, 1), (2, 2)], [(0, 0), (2, 2), (3, 3)]] }],
        two_sided: true,
    }
}

fn node(name: &str, parent: Option<usize>, m: Matrix34, kind: NodeKind) -> Node {
    Node { name: name.into(), parent, children: vec![], transform: m, kind }
}

/// Three bands under a null node: `left` and `right` stand still at different flipbook offsets, `spin` turns about y on a
/// channel of eight keys over two seconds (45 degrees each).
fn authored_model() -> Model {
    let keys: Vec<Matrix34> = (0..8).map(|k| about_y(k as f32 * 45.0, [0.0, 30.0, 0.0])).collect();
    Model {
        format: SourceFormat::Sod,
        version: 1.8,
        materials: vec![LightingMaterial { name: "m".into(), ambient: [1.0; 3], diffuse: [1.0; 3], specular: [0.0; 3], power: 1.0, model: 0, self_illuminating: false }],
        nodes: vec![
            node("root", None, Matrix34::IDENTITY, NodeKind::Null),
            node("null2", Some(0), Matrix34::IDENTITY, NodeKind::Null),
            node("left", Some(1), about_y(0.0, [-30.0, -30.0, 0.0]), NodeKind::Mesh(square(10.0))),
            node("right", Some(0), about_y(0.0, [30.0, -30.0, 0.0]), NodeKind::Mesh(square(10.0))),
            node("spin", Some(0), about_y(0.0, [0.0, 30.0, 0.0]), NodeKind::Mesh(square(10.0))),
        ],
        channels: vec![Channel { node: "spin".into(), period: 2.0, keys: Keys::Matrix(keys) }],
        // Four frames of a second; the helper's rule puts time 0.55 s in frame 2.
        flipbooks: vec![("left".into(), "cells".into(), 0.0), ("right".into(), "cells".into(), 0.55), ("spin".into(), "cells".into(), 0.0)],
        warnings: Vec::new(),
        pbr: None,
    }
}

fn sheet_table() -> SpriteTable {
    SpriteTable::parse(b"sprite_table\n@referenceWidth=64\n@referenceHeight=64\n@animation cells\noffset 4 1.0 step\n@auto=square\n@keyframes\n")
}

#[test]
fn the_bands_are_every_mesh_node_with_their_channel_flipbook_and_parent() {
    let geo = band_geometry(&authored_model()).unwrap();
    assert_eq!(geo.bands.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["left", "right", "spin"]);
    assert!(geo.bands.iter().all(|b| b.texture.as_deref() == Some("authored_sheet")));
    assert_eq!(geo.bands.iter().map(|b| b.keys.is_some()).collect::<Vec<_>>(), [false, false, true]);
    assert_eq!(geo.bands[1].flipbook, Some(("cells".to_owned(), 0.55)));
    // The radius is the model's database radius (the bands' box is centred on the origin).
    assert!((geo.r_db - database_radius(&authored_model()).unwrap()).abs() < 1e-6);
    // A model with no mesh draws nothing.
    assert!(band_geometry(&Model { nodes: vec![node("n", None, Matrix34::IDENTITY, NodeKind::Null)], ..authored_model() }).is_none());
}

#[test]
fn a_band_spins_in_steps_of_its_channel_evenly_over_the_period_and_loops() {
    let geo = band_geometry(&authored_model()).unwrap();
    let b = &geo.bands[2];
    let at = |t: f32| band_transform(&b.parent, &b.local, b.keys.as_ref(), t);
    // Eight keys over two seconds: a key every quarter second, 45 degrees each; a quarter turn at half a second.
    assert!(at(0.5).rotation.angle_between(Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2)) < 1e-3, "{:?}", at(0.5).rotation);
    // (The model's rotation about +y turns the other way in render space, which mirrors z: the sense is the model's.)
    // A step: the key holds until the next one.
    assert_eq!(at(0.5), at(0.5 + 0.1));
    assert_ne!(at(0.5), at(0.5 + 0.25));
    // A full turn over the period, then the same again; a static band stays where its node is.
    assert_eq!(at(2.0 + 0.5), at(0.5));
    assert!((at(0.0).translation - Vec3::new(0.0, 30.0, 0.0)).length() < 1e-4);
    let l = &geo.bands[0];
    assert_eq!(band_transform(&l.parent, &l.local, l.keys.as_ref(), 2.7).translation, Vec3::new(-30.0, -30.0, 0.0));
}

#[test]
fn the_effect_is_wanted_only_with_the_hold_and_a_pool_at_zero() {
    assert!(wanted(true, 0.0, 100.0));
    assert!(!wanted(false, 0.0, 100.0), "without the hold the pool is above zero on the next tick: no flicker");
    assert!(!wanted(true, 5.0, 100.0));
    assert!(!wanted(true, 0.0, 0.0), "no shields at all");
}

// ---------------------------------------------------------------------------------------------
// In a world
// ---------------------------------------------------------------------------------------------

/// A scripted source whose units 2 and above show pools that follow the tick: `now(tick)` per facing, 100 at most.
struct Pools {
    inner: ScriptedSource,
    now: fn(u64) -> [i32; 4],
    /// Whether the view shows unit 2 cloaked at a tick.
    cloak: fn(u64) -> bool,
    tick: u64,
}

impl Pools {
    fn patch(&self, view: Arc<WorldView>) -> Arc<WorldView> {
        let mut v = (*view).clone();
        let now = (self.now)(self.tick);
        for u in v.units.iter_mut().filter(|u| u.id.0 >= 2) {
            if (self.cloak)(self.tick) {
                u.flags = u.flags | sta_sim::view::UnitFlags::CLOAKED;
            }
            for f in 0..4 {
                u.shields[f] = [Fixed::from_int(now[f]), Fixed::from_int(100)];
            }
        }
        Arc::new(v)
    }
}

impl WorldSource for Pools {
    fn step(&mut self, c: &[sta_sim::command::Command]) -> TickOutput {
        self.tick += 1;
        let mut o = self.inner.step(c);
        o.view = self.patch(o.view);
        o
    }
    fn view(&self) -> Arc<WorldView> {
        self.patch(self.inner.view())
    }
}

fn content(hold: bool) -> ContentDb {
    let mut target = UnitDef::new("t_target", "cruiser");
    let mut features = Features::default();
    features.shield_zero_hold = hold;
    target.features = Some(features);
    ContentBuilder::default()
        .weapon(WeaponDef::new("w_x", "phaser", WeaponKind::Beam))
        .unit(UnitDef::new("t_ship", "cruiser"))
        .unit(target)
        .build()
        .unwrap()
}

/// The effects present per frame as `(unit, facing)` lists, for `frames` frames of a world under `profile`.
fn run(hold: bool, profile: Option<RenderProfile>, now: fn(u64) -> [i32; 4], events: &str, until: Option<u64>, frames: usize) -> (Vec<Vec<u32>>, usize) {
    let (rows, shells, _) = run_cloak(hold, profile, now, |_| false, events, until, frames);
    (rows, shells)
}

/// [`run`] with a cloak per tick; also the effect's visibility per frame (`None`: no effect that frame).
#[allow(clippy::type_complexity)]
fn run_cloak(hold: bool, profile: Option<RenderProfile>, now: fn(u64) -> [i32; 4], cloak: fn(u64) -> bool, events: &str, until: Option<u64>, frames: usize) -> (Vec<Vec<u32>>, usize, Vec<Option<bool>>) {
    let db = content(hold);
    let text = format!(
        "{{ units: [
            {{ id: 1, def: \"t_ship\", owner: 0, hull: 800, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 0 }} }}] }},
            {{ id: 2, def: \"t_target\", owner: 1, hull: 400, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 600 }} }}]{} }},
        ], events: [{events}] }}",
        until.map_or(String::new(), |t| format!(", until: {t}"))
    );
    let inner = ScriptedSource::from_json5(&text, &db).unwrap();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<Assets<crate::render::fx::quad::FxQuadMaterial>>()
        .init_resource::<Assets<crate::render::fx::shield::ShieldMaterial>>()
        .init_resource::<Assets<crate::render::fx::fire::FireMaterial>>()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / TICK_HZ as f64)))
        .add_plugins(SimLinkPlugin)
        .insert_resource(ContentRes(Arc::new(db)))
        .insert_resource(SpriteTables(Arc::new(sheet_table())))
        .insert_resource(SimLink::new(Box::new(Pools { inner, now, cloak, tick: 0 }), Box::new(LocalRouter::new()), PlayerId(0)));
    if let Some(p) = profile {
        app.insert_resource(p);
    }
    app.add_plugins((crate::render::fx::weapons::WeaponFxPlugin, ImpactFxPlugin));
    configure_sets(&mut app);
    let (mut rows, mut shells, mut shown) = (Vec::new(), 0, Vec::new());
    for _ in 0..frames {
        app.update();
        let mut v: Vec<u32> = app.world_mut().query::<&ShieldsDown>().iter(app.world()).map(|s| s.unit.0).collect();
        v.sort();
        rows.push(v);
        shown.push(app.world_mut().query::<(&ShieldsDown, &Visibility)>().iter(app.world()).next().map(|(s, v)| s.shown && *v != Visibility::Hidden));
        shells = shells.max(app.world_mut().query::<&ImpactFx>().iter(app.world()).filter(|f| f.kind == ImpactKind::Shield).count());
    }
    (rows, shells, shown)
}

fn down(at: u64, facing: u8) -> String {
    format!("{{ at: {at}, event: {{ ShieldsDown: {{ id: 2, facing: {facing} }} }} }},")
}

/// Facing 1 empties at tick 10 and the pool is back above zero from tick 40.
fn one_pool(tick: u64) -> [i32; 4] {
    if (10..40).contains(&tick) { [60, 0, 60, 60] } else { [60; 4] }
}

#[test]
fn the_effect_is_there_while_the_pool_is_at_zero_after_the_event_and_gone_when_it_recharges() {
    let (rows, _) = run(true, Some(RenderProfile::Stock), one_pool, &down(10, 1), None, 60);
    let first = rows.iter().position(|r| !r.is_empty()).expect("the effect appeared");
    assert!((9..=12).contains(&first), "appeared at frame {first}");
    let last = rows.iter().rposition(|r| !r.is_empty()).unwrap();
    assert!((36..=40).contains(&last), "gone after frame {last}: the pool is above zero again from tick 40");
    assert!(rows[first..=last].iter().all(|r| r == &[2]), "one effect for the craft, all the while: {:?}", &rows[first..=last]);
    assert!(rows[last + 1..].iter().all(Vec::is_empty), "it does not come back");
}

#[test]
fn nothing_under_the_remaster_look_which_keeps_its_shell() {
    let (rows, shells) = run(true, Some(RenderProfile::Refit), one_pool, &down(10, 1), None, 60);
    assert!(rows.iter().all(Vec::is_empty));
    assert_eq!(shells, 1, "the Refit shell of ShieldsDown is still drawn");
    // And under Stock that shell is not (the effect takes its place).
    let (_, shells) = run(true, Some(RenderProfile::Stock), one_pool, &down(10, 1), None, 60);
    assert_eq!(shells, 0);
    // Without the profile resources (today's windowless path) it is the Refit path as well.
    assert!(run(true, None, one_pool, &down(10, 1), None, 60).0.iter().all(Vec::is_empty));
}

#[test]
fn nothing_for_a_profile_without_the_hold_where_the_pool_is_back_on_the_next_tick() {
    // The same event and the same view, but the craft's features have no `shield_zero_hold`: not even one frame.
    let (rows, _) = run(false, Some(RenderProfile::Stock), one_pool, &down(10, 1), None, 60);
    assert!(rows.iter().all(Vec::is_empty));
    // A pool already above zero when the event is read (no hold, recharged at once) draws nothing either.
    fn back(tick: u64) -> [i32; 4] {
        if tick == 10 { [0; 4] } else { [60; 4] }
    }
    assert!(run(true, Some(RenderProfile::Stock), back, &down(10, 1), None, 40).0.iter().skip(14).all(Vec::is_empty), "gone as soon as the pool is above zero");
}

#[test]
fn directional_pools_one_effect_per_craft_from_the_first_event_until_no_pool_is_at_zero() {
    fn two(tick: u64) -> [i32; 4] {
        match tick {
            10..=29 => [60, 0, 0, 60],
            30..=49 => [60, 60, 0, 60],
            _ => [60; 4],
        }
    }
    let (rows, _) = run(true, Some(RenderProfile::Stock), two, &(down(10, 1) + &down(12, 2)), None, 70);
    // Two pools down, one effect: never two at once.
    assert!(rows.iter().all(|r| r.len() <= 1));
    let first = rows.iter().position(|r| !r.is_empty()).expect("the effect appeared");
    assert!((9..=12).contains(&first), "{first}");
    // After tick 30 one pool is still at zero: the effect stays, and it ends when the last pool is back.
    assert!(rows[35..45].iter().all(|r| r == &[2]), "{:?}", &rows[35..45]);
    assert!(rows[55..].iter().all(Vec::is_empty));
    // An event for a pool above zero, with no pool at zero, makes nothing.
    let (rows, _) = run(true, Some(RenderProfile::Stock), one_pool, &down(10, 3), None, 30);
    assert!(rows.iter().all(Vec::is_empty));
}

#[test]
fn no_bands_while_the_view_shows_the_craft_cloaked_and_they_come_back_if_the_pool_is_still_at_zero() {
    // Cloaked from tick 20 to 29 (the view's flag), the pool at zero from 10 to 39; the craft stays visible to the viewer.
    fn cloaked(tick: u64) -> bool {
        (20..30).contains(&tick)
    }
    let (rows, _, shown) = run_cloak(true, Some(RenderProfile::Stock), one_pool, cloaked, &down(10, 1), None, 60);
    // Shown before the cloak, hidden (not ended) while cloaked, shown again after, gone with the pool.
    assert_eq!(shown[16], Some(true));
    assert!(shown[22..27].iter().all(|s| *s == Some(false)), "{:?}", &shown[22..27]);
    assert!(rows[22..27].iter().all(|r| r == &[2]), "the effect is hidden, not ended");
    assert_eq!(shown[33], Some(true), "back after the decloak with the pool still at zero");
    assert!(shown[45..].iter().all(Option::is_none));
    // Under the Remaster look nothing of this exists (its shell is drawn as before).
    let (rows, shells, _) = run_cloak(true, Some(RenderProfile::Refit), one_pool, cloaked, &down(10, 1), None, 60);
    assert!(rows.iter().all(Vec::is_empty));
    assert_eq!(shells, 1);
    assert!(hidden_by_cloak(true) && !hidden_by_cloak(false));
}

#[test]
fn the_effect_ends_with_its_unit() {
    let (rows, _) = run(true, Some(RenderProfile::Stock), one_pool, &down(10, 1), Some(25), 60);
    let last = rows.iter().rposition(|r| !r.is_empty()).unwrap();
    assert!((20..=26).contains(&last), "the unit left the view at tick 25: gone after frame {last}");
}

#[test]
fn the_assets_are_dropped_and_loaded_again_when_the_mounted_files_change() {
    use crate::render::textures::TextureCache;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(crate::viewer::Mounted(sta_vfs::Vfs::default()))
        .init_resource::<TextureCache>()
        .init_resource::<DownLoadState>();
    app.add_systems(Update, reset_on_remount.run_if(resource_exists_and_changed::<crate::viewer::Mounted>));
    app.update();
    app.insert_resource(ShieldsDownAssets { bands: Vec::new(), r_db: 5.0 });
    app.world_mut().resource_mut::<DownLoadState>().tried = true;
    app.update();
    assert!(app.world().get_resource::<ShieldsDownAssets>().is_some() && app.world().resource::<DownLoadState>().tried);
    app.world_mut().resource_mut::<crate::viewer::Mounted>().0 = sta_vfs::Vfs::default();
    app.update();
    assert!(app.world().get_resource::<ShieldsDownAssets>().is_none());
    assert!(!app.world().resource::<DownLoadState>().tried);
}

// ---------------------------------------------------------------------------------------------
// The frame
// ---------------------------------------------------------------------------------------------

/// With `STA64_GPU_TESTS=1` and a hardware adapter: an invented model and sheet drawn through the production band
/// assets, visual spawn, spin and flipbook systems, read back from the frame:
/// - added to what is there, the texture's alpha darkens nothing;
/// - each band takes its own flipbook cell (their time offsets differ) from the game clock, times the white gain;
/// - a band spins on its channel: face on at the start, edge on a quarter turn (half a second) later.
///
/// Prints SKIP and passes otherwise.
#[test]
fn gpu_the_bands_add_in_their_own_cells_and_spin_on_the_game_clock() {
    use bevy::asset::RenderAssetUsages;
    use bevy::camera::RenderTarget;
    use bevy::image::ImageSampler;
    use bevy::render::RenderPlugin;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
    use bevy::window::ExitCondition;
    use std::sync::Mutex;

    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 to draw the shields-down test");
        return;
    }
    let _serial = crate::bench::frames::gpu_serial();
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .set(ImagePlugin::default_linear())
            .disable::<bevy::log::LogPlugin>()
            .disable::<bevy::winit::WinitPlugin>(),
    );
    if let Some(info) = app.world().get_resource::<bevy::render::renderer::RenderAdapterInfo>() {
        eprintln!("adapter: {} ({:?})", info.name, info.device_type);
        if crate::bench::frames::is_software_adapter(info.device_type, &info.name) {
            eprintln!("SKIP: only a software adapter is available ({})", info.name);
            return;
        }
    }
    app.add_plugins(crate::render::fx::stock_shots::StockShotsMaterialPlugin).insert_resource(ClearColor(Color::BLACK));

    // The sheet: 2 x 2 cells of 32 texels; cell 0 a quarter, cell 2 (column 0, row 1) three eighths, the others full.
    let mut data = Vec::new();
    for y in 0..64usize {
        for x in 0..64usize {
            let v = match (x / 32, y / 32) {
                (0, 0) => 64u8,
                (0, 1) => 96,
                _ => 255,
            };
            data.extend_from_slice(&[v, v, v, 255]);
        }
    }
    let mut sheet = Image::new(Extent3d { width: 64, height: 64, depth_or_array_layers: 1 }, TextureDimension::D2, data, TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD);
    sheet.sampler = ImageSampler::nearest();
    let texture = app.world_mut().resource_mut::<Assets<Image>>().add(sheet);
    let geo = band_geometry(&authored_model()).unwrap();
    let assets = app.world_mut().resource_scope(|w, mut meshes: Mut<Assets<Mesh>>| {
        w.resource_scope(|_, mut mats: Mut<Assets<StockShotMaterial>>| build_assets(geo, &mut |_| Some(texture.clone()), &mut meshes, &mut mats).unwrap())
    });
    app.insert_resource(assets);
    app.insert_resource(SpriteTables(Arc::new(sheet_table())));
    app.insert_resource(AnimClock::new(crate::clock::ClockMode::Wall, 0));
    app.add_systems(Update, (attach_visuals, animate).chain());

    let eye = Vec3::new(0.0, 0.0, 64.0 / (std::f32::consts::FRAC_PI_8).tan());
    let mut target = Image::new_target_texture(128, 128, TextureFormat::Rgba8UnormSrgb, None);
    target.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(target);
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::from(target.clone()),
        Msaa::Off,
        bevy::core_pipeline::tonemapping::Tonemapping::None,
        Transform::from_translation(eye).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    // The effect's frame: the ellipsoid is a sphere of the model's database radius, so the model's own size shows.
    app.world_mut().spawn((ShieldsDown { unit: EntityId(2), shown: true }, Transform::IDENTITY, Visibility::Inherited));
    app.finish();
    app.cleanup();
    let capture = |app: &mut App| {
        let frames = Arc::new(Mutex::new(Vec::<Image>::new()));
        let out = Arc::clone(&frames);
        app.world_mut().spawn(Screenshot::image(target.clone())).observe(move |shot: On<ScreenshotCaptured>| out.lock().unwrap().push(shot.image.clone()));
        for _ in 0..60 {
            app.update();
            if !frames.lock().unwrap().is_empty() {
                break;
            }
        }
        let frame = frames.lock().unwrap().first().cloned().expect("no capture");
        frame.try_into_dynamic().unwrap().to_rgba8()
    };
    let srgb = |v: f32| (255.0 * if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }).round() as i32;
    let near = |got: [u8; 3], want: [i32; 3]| got.iter().zip(want).all(|(g, w)| (i32::from(*g) - w).abs() <= 12);
    let px = |img: &image::RgbaImage, x: u32, y: u32| {
        let p = img.get_pixel(x, y).0;
        [p[0], p[1], p[2]]
    };
    let show = |app: &mut App, tick: u64, bg: f32| {
        app.world_mut().resource_mut::<ClearColor>().0 = Color::linear_rgb(bg, bg, bg);
        let mut clock = AnimClock::new(crate::clock::ClockMode::Wall, 0);
        for _ in 0..tick {
            clock.ticked();
        }
        app.insert_resource(clock);
        for _ in 0..4 {
            app.update();
        }
    };
    // Left band, cell 0 (a quarter of full, times the gain); right band, its own later offset, cell 2 (three eighths).
    let (a, b) = (GAIN * 64.0 / 255.0, GAIN * 96.0 / 255.0);
    show(&mut app, 0, 0.0);
    let img = capture(&mut app);
    assert!(near(px(&img, 34, 94), [srgb(a); 3]), "left band: {:?}", px(&img, 34, 94));
    assert!(near(px(&img, 94, 94), [srgb(b); 3]), "right band: {:?}", px(&img, 94, 94));
    // The spinning band is face on at the start; between the bands nothing is drawn.
    assert!(near(px(&img, 64, 34), [srgb(a); 3]), "spinning band face on: {:?}", px(&img, 64, 34));
    assert!(near(px(&img, 64, 94), [0, 0, 0]));
    // Added to what is there: a grey of 0.2 plus the band's light.
    show(&mut app, 0, 0.2);
    let img = capture(&mut app);
    assert!(near(px(&img, 34, 94), [srgb(0.2 + a); 3]), "additive: {:?}", px(&img, 34, 94));
    assert!(near(px(&img, 5, 5), [srgb(0.2); 3]));
    // Half a second of game time on, the spinning band has turned a quarter and stands edge on; the cells moved on.
    show(&mut app, TICK_HZ as u64 / 2, 0.0);
    let img = capture(&mut app);
    assert!(near(px(&img, 64, 34), [0, 0, 0]), "spinning band edge on: {:?}", px(&img, 64, 34));
}

/// With `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`): the shields-down model resolves and loads, has bands, each band's
/// texture is mounted and its flipbook is an animation of the sprite table, and the radius of its database is positive.
/// Skips without an install.
#[test]
fn install_the_shields_down_model_resolves_with_its_textures_and_flipbooks() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
        eprintln!("SKIP: STA64_TEST_INSTALL names no install");
        return;
    };
    let m = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), m.as_deref()).unwrap();
    let Some((path, model)) = crate::render::fx::mining_beam::load_named_model(&vfs, MODEL, "test") else {
        panic!("the shields-down model {MODEL} is not in the install at {root}");
    };
    eprintln!("the shields-down model resolved to {path}");
    let geo = band_geometry(&model).expect("the model has bands");
    assert!(geo.r_db > 0.0);
    let table = sta_assets::load_sprite_table(&vfs);
    for b in &geo.bands {
        let tex = b.texture.as_deref().unwrap_or_else(|| panic!("band {} names no texture", b.name));
        assert!(sta_assets::find_texture(&vfs, tex).is_some(), "band {}: the texture {tex} is not mounted", b.name);
        let (anim, _) = b.flipbook.as_ref().unwrap_or_else(|| panic!("band {} names no flipbook", b.name));
        assert!(table.animation(anim).is_some(), "band {}: the flipbook {anim} is in no sprite table", b.name);
        assert!(b.keys.as_ref().is_some_and(|(period, k)| *period > 0.0 && k.len() > 1), "band {} has no channel", b.name);
    }
}
