//! Map view: a BZN from the mounted install, every object placed with its model.
//!
//! Object to model: the SOD (or M3D, glTF) named like the object's ODF, scaled by
//! the ODF's `ScaleSOD`; `dont_load_SOD = 1` objects (asteroid fields) have no
//! model. The BZN team picks the object's entry in `teamcolor.odf`. The map's
//! `BackgroundArt` SOD is the skybox, drawn unlit around the camera.
//!
//! Controls: Left/Right map, PgUp/PgDn x10, WASD pan, left-drag orbit,
//! right-drag pan, wheel zoom, R reset view.

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

#[derive(Component)]
struct Sky;

#[derive(Component)]
struct MapHud;

type MapEntities = Or<(With<MapRoot>, With<Sky>)>;

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
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (
                    input,
                    rebuild,
                    pan_keys,
                    crate::viewer::orbit_camera,
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
        DirectionalLight {
            illuminance: 6000.0,
            ..default()
        },
        Transform::from_xyz(3.0, 5.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
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

/// WASD moves the camera's focus over the map plane.
fn pan_keys(keys: Res<ButtonInput<KeyCode>>, time: Res<Time>, mut q: Query<(&mut Orbit, &Transform)>) {
    for (mut o, tf) in &mut q {
        let fwd = Vec3::new(tf.forward().x, 0.0, tf.forward().z).normalize_or_zero();
        let right = Vec3::new(tf.right().x, 0.0, tf.right().z).normalize_or_zero();
        let mut d = Vec3::ZERO;
        for (k, v) in [(KeyCode::KeyW, fwd), (KeyCode::KeyS, -fwd), (KeyCode::KeyD, right), (KeyCode::KeyA, -right)] {
            if keys.pressed(k) {
                d += v;
            }
        }
        let step = d * o.dist * 0.8 * time.delta_secs();
        o.focus += step;
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
    mut ambient: ResMut<GlobalAmbientLight>,
    mut particles: ResMut<crate::particles::Particles>,
) {
    if !browser.dirty {
        return;
    }
    browser.dirty = false;
    particles.clear();
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
    let (mut drawn, mut no_model, mut no_odf, mut tris) = (0usize, 0usize, 0usize, 0usize);
    let mut missing: Vec<String> = Vec::new();

    for o in &map.objects {
        let Some(odf) = catalogue.odf(&vfs.0, &o.odf) else {
            no_odf += 1;
            missing.push(o.odf.clone());
            continue;
        };
        if odf.i64("dont_load_SOD").unwrap_or(0) != 0 {
            no_model += 1;
            continue;
        }
        let Some((path, model)) = catalogue.model(&vfs.0, &o.odf) else {
            no_model += 1;
            continue;
        };
        let scale = odf.f32("ScaleSOD").filter(|s| *s > 0.0).unwrap_or(1.0);
        let mut tf = to_bevy(&Matrix34 { m: o.transform });
        tf.scale *= scale;
        let entity = commands.spawn((tf, Visibility::default(), ChildOf(root))).id();
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
        };
        let stats = model_scene::spawn(&mut ctx, &model, entity);
        tris += stats.triangles;
        drawn += 1;
    }

    // Skybox: the BackgroundArt SOD around the camera, unlit, far out.
    let mut sky_note = String::from("none");
    if !map.background.is_empty() {
        sky_note = format!("{} (not found)", map.background);
        if let Some((path, model)) = catalogue.model(&vfs.0, &map.background) {
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
            };
            model_scene::spawn(&mut ctx, &model, sky);
            sky_note = path;
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

    missing.sort();
    missing.dedup();
    let mut info = format!(
        "{header}\n{}  v{}  {}\nsize {:.0} x {:.0}   race {}   tech tree {}   scripts {:?}\nsky {}   starfield {}\n{} objects: {} drawn, {} without a model, {} without an ODF   {} triangles",
        map.description,
        map.version,
        if map.binary { "binary" } else { "text" },
        map.size[0],
        map.size[2],
        map.race,
        map.tech_tree_file,
        map.scripts,
        sky_note,
        map.starfield_odf,
        map.objects.len(),
        drawn,
        no_model,
        no_odf,
        tris
    );
    if !missing.is_empty() {
        info.push_str(&format!("\nno ODF: {}", missing.iter().take(12).cloned().collect::<Vec<_>>().join(", ")));
    }
    browser.info = info;
}

fn hud(browser: Res<MapBrowser>, mut q: Query<&mut Text, With<MapHud>>) {
    if !browser.is_changed() {
        return;
    }
    for mut t in &mut q {
        t.0 = format!(
            "STA64 {} map view\n{}\n\n{}\n\nLeft/Right map  PgUp/PgDn x10  WASD pan  left-drag orbit  right-drag pan  wheel zoom  R reset",
            env!("CARGO_PKG_VERSION"),
            browser.mount_summary,
            browser.info
        );
    }
}
