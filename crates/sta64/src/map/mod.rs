//! Map view: a BZN from the mounted install, every object placed with its model.
//!
//! Object to model: the SOD (or M3D, glTF) named like the object's ODF, scaled by
//! the ODF's `ScaleSOD`; `dont_load_SOD = 1` objects (asteroid fields) have no
//! model. The BZN team picks the object's entry in `teamcolor.odf`. The map's
//! `BackgroundArt` SOD is the skybox, drawn unlit around the camera.
//!
//! Controls: left click/drag select, right click move, Space stop, Left/Right
//! map, PgUp/PgDn x10, WASD pan, middle-drag orbit, wheel zoom, R reset view.

use crate::clock::AnimClock;
use crate::render::model_scene::{self, PartCache, SpawnCtx, to_bevy};
use crate::render::sprites::SpriteTables;
use crate::render::textures::{Loader, TextureCache};
use crate::render::units::SelectRadius;
use crate::sim_link::{CommandQueue, ContentRes, Selection, SimEntity, SimLink};
use crate::viewer::{Mounted, Orbit, TeamColours};
use bevy::app::{HierarchyPropagatePlugin, Propagate};
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, PerspectiveProjection, Projection};
use bevy::image::CompressedImageFormats;
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use sta_assets::a2fmt::Matrix34;
use sta_assets::model::Model;
use std::collections::HashMap;
use std::sync::Arc;
use sta_content::{ContentDb, MapDef, OdfIndex};
use sta_sim::command::LocalRouter;
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{ContentKey, EntityId, Fixed, PlayerId, Sim, TeamId, Vec3 as SimVec3};

#[derive(Resource)]
pub struct MapBrowser {
    pub maps: Vec<String>,
    pub index: usize,
    pub dirty: bool,
    pub mount_summary: String,
    pub info: String,
}

pub fn browser(vfs: &sta_vfs::Vfs, initial: Option<&str>, mount_summary: String) -> MapBrowser {
    let mut maps: Vec<String> = vfs
        .list("bzn")
        .into_iter()
        .filter(|p| p.ends_with(".bzn"))
        .map(|p| p.trim_start_matches("bzn/").trim_end_matches(".bzn").to_owned())
        .collect();
    maps.sort();
    let index = initial
        .and_then(|n| {
            let n = n.to_ascii_lowercase();
            maps.iter().position(|m| *m == n.trim_end_matches(".bzn"))
        })
        .unwrap_or(0);
    MapBrowser {
        maps,
        index,
        dirty: true,
        mount_summary,
        info: String::new(),
    }
}

/// The seat this client plays in the map view (`--seat <n>`): the link speaks for
/// it, `LocalPlayer` is it (so the fog and shroud are its) and only its units take
/// orders. `None` (the default) is the view as before: every unit visible, every
/// team's ships ordered, as the bench.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MapSeat(pub Option<u8>);

/// The sim's content, loaded once per mount: the database and the ODF index its
/// map conversions resolve scenery through.
pub(crate) struct SimContent {
    pub(crate) db: Arc<ContentDb>,
    pub(crate) index: Arc<OdfIndex>,
    /// The race ODFs: start units, stock, GUI cfg and music per race.
    pub(crate) races: Arc<sta_content::Races>,
}

/// Loaded models, kept across maps, and the render-side ODF cache.
#[derive(Resource, Default)]
pub(crate) struct Catalogue {
    /// What the sim reads: `ContentDb::load` (CON-03) of the mount.
    content: Option<SimContent>,
    /// What only the renderer reads (nebula and planet keys, the starfield's
    /// sprites): the ODFs the map names, as raw keys. The sim's database does not
    /// carry them.
    legacy: sta_content::legacy::Legacy,
    models: HashMap<String, Option<(String, Arc<Model>)>>,
    /// The moon each resource marker is swapped for (RND-16), read from its ODFs.
    moons: crate::render::moons::MoonVariants,
}

impl Catalogue {
    /// The mount's content database, built on first use.
    pub(crate) fn content(&mut self, vfs: &sta_vfs::Vfs) -> Result<&SimContent, String> {
        if self.content.is_none() {
            let (index, dupes) = OdfIndex::build(vfs);
            let (db, warnings) = ContentDb::load(vfs, &index).map_err(|e| e.to_string())?;
            info!(
                "content: {} definitions, profile {:?}, {} warnings ({} duplicate ODFs)",
                db.len(),
                db.profile(),
                warnings.len(),
                dupes.len()
            );
            let (races, race_warnings) = sta_content::Races::load(vfs, &index);
            info!("races: {} ({} warnings)", races.iter().map(|r| r.stem.as_str()).collect::<Vec<_>>().join(", "), race_warnings.len());
            self.content = Some(SimContent { db: Arc::new(db), index: Arc::new(index), races: Arc::new(races) });
        }
        Ok(self.content.as_ref().expect("just set"))
    }

    fn model(&mut self, vfs: &sta_vfs::Vfs, name: &str) -> Option<(String, Arc<Model>)> {
        let key = name.to_ascii_lowercase();
        let key = key.trim_end_matches(".sod").to_owned();
        if let Some(m) = self.models.get(&key) {
            return m.clone();
        }
        let m = sta_assets::find_model(vfs, &key).and_then(|(path, fmt)| match sta_assets::load_model(vfs, &path, fmt) {
            Ok(m) => Some((path, Arc::new(m))),
            Err(e) => {
                warn!("{e}");
                None
            }
        });
        self.models.insert(key, m.clone());
        m
    }
}

/// The sim entities a map's objects became, found again by what they were placed
/// as: `from_setup` spawns the `MapDef`'s objects, the view lists them back, and
/// each BZN object takes the next entity of its definition, place and owner.
struct Placed(HashMap<(ContentKey, [Fixed; 3], PlayerId), std::collections::VecDeque<EntityId>>);

impl Placed {
    fn of(sim: &Sim) -> Placed {
        use sta_sim::WorldSource;
        let mut by: HashMap<_, std::collections::VecDeque<EntityId>> = HashMap::new();
        for u in &sim.view().units {
            by.entry((u.def, [u.pos.x, u.pos.y, u.pos.z], u.owner)).or_default().push_back(u.id);
        }
        Placed(by)
    }

    fn take(&mut self, key: ContentKey, o: &sta_bzn::MapObject) -> Option<EntityId> {
        let p = SimVec3::from_f32([o.transform[9], o.transform[10], o.transform[11]]);
        let owner = if o.team > 0 { PlayerId((o.team - 1).min(255) as u8) } else { sta_sim::world::NEUTRAL };
        // Resources are neutral in the sim whoever the BZN gave them to (SIM-18).
        let mut pop = |o| self.0.get_mut(&(key, [p.x, p.y, p.z], o)).and_then(|q| q.pop_front());
        pop(owner).or_else(|| pop(sta_sim::world::NEUTRAL))
    }

    /// Entities no BZN object took (`from_setup` made them from something the
    /// renderer does not draw as a ship).
    #[cfg(test)]
    fn rest(self) -> Vec<EntityId> {
        self.rest_where(|_| true)
    }

    /// As [`Placed::rest`], keeping only the entities `keep` says yes to, by owner.
    fn rest_where(self, keep: impl Fn(PlayerId) -> bool) -> Vec<EntityId> {
        let mut ids: Vec<EntityId> = self.0.into_iter().filter(|((_, _, owner), _)| keep(*owner)).flat_map(|(_, v)| v).collect();
        ids.sort();
        ids
    }
}

/// What the sim is made from: the BZN converted to a `MapDef` (CON-07), a `GameSetup`
/// over it and the mount's content, and the sim from that (`Sim::from_setup`).
pub(crate) struct SimSide {
    pub(crate) setup: GameSetup,
    pub(crate) sim: Sim,
    /// Conversion and spawn warnings together.
    pub(crate) warnings: usize,
    /// A skirmish's computer players, for the link to think after each tick.
    pub(crate) controllers: Vec<Box<dyn sta_sim::setup::Controller>>,
    /// A skirmish's local race, or why the skirmish could not be set up.
    pub(crate) skirmish: Option<Result<crate::game::Seats, String>>,
}

/// The sim of a map: the map view's (`skirmish: None`: one idle seat per owner or
/// start, nobody starts with anything) or a skirmish's (two seats, each with its
/// race's starting forces, the opponent under the AI or idle).
pub(crate) fn build_sim(
    map: &sta_bzn::Map,
    name: &str,
    content: &Arc<ContentDb>,
    index: &OdfIndex,
    vfs: &sta_vfs::Vfs,
    seat: Option<u8>,
    skirmish: Option<(&crate::game::SkirmishCfg, &sta_content::Races)>,
) -> SimSide {
    let converted = MapDef::convert_bzn_in(map, content, vfs, index);
    let mut map_def = converted.map;
    map_def.name = name.to_owned();
    let mut plan = None;
    let players = match skirmish {
        Some((cfg, races)) => match crate::game::seats(&map_def, cfg, races, content) {
            Ok(s) => {
                let players = s.players.clone();
                plan = Some(Ok(s));
                players
            }
            Err(e) => {
                plan = Some(Err(e));
                seats(&map_def, seat, &map.race)
            }
        },
        None => seats(&map_def, seat, &map.race),
    };
    let setup = GameSetup { seed: 1, players, map: Arc::new(map_def), content: content.clone(), options: GameOptions::default() };
    let (sim, spawn_warnings, controllers) = match (&plan, skirmish) {
        (Some(Ok(_)), Some((_, races))) => crate::game::start(&setup, races),
        _ => {
            let (sim, w) = Sim::from_setup_with_warnings(&setup);
            (sim, w, Vec::new())
        }
    };
    for w in converted.warnings.iter().chain(&spawn_warnings).take(20) {
        debug!("{}: {}", w.file, w.message);
    }
    SimSide { warnings: converted.warnings.len() + spawn_warnings.len(), setup, sim, controllers, skirmish: plan }
}

/// The seats of a map: every owner and start slot, and the one the client plays.
fn seats(map: &MapDef, me: Option<u8>, race: &str) -> Vec<PlayerSetup> {
    let n = map.objects.iter().filter_map(|o| o.owner).chain(map.starts.iter().map(|s| s.slot)).chain(me).max().map_or(1, |m| m as usize + 1);
    (0..n.min(250) as u8)
        .map(|i| PlayerSetup {
            id: PlayerId(i),
            team: TeamId(i),
            race: race.to_owned(),
            control: if me == Some(i) { Control::Human } else { Control::Idle },
        })
        .collect()
}

#[derive(Component)]
struct MapRoot;

/// Camera limits for the current map (Bevy space) and settings from RTS_CFG.h.
#[derive(Resource)]
pub struct CameraLimits {
    lo: Vec3,
    hi: Vec3,
    max_dist: f32,
    /// Pixels from the window edge that scroll the view (`SCROLL_BORDER_WIDTH`).
    border: f32,
}

impl CameraLimits {
    pub fn from_cfg(cfg: Option<&sta_assets::cfg_h::CfgH>) -> CameraLimits {
        CameraLimits {
            lo: Vec3::splat(-1e9),
            hi: Vec3::splat(1e9),
            max_dist: 1e9,
            border: cfg.and_then(|c| c.f32("SCROLL_BORDER_WIDTH")).unwrap_or(8.0),
        }
    }
}

/// The render layer of what is drawn before the scene: the sky, the starfield and
/// the background objects. Stock draws them, then clears the depth for the scene, so
/// they never hide it; here the background camera draws this layer and the main
/// camera draws the scene over it with its own depth.
pub const BACKGROUND_LAYER: usize = 1;

/// The camera that draws [`BACKGROUND_LAYER`], a child of the main camera.
#[derive(Component)]
pub struct BackgroundCamera;

/// The main (scene) camera, for queries that want the one camera the player moves.
pub type MainCamera = (With<Camera3d>, Without<BackgroundCamera>);

#[derive(Component)]
#[require(Propagate<RenderLayers> = Propagate(RenderLayers::layer(BACKGROUND_LAYER)))]
struct Sky;

/// A `background_obj` (suns, background planets, wrecks, gates) at its place in the
/// map, drawn before the scene.
#[derive(Component)]
#[require(Propagate<RenderLayers> = Propagate(RenderLayers::layer(BACKGROUND_LAYER)))]
struct BackgroundObject;

#[derive(Component)]
struct MapHud;

/// Frames per second, top right.
#[derive(Component)]
struct FpsText;

/// The map's lights: 0 the key, 1 the fill. Aimed by the map's `dlight` objects.
#[derive(Component)]
struct MapLight(usize);

/// Constant rotation about a local axis (asteroid fields orbit, rocks tumble).
#[derive(Component)]
struct Spin {
    axis: Vec3,
    /// Radians per second.
    speed: f32,
}


/// The rock models an asteroid field draws from (the engine's fixed table).
const ROCKS: [&str; 8] = ["master", "master1", "master2", "master3", "master4", "master5", "master4", "master7"];

/// Small deterministic generator, seeded per field.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32) / ((1u64 << 24) as f32)
    }
}

type MapEntities = Or<(With<MapRoot>, With<Sky>)>;

/// The rest of the map's state, as one system parameter.
type MapState<'w> = (
    ResMut<'w, crate::render::particles::Particles>,
    ResMut<'w, Selection>,
    ResMut<'w, CommandQueue>,
    ResMut<'w, AnimClock>,
    ResMut<'w, CameraLimits>,
    ResMut<'w, Assets<crate::render::planet::PlanetMaterial>>,
    ResMut<'w, Assets<crate::render::nebula::NebulaMaterial>>,
    Res<'w, crate::render::nebula::NoiseVolume>,
    Res<'w, MapSeat>,
    Option<Res<'w, crate::game::SkirmishCfg>>,
);

/// The ambient light and the two map lights, as one system parameter.
type Lighting<'w, 's> = (
    ResMut<'w, crate::render::strategic::Silhouettes>,
    ResMut<'w, GlobalAmbientLight>,
    Query<'w, 's, (&'static MapLight, &'static mut Transform, &'static mut DirectionalLight), Without<Orbit>>,
);

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TextureCache>()
            .init_resource::<PartCache>()
            .init_resource::<Catalogue>()
            .init_resource::<MapSeat>()
            .add_message::<crate::ui::radar::RadarClick>()
            .init_resource::<crate::render::particles::Particles>()
            .insert_resource(ClearColor(Color::BLACK))
            .insert_resource(GlobalAmbientLight {
                color: Color::WHITE,
                brightness: 60.0,
                ..default()
            })
            .insert_resource(crate::viewer::OrbitButtons {
                rotate: MouseButton::Middle,
                pan: None,
            })
            .add_plugins(crate::render::planet::PlanetPlugin)
            .add_plugins(crate::render::rock::RockPlugin)
            .add_plugins(crate::render::nebula::NebulaPlugin)
            .add_plugins(crate::render::strategic::StrategicPlugin)
            .init_resource::<crate::render::strategic::Silhouettes>()
            .add_plugins(bevy::diagnostic::FrameTimeDiagnosticsPlugin::default())
            .add_plugins(HierarchyPropagatePlugin::<RenderLayers>::new(PostUpdate))
            .add_systems(OnEnter(crate::app::AppState::FrontEnd), clear_match);
        // Behind the front end the match's cameras, lights and texts exist only from the
        // loading screen to the front end's return (INT-M4); otherwise from the start.
        if app.world().contains_resource::<crate::game::flow::Flow>() {
            app.add_systems(OnEnter(crate::app::AppState::Loading), setup);
        } else {
            app.add_systems(Startup, setup);
        }
        app.add_systems(
                Update,
                (
                    crate::clock::advance_idle_clock.run_if(not(resource_exists::<crate::sim_link::SimLink>)),
                    input,
                    rebuild,
                    radar_click,
                    pan_keys,
                    crate::viewer::orbit_camera,
                    radar_camera,
                    spin,
                    sky_follows_camera,
                    hud,
                    hud_toggle,
                    fps,
                    model_scene::animate,
                    crate::render::sprites::animate,
                    crate::render::sprites::face_camera,
                    crate::render::particles::simulate,
                    crate::render::particles::draw,
                )
                    .chain()
                    .after(crate::app::GameSet::SimLink)
                    .run_if(crate::game::flow::match_running)
                    .run_if(crate::platform::foreground),
            );
    }
}

/// What the match's own cameras, lights and texts are tagged with, to take them
/// down when the front end returns.
#[derive(Component)]
pub(crate) struct MatchEntity;

/// Back at the front end: the map, its sky and the match's cameras go.
fn clear_match(mut commands: Commands, q: Query<Entity, Or<(MapEntities, With<MatchEntity>)>>, mut particles: ResMut<crate::render::particles::Particles>) {
    particles.clear();
    for e in &q {
        commands.entity(e).despawn();
    }
}

fn setup(mut commands: Commands) {
    let projection = Projection::Perspective(PerspectiveProjection {
        near: 1.0,
        far: 400_000.0,
        ..default()
    });
    // The scene, drawn over the background camera's picture with its own depth.
    commands
        .spawn((
            MatchEntity,
            Camera3d::default(),
            Camera {
                order: 1,
                clear_color: ClearColorConfig::None,
                ..default()
            },
            // The nebulae's march stops at the scene's depth.
            bevy::core_pipeline::prepass::DepthPrepass,
            projection.clone(),
            Transform::from_xyz(0.0, 3000.0, 6000.0).looking_at(Vec3::ZERO, Vec3::Y),
            Orbit {
                focus: Vec3::ZERO,
                yaw: 0.0,
                pitch: 0.9,
                dist: 8000.0,
                home: (Vec3::ZERO, 8000.0),
            },
        ))
        .with_child((BackgroundCamera, Camera3d::default(), Camera { order: 0, ..default() }, projection, RenderLayers::layer(BACKGROUND_LAYER)));
    commands.spawn((
        MatchEntity,
        MapLight(0),
        RenderLayers::from_layers(&[0, BACKGROUND_LAYER]),
        DirectionalLight {
            illuminance: 6000.0,
            ..default()
        },
        Transform::from_xyz(3.0, 5.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        MatchEntity,
        MapLight(1),
        RenderLayers::from_layers(&[0, BACKGROUND_LAYER]),
        DirectionalLight {
            illuminance: 1500.0,
            ..default()
        },
        Transform::from_xyz(-4.0, -2.0, -3.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        MatchEntity,
        FpsText,
        Text::new(""),
        TextFont {
            font_size: bevy::text::FontSize::Px(15.0),
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.0),
            right: Val::Px(12.0),
            ..default()
        },
    ));
    commands.spawn((
        MatchEntity,
        MapHud,
        Text::new(""),
        TextFont {
            font_size: bevy::text::FontSize::Px(15.0),
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.0),
            left: Val::Px(12.0),
            max_width: Val::Percent(HUD_TEXT_WIDTH_PCT),
            ..default()
        },
    ));
}

/// Camera distance a skirmish opens at, over the player's start.
const START_DIST: f32 = 2500.0;

fn input(keys: Res<ButtonInput<KeyCode>>, mut browser: ResMut<MapBrowser>, mut orbit: Query<&mut Orbit>, skirmish: Option<Res<crate::game::SkirmishCfg>>) {
    let n = browser.maps.len() as isize;
    if n == 0 {
        return;
    }
    // A match stays on its map: the browser's keys are the map viewer's.
    let browsing = skirmish.is_none();
    let step = [
        (KeyCode::ArrowRight, 1),
        (KeyCode::ArrowLeft, -1),
        (KeyCode::PageDown, 10),
        (KeyCode::PageUp, -10),
    ]
    .iter()
    .find(|(k, _)| keys.just_pressed(*k))
    .map(|(_, s)| *s)
    .filter(|_| browsing);
    if let Some(s) = step {
        browser.index = (browser.index as isize + s).rem_euclid(n) as usize;
        browser.dirty = true;
    }
    if keys.just_pressed(KeyCode::KeyR) {
        for mut o in &mut orbit {
            (o.focus, o.dist) = o.home;
        }
    }
}

/// A click or drag on the radar puts the camera over that map point.
fn radar_click(
    mut clicks: MessageReader<crate::ui::radar::RadarClick>,
    limits: Res<CameraLimits>,
    mut q: Query<&mut Orbit>,
) {
    let Some(c) = clicks.read().filter(|c| c.button == bevy::picking::pointer::PointerButton::Primary).last() else { return };
    for mut o in &mut q {
        let at = Vec3::new(c.at.x, 0.0, -c.at.y);
        o.focus = at.clamp(limits.lo, limits.hi);
    }
}

/// The camera's view on the map plane for the radar: where its four screen corners
/// look at ground level (sim x, z). A corner above the horizon goes far along its ray.
fn radar_camera(
    camera: Query<(&Camera, &GlobalTransform), MainCamera>,
    limits: Res<CameraLimits>,
    feed: Option<ResMut<crate::ui::radar::RadarFeed>>,
) {
    let (Some(mut feed), Ok((cam, gt))) = (feed, camera.single()) else { return };
    let Some(size) = cam.logical_viewport_size() else { return };
    let far = limits.max_dist.min(1e6) * 3.0;
    let corners = [Vec2::ZERO, Vec2::new(size.x, 0.0), size, Vec2::new(0.0, size.y)];
    let mut out = [Vec2::ZERO; 4];
    for (o, c) in out.iter_mut().zip(corners) {
        let Ok(ray) = cam.viewport_to_world(gt, c) else { return };
        let (p, d) = (ray.origin, *ray.direction);
        let t = if d.y < -1e-3 { (-p.y / d.y).min(far) } else { far };
        let hit = p + d * t;
        *o = Vec2::new(hit.x, -hit.z);
    }
    feed.camera = Some(out);
}

/// WASD and the window edges move the camera's focus over the map plane; the
/// focus stays over the map and the zoom within its size.
fn pan_keys(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    limits: Res<CameraLimits>,
    window: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut q: Query<(&mut Orbit, &Transform)>,
) {
    let edge = window.single().ok().and_then(|w| {
        let c = w.cursor_position()?;
        let (wd, ht, b) = (w.width(), w.height(), limits.border);
        Some(Vec2::new(
            if c.x <= b { -1.0 } else if c.x >= wd - b { 1.0 } else { 0.0 },
            if c.y <= b { 1.0 } else if c.y >= ht - b { -1.0 } else { 0.0 },
        ))
    });
    for (mut o, tf) in &mut q {
        let fwd = Vec3::new(tf.forward().x, 0.0, tf.forward().z).normalize_or_zero();
        let right = Vec3::new(tf.right().x, 0.0, tf.right().z).normalize_or_zero();
        let mut d = Vec3::ZERO;
        for (k, v) in [(KeyCode::KeyW, fwd), (KeyCode::KeyS, -fwd), (KeyCode::KeyD, right), (KeyCode::KeyA, -right)] {
            if keys.pressed(k) {
                d += v;
            }
        }
        if let Some(e) = edge {
            d += right * e.x + fwd * e.y;
        }
        let step = d * o.dist * 0.8 * time.delta_secs();
        o.focus += step;
        o.focus = o.focus.clamp(limits.lo, limits.hi);
        o.dist = o.dist.clamp(50.0, limits.max_dist);
    }
}

fn spin(clock: Res<AnimClock>, mut q: Query<(&Spin, &mut Transform)>) {
    for (s, mut t) in &mut q {
        t.rotate_local(Quat::from_axis_angle(s.axis, s.speed * clock.frame_dt()));
    }
}

fn sky_follows_camera(camera: Query<&Transform, (MainCamera, Without<Sky>)>, mut sky: Query<&mut Transform, With<Sky>>) {
    let Ok(cam) = camera.single() else { return };
    for mut t in &mut sky {
        t.translation = cam.translation;
    }
}

#[allow(clippy::too_many_arguments)]
fn rebuild(
    mut commands: Commands,
    mut browser: ResMut<MapBrowser>,
    vfs: Res<Mounted>,
    roots: Query<Entity, MapEntities>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut cache: ResMut<TextureCache>,
    mut parts: ResMut<PartCache>,
    mut catalogue: ResMut<Catalogue>,
    device: Option<Res<RenderDevice>>,
    tables: Res<SpriteTables>,
    teams: Res<TeamColours>,
    mut orbit: Query<&mut Orbit>,
    state: MapState,
    lighting: Lighting,
) {
    let (mut silhouettes, mut ambient, mut lights) = lighting;
    let (mut particles, mut selection, mut queue, mut clock, mut limits, mut planet_materials, mut nebula_materials, noise, seat, skirmish_cfg) = state;
    if !browser.dirty {
        return;
    }
    browser.dirty = false;
    particles.clear();
    // The old map's sim goes; the new one is linked once its ships are placed.
    commands.remove_resource::<SimLink>();
    selection.0.clear();
    queue.0.clear();
    *clock = AnimClock::new(clock.mode(), clock.seed());
    for e in &roots {
        commands.entity(e).despawn();
    }
    let Some(name) = browser.maps.get(browser.index).cloned() else {
        browser.info = "No maps under bzn/".into();
        return;
    };
    let header = format!("{name}  ({}/{})", browser.index + 1, browser.maps.len());
    let map = match vfs.0.read(&format!("bzn/{name}.bzn")).map(|r| r.map_err(|e| e.to_string()).and_then(|d| sta_bzn::parse(&d).map_err(|e| e.to_string()))) {
        Some(Ok(m)) => m,
        Some(Err(e)) => {
            browser.info = format!("{header}\n{e}");
            return;
        }
        None => {
            browser.info = format!("{header}\nnot found");
            return;
        }
    };
    // The sim's side: the mount's content, the map converted to a `MapDef`, a
    // `GameSetup` and the sim from it.
    let (content, index, races) = match catalogue.content(&vfs.0) {
        Ok(c) => (c.db.clone(), c.index.clone(), c.races.clone()),
        Err(e) => {
            browser.info = format!("{header}\ncontent: {e}");
            return;
        }
    };
    let SimSide { setup, sim, warnings: sim_warnings, controllers, skirmish } =
        build_sim(&map, &name, &content, &index, &vfs.0, seat.0, skirmish_cfg.as_deref().map(|c| (c, &*races)));
    let mut skirmish_note = None;
    let mut played_race = None;
    let is_match = matches!(skirmish, Some(Ok(_)));
    match skirmish {
        Some(Ok(s)) => {
            info!("{}", s.summary);
            skirmish_note = Some(s.summary);
            played_race = Some(s.local.name.clone());
            commands.insert_resource(s.local);
        }
        Some(Err(e)) => {
            warn!("skirmish: {e}");
            skirmish_note = Some(format!("not a skirmish: {e}"));
        }
        None => {}
    }
    let map_def = setup.map.clone();
    let mut placed = Placed::of(&sim);
    commands.insert_resource(ContentRes(content.clone()));
    let seat_player = seat.0.map(PlayerId);
    commands.insert_resource(crate::render::units::LocalPlayer(seat_player));
    {
        let (lo, hi) = map_def.bounds;
        // The radar draws the map's own rectangle, not the units' spread.
        let (rlo, rhi) = (Vec2::new(lo.x.to_f32(), lo.z.to_f32()), Vec2::new(hi.x.to_f32(), hi.z.to_f32()));
        commands.queue(move |w: &mut World| {
            if let Some(mut st) = w.get_resource_mut::<crate::ui::radar::RadarState>() {
                st.map = Some(crate::ui::radar::RadarMap::of_bounds(rlo, rhi));
                st.map_fixed = true;
            }
        });
        commands.insert_resource(crate::ui::hud::ghost::PlacementBounds(Some((lo, hi))));
        commands.insert_resource(crate::render::fog::FogBounds {
            min: Vec2::new(lo.x.to_f32(), lo.z.to_f32()),
            max: Vec2::new(hi.x.to_f32(), hi.z.to_f32()),
        });
    }
    // Every ODF the map names, its ships' physics ODFs come with them, and the starfield:
    // what the renderer reads that the sim's database does not carry.
    let star_odf = map.starfield_odf.trim_end_matches(".odf").trim_end_matches(".ODF").to_owned();
    let db = Arc::new(catalogue.legacy.db(&vfs.0, map.objects.iter().map(|o| o.odf.as_str()).chain([star_odf.as_str()])));
    let formats = device
        .map(|d| CompressedImageFormats::from_features(d.features()))
        .unwrap_or(CompressedImageFormats::BC);
    let a = map.ambient;
    ambient.color = Color::srgb(a[0].clamp(0.0, 1.0), a[1].clamp(0.0, 1.0), a[2].clamp(0.0, 1.0));
    ambient.brightness = 250.0;

    let neutral = teams.0.first().map(|t| t.1).unwrap_or(Color::WHITE);
    let root = commands.spawn((MapRoot, Transform::IDENTITY, Visibility::default())).id();
    let (mut drawn, mut no_model, mut no_odf, mut tris, mut planets, mut rocks) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    let mut missing: Vec<String> = Vec::new();
    let mut map_lights: Vec<(Vec3, Option<[f32; 3]>)> = Vec::new();
    let mut nebulae = 0usize;
    let mut units = 0usize;
    // Nebula classes drawn as gas: (recipe, radius, positions in Bevy space).
    let mut gas: std::collections::BTreeMap<String, (sta_surface::nebula::NebulaRecipe, f32, Vec<[f32; 3]>)> = Default::default();
    let mut hidden = 0usize;

    for o in &map.objects {
        let sim_key = content.key(&o.odf).filter(|&k| content.unit(k).is_some());
        let Some(def) = db.key(&o.odf).and_then(|k| db.unit(k)) else {
            no_odf += 1;
            missing.push(o.odf.clone());
            continue;
        };
        let odf = &*def.raw;
        // Classes with `hidden = 1` are editor markers, drawn only in edit mode.
        // A directional-light marker aims the map's lights along its front axis.
        if odf.i64("hidden").unwrap_or(0) != 0 {
            if o.odf.to_ascii_lowercase().contains("light") {
                let f = [o.transform[6], o.transform[7], -o.transform[8]];
                map_lights.push((Vec3::from(f).normalize_or(Vec3::NEG_Y), o.light_colour));
            }
            hidden += 1;
            continue;
        }
        // Every nebula carries a light in its glow colour (Refit's lights.rs: the ODF's
        // red/green/blue_glow x 1.4, out to 8x its falloff; a gas class takes its
        // recipe's hue at that strength). Calibrated against the key light at the
        // falloff's start, cut off at its end.
        if odf.class_label().is_some_and(|c| c.eq_ignore_ascii_case("nebula_obj")) {
            let glow = Vec3::new(odf.f32("red_glow").unwrap_or(0.0), odf.f32("green_glow").unwrap_or(0.0), odf.f32("blue_glow").unwrap_or(0.0));
            let strength = glow.length();
            if strength > 0.0 {
                let hue = crate::render::nebula::recipe(&vfs.0, &o.odf).map(|r| Vec3::from(r.gas_a)).filter(|h| h.length() > 0.0).unwrap_or(glow);
                let c = hue / hue.max_element().max(1e-3);
                let start = odf.f32("glow_falloff_start").unwrap_or(60.0) * 8.0;
                let reach = start + odf.f32("glow_falloff_range").unwrap_or(60.0) * 8.0;
                commands.spawn((
                    PointLight {
                        color: Color::srgb(c.x, c.y, c.z),
                        intensity: 1.4 * strength * crate::render::planet::KEY_LUX * 4.0 * std::f32::consts::PI * start * start,
                        range: reach,
                        shadow_maps_enabled: false,
                        ..default()
                    },
                    Transform::from_xyz(o.transform[9], o.transform[10], -o.transform[11]),
                    ChildOf(root),
                ));
            }
        }
        // A nebula class with a gas recipe: drawn as one field, not as billboards.
        if odf.class_label().is_some_and(|c| c.eq_ignore_ascii_case("nebula_obj") || c.eq_ignore_ascii_case("latinum")) {
            let key = o.odf.to_ascii_lowercase();
            let entry = gas.entry(key.clone()).or_insert_with(|| {
                let r = crate::render::nebula::recipe(&vfs.0, &key);
                let latinum = odf.class_label().is_some_and(|c| c.eq_ignore_ascii_case("latinum"));
                let radius = if latinum { 100.0 } else { odf.f32("effectRadius").filter(|r| *r > 10.0 && *r < 5000.0).unwrap_or(300.0) };
                (r.unwrap_or_else(|| sta_surface::nebula::NebulaRecipe::parse("")), radius, Vec::new())
            });
            if crate::render::nebula::recipe(&vfs.0, &key).is_some() {
                entry.2.push([o.transform[9], o.transform[10], -o.transform[11]]);
                nebulae += 1;
                drawn += 1;
                continue;
            }
        }
        if odf.class_label().is_some_and(|c| c.eq_ignore_ascii_case("asteroidfield")) {
            // minAsteroids..maxAsteroids rocks from the engine's table within
            // fieldRadius, scaled by ScaleSOD; the field turns at orbitingSpeed
            // (degrees/s, our reading) and each rock tumbles at rotationSpeed (rad/s).
            // Placement is our approximation of the engine's.
            let mut rng = Lcg(o.entity_id as u32 as u64 ^ 0x9e37_79b9_7f4a_7c15);
            let (lo, hi) = (odf.i64("minAsteroids").unwrap_or(3).max(0), odf.i64("maxAsteroids").unwrap_or(6).max(0));
            let count = lo + ((hi - lo + 1) as f32 * rng.next()) as i64;
            let radius = odf.f32("fieldRadius").unwrap_or(200.0);
            let scale = odf.f32("ScaleSOD").or_else(|| odf.f32("scaleSOD")).filter(|s| *s > 0.0).unwrap_or(1.0);
            let orbit = odf.f32("orbitingSpeed").unwrap_or(0.0).to_radians();
            let tumble = odf.f32("rotationSpeed").unwrap_or(0.0);
            let field = commands
                .spawn((
                    to_bevy(&Matrix34 { m: o.transform }),
                    Visibility::default(),
                    ChildOf(root),
                    Spin { axis: Vec3::Y, speed: orbit },
                ))
                .id();
            for _ in 0..count {
                let name = ROCKS[(rng.next() * ROCKS.len() as f32) as usize % ROCKS.len()];
                let Some((path, model)) = catalogue.model(&vfs.0, name) else { continue };
                let (a, r) = (rng.next() * std::f32::consts::TAU, radius * rng.next().sqrt());
                let at = Vec3::new(a.cos() * r, (rng.next() - 0.5) * radius * 0.3, a.sin() * r);
                let axis = Vec3::new(rng.next() - 0.5, rng.next() - 0.5, rng.next() - 0.5).normalize_or(Vec3::Y);
                let turn = Quat::from_axis_angle(axis, rng.next() * std::f32::consts::TAU);
                let rock = commands
                    .spawn((
                        Transform::from_translation(at).with_rotation(turn),
                        Visibility::default(),
                        ChildOf(field),
                        Spin { axis, speed: tumble },
                    ))
                    .id();
                let mut ctx = SpawnCtx {
                    commands: &mut commands,
                    meshes: &mut meshes,
                    materials: &mut materials,
                    textures: Loader {
                        vfs: &vfs.0,
                        cache: &mut cache,
                        images: &mut images,
                        formats,
                    },
                    team: neutral,
                    borg: false,
                    sprites: &tables.0,
                    damaged: false,
                    normal_maps: true,
                    parts: Some(&mut parts),
                    model_key: path,
                    unlit: false,
                    scale,
                };
                tris += model_scene::spawn(&mut ctx, &model, rock).triangles;
                rocks += 1;
            }
            drawn += 1;
            continue;
        }
        if odf.i64("dont_load_SOD").unwrap_or(0) != 0 {
            no_model += 1;
            continue;
        }
        if odf.class_label().is_some_and(|c| c.eq_ignore_ascii_case("planet")) {
            // Planets are bodies the engine builds itself; the SOD's sphere only
            // gives the radius.
            let scale = odf.f32("ScaleSOD").filter(|s| *s > 0.0).unwrap_or(1.0);
            let radius = catalogue
                .model(&vfs.0, &def.model)
                .and_then(|(_, m)| m.bounds())
                .map(|(lo, hi)| (Vec3::from(hi) - Vec3::from(lo)).max_element() * 0.5)
                .unwrap_or(500.0)
                * scale;
            let ground = odf.str("groundTextureName").unwrap_or("").to_owned();
            let atmo = odf.str("atmosphereTextureName").filter(|a| !a.is_empty()).unwrap_or("PA_ECFR0").to_owned();
            let entity = commands.spawn((to_bevy(&Matrix34 { m: o.transform }), Visibility::default(), ChildOf(root))).id();
            let mut loader = Loader {
                vfs: &vfs.0,
                cache: &mut cache,
                images: &mut images,
                formats,
            };
            missing.extend(crate::render::planet::spawn(&mut commands, &mut meshes, &mut planet_materials, &mut loader, &ground, &atmo, radius, entity));
            planets += 1;
            drawn += 1;
            continue;
        }
        // A resource marker (`trigger_dilithium`, `trigger_tritanium`, ...) is drawn as the
        // moon the game swaps it for (RND-16), the same for whichever resource it is.
        let sim_def = sim_key.and_then(|k| content.unit(k));
        let moon = sim_def.and_then(|d| catalogue.moons.moon_for(&vfs.0, &content, d, o.entity_id as u64));
        let (def, odf) = moon.map_or((def, odf), |m| (m, &*m.raw));
        let Some((path, model)) = catalogue.model(&vfs.0, &def.model) else {
            no_model += 1;
            continue;
        };
        let scale = odf.f32("ScaleSOD").filter(|s| *s > 0.0).unwrap_or(1.0);
        // ScaleSOD goes to the model's positions, not to the object's transform.
        let tf = to_bevy(&Matrix34 { m: o.transform });
        // Background objects (suns, background planets, wrecks, gates) stay where the
        // map puts them but are drawn before the scene, which they never hide.
        let background_obj = odf.class_label().is_some_and(|c| c.eq_ignore_ascii_case("background_obj"));
        let entity = commands.spawn((tf, Visibility::default(), ChildOf(root))).id();
        if background_obj {
            commands.entity(entity).insert(BackgroundObject);
        }
        // A stock nebula drawn as billboards: its puffs fade near and behind others.
        if odf.class_label().is_some_and(|c| c.eq_ignore_ascii_case("nebula_obj")) {
            let radius = odf.f32("effectRadius").filter(|r| *r > 10.0 && *r < 5000.0).unwrap_or(300.0);
            commands.entity(entity).insert(crate::render::nebula::NebulaBoards { radius });
        }
        let units_before = units;
        // What the sim made of the object (a unit of the content database) is drawn
        // by this entity: the ship moves with it and takes orders as one.
        // (The sim placed the moon in the marker's place: the same pick, see `MapDef::convert_bzn`.)
        if let Some(id) = moon.map(|m| m.key).or(sim_key).and_then(|k| placed.take(k, o)) {
            commands.entity(entity).insert(SimEntity(id));
            // A resource (a moon) is neutral and never moves but is clicked for its info and
            // as a mining target.
            if def.movement.max_speed > Fixed::ZERO || o.team > 0 || def.resource.is_some() {
                let radius = model
                    .bounds()
                    .map(|(lo, hi)| (Vec3::from(hi) - Vec3::from(lo)).length() * 0.5)
                    .unwrap_or(20.0)
                    * scale;
                commands.entity(entity).insert(SelectRadius(radius.max(5.0)));
            }
            if def.movement.max_speed > Fixed::ZERO {
                units += 1;
            }
        }
        let team = if o.team > 0 { teams.0.get(o.team as usize).map(|t| t.1).unwrap_or(neutral) } else { neutral };
        // Team-owned ships and stations: their strategic icon, the mod's `<model>_sv`
        // sprite or a silhouette baked from the model.
        let class = odf.class_label().unwrap_or("").to_ascii_lowercase();
        if o.team > 0 && !matches!(class.as_str(), "background_obj" | "latinum" | "asteroidfield" | "nebula_obj" | "planet" | "wormhole_obj") {
            let radius = model.bounds().map(|(lo, hi)| (Vec3::from(hi) - Vec3::from(lo)).length() * 0.5).unwrap_or(20.0) * scale;
            let mobile = units_before != units;
            let mut icon = crate::render::strategic::Strategic::new(crate::render::strategic::shape_for(&class, mobile), team, radius);
            let sv = tables.0.sprite(&format!("{}_sv", def.model));
            if let Some(sprite) = sv
                && let Some(tex) = (Loader { vfs: &vfs.0, cache: &mut cache, images: &mut images, formats }).colour(&sprite.texture, false)
            {
                let uv = sta_assets::spr_anim::sprite_uv(&tables.0, sprite, 0.0);
                icon = icon.with_image(tex, Some(uv));
            } else if let Some(img) = silhouettes.get(&path, &model, &mut images) {
                icon = icon.with_image(img, None);
            }
            commands.entity(entity).insert(icon);
        }
        let mut ctx = SpawnCtx {
            commands: &mut commands,
            meshes: &mut meshes,
            materials: &mut materials,
            textures: Loader {
                vfs: &vfs.0,
                cache: &mut cache,
                images: &mut images,
                formats,
            },
            team,
            borg: false,
            sprites: &tables.0,
            damaged: false,
            normal_maps: true,
            parts: Some(&mut parts),
            model_key: path,
            unlit: false,
            scale,
        };
        let stats = model_scene::spawn(&mut ctx, &model, entity);
        tris += stats.triangles;
        drawn += 1;
    }

    // Skybox: the BackgroundArt cube SOD, or six prefix faces, unlit, about the camera.
    let mut sky_note = String::from("none");
    if !map.background.is_empty() {
        sky_note = format!("{} (not found)", map.background);
        let cube = if map.background.to_ascii_lowercase().ends_with(".sod") { catalogue.model(&vfs.0, &map.background) } else { None };
        if cube.is_none() {
            let prefix = map.background.trim_end_matches(".sod").trim_end_matches(".SOD").to_owned();
            let sky = commands.spawn((Sky, Transform::from_scale(Vec3::splat(1500.0)), Visibility::default())).id();
            let mut loader = Loader {
                vfs: &vfs.0,
                cache: &mut cache,
                images: &mut images,
                formats,
            };
            let faces = crate::render::space::sky_faces(&mut commands, &mut meshes, &mut materials, &mut loader, &prefix, sky);
            sky_note = format!("{prefix} ({faces}/6 faces)");
        }
        if let Some((path, model)) = cube {
            let radius = model
                .bounds()
                .map(|(lo, hi)| (Vec3::from(hi) - Vec3::from(lo)).length() * 0.5)
                .filter(|r| *r > 0.0)
                .unwrap_or(1.0);
            let sky = commands.spawn((Sky, Transform::from_scale(Vec3::splat(150_000.0 / radius)), Visibility::default())).id();
            let mut ctx = SpawnCtx {
                commands: &mut commands,
                meshes: &mut meshes,
                materials: &mut materials,
                textures: Loader {
                    vfs: &vfs.0,
                    cache: &mut cache,
                    images: &mut images,
                    formats,
                },
                team: Color::WHITE,
                borg: false,
                sprites: &tables.0,
                damaged: false,
                normal_maps: false,
                parts: Some(&mut parts),
                model_key: format!("sky:{path}"),
                unlit: true,
                scale: 1.0,
            };
            model_scene::spawn(&mut ctx, &model, sky);
            sky_note = path;
        }
    }

    // Starfield: the map's starfield ODF, `<sprite> = count` and `<sprite>.scale`.
    let mut star_note = format!("{} (not found)", map.starfield_odf);
    if let Some(sf) = db.key(&star_odf).and_then(|k| db.unit(k)).map(|d| &*d.raw) {
        let entries: Vec<(String, u32, f32)> = sf
            .keys()
            .filter(|k| !k.to_ascii_lowercase().ends_with(".scale"))
            .filter_map(|k| {
                let n = sf.i64(k)?;
                let scale = sf.f32(&format!("{k}.scale")).unwrap_or(1.0);
                (n > 0).then(|| (k.to_owned(), n as u32, scale))
            })
            .collect();
        let sizing = crate::render::space::StarSizing::from_cfg(
            vfs.0.read("ART_CFG.h").and_then(Result::ok).map(|b| sta_assets::cfg_h::CfgH::parse(&String::from_utf8_lossy(&b))).as_ref(),
        );
        let stars = commands.spawn((Sky, Transform::IDENTITY, Visibility::default())).id();
        let mut loader = Loader {
            vfs: &vfs.0,
            cache: &mut cache,
            images: &mut images,
            formats,
        };
        let n = crate::render::space::starfield(&mut commands, &mut meshes, &mut materials, &mut loader, &tables.0, &entries, &sizing, 100_000.0, stars);
        star_note = format!("{} ({n} stars)", map.starfield_odf);
    }

    // Nebula fields, one per class with a recipe.
    let classes = gas.values().filter(|g| !g.2.is_empty()).count();
    for (_, (recipe, radius, pos)) in gas.into_iter().filter(|g| !g.1 .2.is_empty()) {
        crate::render::nebula::spawn_field(&mut commands, &mut meshes, &mut nebula_materials, &mut images, &noise, recipe, radius, &pos, root);
    }

    // Lights: the map's dlights if it has them, else the defaults.
    for (ml, mut tf, mut light) in &mut lights {
        match map_lights.get(ml.0) {
            Some((dir, colour)) => {
                *tf = Transform::IDENTITY.looking_to(*dir, if dir.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y });
                light.color = colour.map(|c| Color::srgb(c[0], c[1], c[2])).unwrap_or(Color::WHITE);
            }
            None => {
                let from = if ml.0 == 0 { Vec3::new(3.0, 5.0, 4.0) } else { Vec3::new(-4.0, -2.0, -3.0) };
                *tf = Transform::from_translation(from).looking_at(Vec3::ZERO, Vec3::Y);
                light.color = Color::WHITE;
            }
        }
    }

    // Frame the playable area (SOD space: mirror Z).
    let (lo, size) = (Vec3::from(map.min_extent), Vec3::from(map.size));
    let centre = lo + size * 0.5;
    let focus = Vec3::new(centre.x, 0.0, -centre.z);
    let mut dist = size.x.max(size.z).max(2000.0) * 0.75;
    // A match opens over the local seat's own starting units (their centroid; the start marker
    // is only where the grid begins), near enough to see the base; R returns there.
    let focus = {
        use sta_sim::WorldSource;
        let mine: Vec<Vec2> = match seat_player.filter(|_| is_match) {
            Some(p) => sim.view().units.iter().filter(|u| u.owner == p).map(|u| Vec2::new(u.pos.x.to_f32(), u.pos.z.to_f32())).collect(),
            None => Vec::new(),
        };
        if mine.is_empty() {
            focus
        } else {
            dist = dist.min(START_DIST);
            let c = mine.iter().sum::<Vec2>() / mine.len() as f32;
            Vec3::new(c.x, 0.0, -c.y)
        }
    };
    for mut o in &mut orbit {
        o.focus = focus;
        o.dist = dist;
        o.home = (focus, dist);
    }
    // Keep the focus over the map (with a margin) and the zoom within its size.
    let margin = size.x.max(size.z) * 0.25;
    let (a, b) = (Vec3::new(lo.x, lo.y, -(lo.z + size.z)), Vec3::new(lo.x + size.x, lo.y + size.y, -lo.z));
    limits.lo = a.min(b) - Vec3::splat(margin);
    limits.hi = a.max(b) + Vec3::splat(margin);
    limits.max_dist = size.x.max(size.z).max(2000.0) * 2.0;

    // Sim entities the renderer draws some other way (planets, nebulae, rocks, things
    // without a model): linked to a bare entity, so the units view does not draw them a
    // second time.
    // A match's start units (a seat's outpost, builders) are made by the sim, not by a BZN
    // object, so nothing took them: they are left to the units view, which draws them.
    let unlinked = placed.rest_where(|owner| !is_match || owner == sta_sim::world::NEUTRAL);
    let ghosts = unlinked.len();
    for id in unlinked {
        commands.spawn((SimEntity(id), Transform::IDENTITY, Visibility::default(), ChildOf(root)));
    }

    // The sim, stepped by the sim link. Without a seat the view orders any team's ships,
    // so the link speaks as the bench, which the sim does not restrict to its own units;
    // with one it speaks as the seat and the sim holds it to its own.
    commands.insert_resource(
        SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), seat_player.unwrap_or(PlayerId::BENCH)).with_controllers(controllers),
    );

    missing.sort();
    missing.dedup();
    let mut info = format!(
        "{header}\n{}  v{}  {}\nsize {:.0} x {:.0}   {}   tech tree {}   scripts {:?}\nsky {}   starfield {}\n{} objects: {} drawn ({} ships under the sim, {} planets, {} asteroid rocks, {} nebulae as {} gas fields), {} editor markers ({} lights), {} without a model, {} without an ODF   {} triangles",
        map.description,
        map.version,
        if map.binary { "binary" } else { "text" },
        map.size[0],
        map.size[2],
        race_field(&map.race, played_race.as_deref()),
        map.tech_tree_file,
        map.scripts,
        sky_note,
        star_note,
        map.objects.len(),
        drawn,
        units,
        planets,
        rocks,
        nebulae,
        classes,
        hidden,
        map_lights.len(),
        no_model,
        no_odf,
        tris
    );
    info.push_str(&format!(
        "\nsim: {} objects, {} regions, {} starts, {} seats ({}), {} without a model of their own   content {} defs, {:?}, {} conversion warnings",
        map_def.objects.len(),
        map_def.regions.len(),
        map_def.starts.len(),
        setup.players.len(),
        seat.0.map_or("viewing as the bench".to_owned(), |s| format!("playing seat {s}")),
        ghosts,
        content.len(),
        content.profile(),
        sim_warnings
    ));
    if let Some(n) = skirmish_note {
        info.push_str(&format!("\n{n}"));
    }
    if !missing.is_empty() {
        info.push_str(&format!("\nnot found: {}", missing.iter().take(12).cloned().collect::<Vec<_>>().join(", ")));
    }
    info!("map {}", info.replace('\n', " | "));
    browser.info = info;
}

/// The info panel's race field: the race the local seat plays when there is a match,
/// else the map's default race (a bare map view plays nobody).
fn race_field(map_race: &str, played: Option<&str>) -> String {
    match played {
        Some(r) => format!("race {r}"),
        None => format!("race {map_race} (map default)"),
    }
}

/// Width of the top-left text, as a share of the screen. The notices (top centre) take
/// [`crate::ui::notices::NOTICE_WIDTH_PCT`] around the middle, so this stops short of
/// them; the text wraps inside it.
pub(crate) const HUD_TEXT_WIDTH_PCT: f32 = 33.0;

fn fps(diag: Res<bevy::diagnostic::DiagnosticsStore>, mut q: Query<&mut Text, With<FpsText>>) {
    let fps = diag
        .get(&bevy::diagnostic::FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    for mut t in &mut q {
        t.0 = format!("{fps:.0} fps");
    }
}

fn hud(browser: Res<MapBrowser>, skirmish: Option<Res<crate::game::SkirmishCfg>>, mut q: Query<&mut Text, With<MapHud>>) {
    if !browser.is_changed() {
        return;
    }
    let keys = if skirmish.is_some() { "" } else { "Left/Right map  PgUp/PgDn x10  " };
    for mut t in &mut q {
        t.0 = format!(
            "STA64 {} map view\n{}\n\n{}\n\nleft click/drag select (Shift adds)  right click move  Space stop\n\
             {keys}Tab icons  WASD pan  middle-drag orbit  wheel zoom  R reset  F11 hide this",
            env!("CARGO_PKG_VERSION"),
            browser.mount_summary,
            browser.info
        );
    }
}

/// The map view's debug text sits where a match puts its command grid, so a match
/// starts with it hidden; F11 shows or hides it. The bare map view shows it.
fn hud_toggle(
    keys: Res<ButtonInput<KeyCode>>,
    skirmish: Option<Res<crate::game::SkirmishCfg>>,
    mut shown: Local<Option<bool>>,
    mut q: Query<&mut Visibility, With<MapHud>>,
) {
    let shown = shown.get_or_insert(skirmish.is_none());
    if keys.just_pressed(KeyCode::F11) {
        *shown = !*shown;
    }
    let want = if *shown { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in &mut q {
        if *v != want {
            *v = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_info_panel_names_the_played_race() {
        let played = race_field("federation", Some("borg"));
        assert_eq!(played, "race borg");
        assert!(!played.contains("federation"));
        assert_eq!(race_field("federation", None), "race federation (map default)");
    }

    /// The info text (left) and the notices (centre) never share a column, at any
    /// screen width a match runs at.
    #[test]
    fn the_notices_clear_the_info_text() {
        for w in [800.0_f32, 1280.0, 1920.0, 3440.0] {
            let info_right = 12.0 + w * HUD_TEXT_WIDTH_PCT / 100.0;
            let notice_left = crate::ui::notices::notice_span(w).0;
            assert!(info_right <= notice_left, "{w}: info ends at {info_right}, notices start at {notice_left}");
        }
    }

    /// A match's start units (owned by a seat, made by the sim) are not ghosts: the units
    /// view must draw and pick them; neutral leftovers (planets, nebulae) stay ghosts.
    #[test]
    fn a_matchs_start_units_are_not_ghosted() {
        let z = Fixed::from_int(0);
        let at = [z, z, z];
        let mut m = HashMap::new();
        m.insert((ContentKey::NONE, at, PlayerId(0)), std::collections::VecDeque::from([EntityId(1)]));
        m.insert((ContentKey::NONE, at, sta_sim::world::NEUTRAL), std::collections::VecDeque::from([EntityId(2)]));
        let keep_neutral = |o: PlayerId| o == sta_sim::world::NEUTRAL;
        assert_eq!(Placed(m.clone()).rest_where(keep_neutral), vec![EntityId(2)]);
        assert_eq!(Placed(m).rest(), vec![EntityId(1), EntityId(2)]);
    }
    use sta_sim::command::{Command, CommandKind, Edit, Order};
    use sta_sim::WorldSource;

    #[test]
    fn seats_cover_owners_starts_and_the_player() {
        let mut map = MapDef::empty();
        assert_eq!(seats(&map, None, "x").len(), 1);
        map.starts.push(sta_content::StartPos { slot: 3, pos: Default::default(), heading: (Fixed::ZERO, Fixed::ONE) });
        let s = seats(&map, Some(5), "x");
        assert_eq!(s.len(), 6);
        assert_eq!(s.iter().filter(|p| p.control == Control::Human).count(), 1);
        assert_eq!(s[5].control, Control::Human);
        assert_eq!(s[2].team, TeamId(2));
    }

    /// With `STA64_TEST_INSTALL` (+ optional `STA64_TEST_MOD`, empty for none): the map
    /// view's sim side on real maps. Every BZN object the sim made a unit of finds its
    /// entity again, a ship obeys a move order, and two hostile ships fight.
    #[test]
    fn install_maps_build_a_sim_that_moves_and_fights() {
        let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
        let m = std::env::var("STA64_TEST_MOD").unwrap_or_default();
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
        let (index, _) = OdfIndex::build(&vfs);
        let (db, _) = ContentDb::load(&vfs, &index).unwrap();
        let (db, index) = (Arc::new(db), Arc::new(index));
        let mut names: Vec<String> = vfs.list("bzn").into_iter().filter(|p| p.ends_with(".bzn")).collect();
        names.sort();
        let mut maps = 0;
        for path in names.iter().step_by((names.len() / 8).max(1)) {
            let Some(Ok(data)) = vfs.read(path) else { continue };
            let Ok(bzn) = sta_bzn::parse(&data) else { continue };
            if bzn.objects.is_empty() {
                continue;
            }
            let SimSide { setup, mut sim, warnings, .. } = build_sim(&bzn, path, &db, &index, &vfs, Some(0), None);
            let mut placed = Placed::of(&sim);
            let (mut wanted, mut found) = (0, 0);
            for o in &bzn.objects {
                if let Some(k) = db.key(&o.odf).filter(|&k| db.unit(k).is_some()) {
                    wanted += 1;
                    // A resource marker is placed as its moon (`MapDef::convert_bzn`).
                    let moons = sta_content::map::moon_variants(&db, db.unit(k).unwrap(), &mut |n| {
                        sta_odf::resolve_layered(n, &mut |n, v| index.load(&vfs, n, v)).ok().map(Arc::new)
                    });
                    let k = moons.get(o.entity_id as u64 as usize % moons.len().max(1)).copied().unwrap_or(k);
                    found += usize::from(placed.take(k, o).is_some());
                }
            }
            let units = sim.view().units.len();
            println!("{path}: {units} sim units, {found}/{wanted} objects linked, {} unlinked, {} seats, {warnings} warnings", placed.rest().len(), setup.players.len());
            assert_eq!(found, setup.map.objects.len(), "{path}: every placed object finds its entity ({wanted} had a unit definition, the rest became regions or scenery)");
            maps += 1;

            // A ship moves when told to (the bench speaks for any seat).
            let view = sim.view();
            let ship = view.units.iter().find(|u| db.unit(u.def).is_some_and(|d| d.movement.max_speed > Fixed::ZERO));
            if let Some(ship) = ship {
                let to = SimVec3 { x: ship.pos.x + Fixed::from_int(2000), y: ship.pos.y, z: ship.pos.z };
                let cmd = Command { player: PlayerId::BENCH, seq: 0, kind: CommandKind::Order(Order::Move { units: vec![ship.id], to, queue: false }) };
                let start = ship.pos;
                let mut v = sim.step(&[cmd]).view;
                for _ in 0..300 {
                    v = sim.step(&[]).view;
                }
                let now = v.unit(ship.id).expect("ship still there").pos;
                assert!(now != start, "{path}: the ship moved");
            }
        }
        assert!(maps > 0, "no map to test on");

        // Two armed ships of different seats, in range, shoot.
        // "Armed" means a weapon that can hurt at a distance and fires within the window: mods
        // carry script-hook units (FO4's 8472active) whose only hardpoint is a dummy weapon
        // with a one-unit range and a refire of about a day, which never shoots by design.
        let armed: Vec<_> = db
            .units()
            .filter(|u| u.movement.max_speed > Fixed::ZERO && u.hull > Fixed::ZERO)
            .filter(|u| {
                u.hardpoints.iter().filter_map(|h| db.weapon(h.weapon.key)).any(|w| w.damage > Fixed::ZERO && w.range >= Fixed::from_int(50) && w.refire <= 450)
            })
            .collect();
        let a = armed.first().expect("an armed ship");
        let reach = a.hardpoints.iter().filter_map(|h| db.weapon(h.weapon.key)).map(|w| w.range).max().unwrap_or(Fixed::ZERO);
        let gap = (reach.to_f32() / 2.0).clamp(10.0, 300.0) as i32;
        let setup = GameSetup {
            seed: 3,
            map: Arc::new(MapDef::empty()),
            content: db.clone(),
            players: vec![
                PlayerSetup { id: PlayerId(0), team: TeamId(0), race: String::new(), control: Control::Human },
                PlayerSetup { id: PlayerId(1), team: TeamId(1), race: String::new(), control: Control::Idle },
            ],
            options: GameOptions::default(),
        };
        let mut sim = Sim::from_setup(&setup);
        let at = |x: i32| SimVec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::ZERO };
        let spawn = |seq, player, x| Command {
            player: PlayerId::BENCH,
            seq,
            kind: CommandKind::Edit(Edit::Spawn { odf: a.key, player: PlayerId(player), pos: at(x), heading: (Fixed::ONE, Fixed::ZERO) }),
        };
        let mut v = sim.step(&[spawn(0, 0, 0), spawn(1, 1, gap)]).view;
        let (first, second) = (v.units[0].id, v.units[1].id);
        let attack = |seq, by, target| Command { player: PlayerId::BENCH, seq, kind: CommandKind::Order(Order::Attack { units: vec![by], target }) };
        v = sim.step(&[attack(2, first, second), attack(3, second, first)]).view;
        let mut hits = 0;
        for _ in 0..900 {
            let out = sim.step(&[]);
            hits += out.events.iter().filter(|e| matches!(e, sta_sim::SimEvent::Hit { .. })).count();
            v = out.view;
        }
        println!("{}: {hits} hits in 30 s, units left {}", a.odf, v.units.len());
        assert!(hits > 0, "{}: armed ships in range hit each other", a.odf);
    }

    /// With `STA64_TEST_INSTALL` (+ optional `STA64_TEST_MOD`): every mobile builder in the
    /// install deploys a structure at a site abeam of it, however wide its turning circle.
    #[test]
    fn install_construction_ships_deploy_at_a_site_abeam() {
        let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
        let m = std::env::var("STA64_TEST_MOD").unwrap_or_default();
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
        let (index, _) = OdfIndex::build(&vfs);
        let (db, _) = ContentDb::load(&vfs, &index).unwrap();
        let db = Arc::new(db);
        let mut tried = 0;
        for b in db.units().filter(|u| u.movement.max_speed > Fixed::ZERO && u.movement.turn_omega > Fixed::ZERO) {
            let Some(what) = b.build.list.iter().find(|r| db.unit(r.key).is_some_and(|d| d.movement.max_speed == Fixed::ZERO && d.build.ticks > 0)) else { continue };
            let setup = GameSetup {
                seed: 5,
                map: Arc::new(MapDef::empty()),
                content: db.clone(),
                players: vec![PlayerSetup { id: PlayerId(0), team: TeamId(0), race: String::new(), control: Control::Human }],
                options: GameOptions::default(),
            };
            let mut sim = Sim::from_setup(&setup);
            let f = Fixed::from_int;
            let spawn = Command {
                player: PlayerId::BENCH,
                seq: 0,
                kind: CommandKind::Edit(Edit::Spawn { odf: b.key, player: PlayerId(0), pos: SimVec3 { x: f(0), y: f(0), z: f(0) }, heading: (f(0), f(1)) }),
            };
            let id = sim.step(&[spawn]).view.units[0].id;
            let rich = |r: u32| Command { player: PlayerId::BENCH, seq: 1 + r, kind: CommandKind::Edit(Edit::Cargo(PlayerId(0), r as u8, f(100_000))) };
            sim.step(&[rich(0), rich(1), rich(2), rich(3)]);
            let site = SimVec3 { x: f(400), y: f(0), z: f(0) };
            let order = Command { player: PlayerId::BENCH, seq: 9, kind: CommandKind::Order(Order::Build { builder: id, what: what.key, at: Some(site) }) };
            let first = sim.step(&[order]).events;
            if let Some(r) = first.iter().find(|e| matches!(e, sta_sim::SimEvent::Rejected { .. })) {
                println!("{}: {r:?}, skipped", b.odf);
                continue;
            }
            let mut started = first.iter().any(|e| matches!(e, sta_sim::SimEvent::BuildStarted { .. }));
            for _ in 0..3000 {
                if started {
                    break;
                }
                started = sim.step(&[]).events.iter().any(|e| matches!(e, sta_sim::SimEvent::BuildStarted { .. }));
            }
            println!("{}: reach {:?}, deploys {} -> {started}", b.odf, sta_sim::production::deploy_reach(&b.movement).to_f32(), what.odf);
            assert!(started, "{}: never deployed {}", b.odf, what.odf);
            tried += 1;
        }
        println!("{tried} builders tried");
    }
}
