//! Map view: a BZN from the mounted install, every object placed with its model.
//!
//! Object to model: the SOD (or M3D, glTF) named like the object's ODF, scaled by
//! the ODF's `ScaleSOD`; `dont_load_SOD = 1` objects (asteroid fields) have no
//! model. The BZN team picks the object's entry in `teamcolor.odf`. The map's
//! `BackgroundArt` SOD is the skybox, drawn unlit around the camera.
//!
//! Controls: left click/drag select, right click move, Space stop, Left/Right
//! map, PgUp/PgDn x10, WASD pan, middle-drag orbit, wheel zoom, R reset view.

use crate::model_scene::{self, PartCache, SpawnCtx, to_bevy};
use crate::sprites::SpriteTables;
use crate::textures::{Loader, TextureCache};
use crate::viewer::{Mounted, Orbit, TeamColours};
use bevy::camera::{PerspectiveProjection, Projection};
use bevy::image::CompressedImageFormats;
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use sta_assets::a2fmt::Matrix34;
use sta_assets::model::Model;
use std::collections::HashMap;
use std::sync::Arc;

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

/// The model an ODF draws: its `basename` (Fleet Operations names models this
/// way, e.g. `dcorvette` -> `dom_destroyer`), else the ODF's own name.
fn model_name(odf: &sta_odf::Odf, odf_name: &str) -> String {
    odf.str("basename").filter(|b| !b.trim().is_empty()).map(|b| b.trim().to_owned()).unwrap_or_else(|| odf_name.to_owned())
}

/// ODF lookups and loaded models, kept across maps.
#[derive(Resource, Default)]
struct Catalogue {
    odf_paths: HashMap<String, String>,
    odfs: HashMap<String, Option<Arc<sta_odf::Odf>>>,
    models: HashMap<String, Option<(String, Arc<Model>)>>,
}

impl Catalogue {
    fn index(&mut self, vfs: &sta_vfs::Vfs) {
        if !self.odf_paths.is_empty() {
            return;
        }
        // ODFs are found by file name across all ODF folders (and A1's addon/).
        for dir in ["odf", "addon"] {
            for p in vfs.list(dir) {
                if let Some(stem) = p.strip_suffix(".odf").map(|s| s.rsplit('/').next().unwrap_or(s).to_owned()) {
                    self.odf_paths.entry(stem).or_insert(p);
                }
            }
        }
    }

    fn odf(&mut self, vfs: &sta_vfs::Vfs, name: &str) -> Option<Arc<sta_odf::Odf>> {
        let key = name.to_ascii_lowercase();
        if let Some(o) = self.odfs.get(&key) {
            return o.clone();
        }
        let paths = &self.odf_paths;
        let mut load = |n: &str| {
            let stem = n.to_ascii_lowercase();
            let stem = stem.trim_end_matches(".odf");
            paths.get(stem).and_then(|p| vfs.read(p)).and_then(Result::ok).map(|b| String::from_utf8_lossy(&b).into_owned())
        };
        let odf = sta_odf::resolve(&key, &mut load).ok().map(Arc::new);
        self.odfs.insert(key, odf.clone());
        odf
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

#[derive(Component)]
struct Sky;

#[derive(Component)]
struct MapHud;

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

/// The ambient light and the two map lights, as one system parameter.
type Lighting<'w, 's> = (
    ResMut<'w, GlobalAmbientLight>,
    Query<'w, 's, (&'static MapLight, &'static mut Transform, &'static mut DirectionalLight), Without<Orbit>>,
);

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TextureCache>()
            .init_resource::<PartCache>()
            .init_resource::<Catalogue>()
            .init_resource::<crate::particles::Particles>()
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
            .add_plugins(crate::units::UnitsPlugin)
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (
                    input,
                    rebuild,
                    pan_keys,
                    crate::viewer::orbit_camera,
                    spin,
                    sky_follows_camera,
                    hud,
                    model_scene::animate,
                    crate::sprites::animate,
                    crate::sprites::face_camera,
                    crate::particles::simulate,
                    crate::particles::draw,
                )
                    .chain(),
            );
    }
}

fn setup(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            near: 1.0,
            far: 400_000.0,
            ..default()
        }),
        Transform::from_xyz(0.0, 3000.0, 6000.0).looking_at(Vec3::ZERO, Vec3::Y),
        Orbit {
            focus: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.9,
            dist: 8000.0,
            home: (Vec3::ZERO, 8000.0),
        },
    ));
    commands.spawn((
        MapLight(0),
        DirectionalLight {
            illuminance: 6000.0,
            ..default()
        },
        Transform::from_xyz(3.0, 5.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        MapLight(1),
        DirectionalLight {
            illuminance: 1500.0,
            ..default()
        },
        Transform::from_xyz(-4.0, -2.0, -3.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
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
            ..default()
        },
    ));
}

fn input(keys: Res<ButtonInput<KeyCode>>, mut browser: ResMut<MapBrowser>, mut orbit: Query<&mut Orbit>) {
    let n = browser.maps.len() as isize;
    if n == 0 {
        return;
    }
    let step = [
        (KeyCode::ArrowRight, 1),
        (KeyCode::ArrowLeft, -1),
        (KeyCode::PageDown, 10),
        (KeyCode::PageUp, -10),
    ]
    .iter()
    .find(|(k, _)| keys.just_pressed(*k))
    .map(|(_, s)| *s);
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

fn spin(time: Res<Time>, mut q: Query<(&Spin, &mut Transform)>) {
    for (s, mut t) in &mut q {
        t.rotate_local(Quat::from_axis_angle(s.axis, s.speed * time.delta_secs()));
    }
}

fn sky_follows_camera(camera: Query<&Transform, (With<Camera3d>, Without<Sky>)>, mut sky: Query<&mut Transform, With<Sky>>) {
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
    state: (ResMut<crate::particles::Particles>, ResMut<crate::units::Game>, ResMut<CameraLimits>),
    lighting: Lighting,
) {
    let (mut ambient, mut lights) = lighting;
    let (mut particles, mut game, mut limits) = state;
    if !browser.dirty {
        return;
    }
    browser.dirty = false;
    particles.clear();
    game.reset();
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
    catalogue.index(&vfs.0);
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
    let mut units = 0usize;
    let mut hidden = 0usize;

    for o in &map.objects {
        let Some(odf) = catalogue.odf(&vfs.0, &o.odf) else {
            no_odf += 1;
            missing.push(o.odf.clone());
            continue;
        };
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
                .model(&vfs.0, &model_name(&odf, &o.odf))
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
            missing.extend(crate::space::planet(&mut commands, &mut meshes, &mut materials, &mut loader, &ground, &atmo, radius, entity));
            planets += 1;
            drawn += 1;
            continue;
        }
        let Some((path, model)) = catalogue.model(&vfs.0, &model_name(&odf, &o.odf)) else {
            no_model += 1;
            continue;
        };
        let scale = odf.f32("ScaleSOD").filter(|s| *s > 0.0).unwrap_or(1.0);
        // ScaleSOD goes to the model's positions, not to the object's transform.
        let tf = to_bevy(&Matrix34 { m: o.transform });
        let entity = commands.spawn((tf, Visibility::default(), ChildOf(root))).id();
        // Craft with a physics ODF move: they become simulation units.
        if let Some(phys) = odf.str("physicsFile").filter(|p| !p.is_empty()).map(|p| p.trim_end_matches(".odf").to_owned())
            && let Some(ph) = catalogue.odf(&vfs.0, &phys)
            && ph.f32("impulseSpeed").is_some_and(|s| s > 0.0)
        {
            let stats = sta_sim::MoveStats::from_f32(
                ph.f32("impulseSpeed").unwrap_or(100.0),
                ph.f32("forwardAccel").unwrap_or(1.0),
                ph.f32("turnOmega").unwrap_or(1.0),
                ph.f32("turnAlpha").unwrap_or(1.0),
                ph.f32("turnOmegaFractionAtRest").unwrap_or(0.2),
            );
            let id = game.spawn(o.team, &o.transform, stats);
            let radius = model
                .bounds()
                .map(|(lo, hi)| (Vec3::from(hi) - Vec3::from(lo)).length() * 0.5)
                .unwrap_or(20.0)
                * scale;
            commands.entity(entity).insert(crate::units::Unit { id, radius: radius.max(5.0) });
            units += 1;
        }
        let team = if o.team > 0 { teams.0.get(o.team as usize).map(|t| t.1).unwrap_or(neutral) } else { neutral };
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
            let faces = crate::space::sky_faces(&mut commands, &mut meshes, &mut materials, &mut loader, &prefix, sky);
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
    let star_odf = map.starfield_odf.trim_end_matches(".odf").trim_end_matches(".ODF").to_owned();
    if let Some(sf) = catalogue.odf(&vfs.0, &star_odf) {
        let entries: Vec<(String, u32, f32)> = sf
            .keys()
            .filter(|k| !k.to_ascii_lowercase().ends_with(".scale"))
            .filter_map(|k| {
                let n = sf.i64(k)?;
                let scale = sf.f32(&format!("{k}.scale")).unwrap_or(1.0);
                (n > 0).then(|| (k.to_owned(), n as u32, scale))
            })
            .collect();
        let stars = commands.spawn((Sky, Transform::IDENTITY, Visibility::default())).id();
        let mut loader = Loader {
            vfs: &vfs.0,
            cache: &mut cache,
            images: &mut images,
            formats,
        };
        let n = crate::space::starfield(&mut commands, &mut meshes, &mut materials, &mut loader, &tables.0, &entries, 100_000.0, stars);
        star_note = format!("{} ({n} stars)", map.starfield_odf);
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
    let dist = size.x.max(size.z).max(2000.0) * 0.75;
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

    missing.sort();
    missing.dedup();
    let mut info = format!(
        "{header}\n{}  v{}  {}\nsize {:.0} x {:.0}   race {}   tech tree {}   scripts {:?}\nsky {}   starfield {}\n{} objects: {} drawn ({} ships under the sim, {} planets, {} asteroid rocks), {} editor markers ({} lights), {} without a model, {} without an ODF   {} triangles",
        map.description,
        map.version,
        if map.binary { "binary" } else { "text" },
        map.size[0],
        map.size[2],
        map.race,
        map.tech_tree_file,
        map.scripts,
        sky_note,
        star_note,
        map.objects.len(),
        drawn,
        units,
        planets,
        rocks,
        hidden,
        map_lights.len(),
        no_model,
        no_odf,
        tris
    );
    if !missing.is_empty() {
        info.push_str(&format!("\nnot found: {}", missing.iter().take(12).cloned().collect::<Vec<_>>().join(", ")));
    }
    browser.info = info;
}

fn hud(browser: Res<MapBrowser>, mut q: Query<&mut Text, With<MapHud>>) {
    if !browser.is_changed() {
        return;
    }
    for mut t in &mut q {
        t.0 = format!(
            "STA64 {} map view\n{}\n\n{}\n\nleft click/drag select (Shift adds)  right click move  Space stop\n\
             Left/Right map  PgUp/PgDn x10  WASD pan  middle-drag orbit  wheel zoom  R reset",
            env!("CARGO_PKG_VERSION"),
            browser.mount_summary,
            browser.info
        );
    }
}
