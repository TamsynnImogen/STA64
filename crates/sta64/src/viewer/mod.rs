//! Model viewer: browse every model visible through the mounted install.
//!
//! Controls: Left/Right previous/next model, PageUp/PageDown jump 10, Home first,
//! T cycle team colour, H hull lighting (Refit look / Bevy PBR), B toggle Borg (`_b` textures, `borg` node), D toggle
//! damage-state nodes, left-drag orbit, right-drag pan, wheel zoom, R reset view.

use crate::render::model_scene::{self, SpawnCtx, TeamPart};
use crate::render::sprites::{self, SpriteTables};
use crate::render::textures::{Loader, TextureCache};
use bevy::image::CompressedImageFormats;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;

#[derive(Resource)]
pub struct Mounted(pub sta_vfs::Vfs);

#[derive(Resource)]
pub struct Browser {
    pub models: Vec<String>,
    pub index: usize,
    pub team: usize,
    pub borg: bool,
    pub damaged: bool,
    pub normal_maps: bool,
    /// Set when the shown model must be rebuilt.
    pub dirty: bool,
    pub mount_summary: String,
    pub info: String,
}

/// Where the engine finds `teamcolor.odf` (the exe names the bare file; the stock
/// A2 install and its mods carry it in `odf/other/`). Searched in this order, the
/// topmost mod layer first within each path.
pub const TEAMCOLOR_PATHS: [&str; 3] = ["odf/other/teamcolor.odf", "odf/system/teamcolor.odf", "odf/teamcolor.odf"];

/// The stock Armada II `odf/other/teamcolor.odf` palette, used when the install and
/// its mods carry none (Fleet Operations Roots, A1's archive format aside). Values are
/// the engine's team table as shipped: neutral, players 1-8 by file order, then the
/// race base colours. Federation's is a pale grey (0.9), the neutral `empty` 0.5.
pub const STOCK_TEAMCOLOR: &str = "empty = 0.5 0.5 0.5
white = 0.9 0.9 0.9
red = 1.0 0.0 0.0
blue = 0.0 0.4 1.0
green = 0.0 1.0 0.0
yellow = 0.9 0.9 0.0
purple = 0.686 0.0 1.0
cyan = 0.0 1.0 0.784
brown = 0.6 0.4 0.0
orange = 1.0 0.588 0.0
pink = 1.0 0.5 0.5
magenta = 1.0 0.0 1.0
gray = 0.5 0.5 0.5
black = 0.0 0.0 0.0
ferengi = 1.0 0.588 0.0
dominion = 0.8 0.2 1.0
klingon = 1.0 0.2 0.2
romulan = 0.4 1.0 0.4
borg = 0.0 0.6 0.4
federation = 0.9 0.9 0.9
cardassian = 0.9 0.6 0.0
breen = 0.0 0.7 0.0
sona = 0.8 0.8 0.8
alien = 0.8 0.8 0.8
";

/// Team colours from `teamcolor.odf` ([`TEAMCOLOR_PATHS`]), in file order: neutral,
/// teams 1-8, then race colours. The stock palette ([`STOCK_TEAMCOLOR`]) when absent.
/// The hull shader multiplies lit texels by the colour, as the engine does.
#[derive(Resource)]
pub struct TeamColours(pub Vec<(String, Color)>);

impl TeamColours {
    pub fn parse(text: &str) -> Vec<(String, Color)> {
        sta_odf::parse(text)
            .entries
            .into_iter()
            .filter_map(|e| {
                let v: Vec<f32> = e.value.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                (v.len() >= 3).then(|| (e.key, Color::srgb(v[0], v[1], v[2])))
            })
            .collect()
    }

    pub fn load(vfs: &sta_vfs::Vfs) -> TeamColours {
        let parsed = TEAMCOLOR_PATHS
            .iter()
            .filter_map(|p| vfs.read(p).and_then(Result::ok))
            .map(|b| Self::parse(&String::from_utf8_lossy(&b)))
            .find(|t| !t.is_empty());
        TeamColours(parsed.unwrap_or_else(|| Self::parse(STOCK_TEAMCOLOR)))
    }

    /// The viewer's starting colour: `white`, the first real team after the neutral
    /// `empty` entry (a pale grey, so hulls keep their own look), else the first entry.
    pub fn default_index(&self) -> usize {
        self.0.iter().position(|(n, _)| n.eq_ignore_ascii_case("white")).unwrap_or(0)
    }
}

#[derive(Component)]
struct ModelRoot;

#[derive(Component)]
struct Hud;

/// The main light, which L swings around the model.
#[derive(Component)]
struct KeyLight;

#[derive(Component)]
pub struct Orbit {
    pub focus: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    pub home: (Vec3, f32),
}

pub struct ViewerPlugin;

impl Plugin for ViewerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TextureCache>()
            .init_resource::<crate::render::particles::Particles>()
            .insert_resource(ClearColor(Color::srgb(0.01, 0.01, 0.025)))
            .insert_resource(GlobalAmbientLight {
                color: Color::WHITE,
                brightness: 60.0,
                ..default()
            })
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (
                    crate::clock::advance_idle_clock.run_if(not(resource_exists::<crate::sim_link::SimLink>)),
                    input,
                    swing_light,
                    rebuild,
                    orbit_camera,
                    hud,
                    model_scene::animate,
                    sprites::animate,
                    sprites::face_camera,
                    crate::render::particles::simulate,
                    crate::render::particles::draw,
                )
                    .chain()
                    .run_if(crate::platform::foreground),
            );
    }
}

/// Build the browser from the mounted VFS, starting at `initial` if given.
pub fn browser(vfs: &sta_vfs::Vfs, initial: Option<&str>, mount_summary: String) -> Browser {
    let models = sta_assets::list_models(vfs);
    let index = initial
        .and_then(|n| {
            let n = n.to_ascii_lowercase();
            let n = n.trim_end_matches(".sod");
            models.iter().position(|m| m == n)
        })
        .unwrap_or(0);
    Browser {
        models,
        index,
        team: 0,
        borg: false,
        damaged: false,
        normal_maps: true,
        dirty: true,
        mount_summary,
        info: String::new(),
    }
}

fn setup(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 0.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y),
        Orbit {
            focus: Vec3::ZERO,
            yaw: 0.6,
            pitch: 0.35,
            dist: 10.0,
            home: (Vec3::ZERO, 10.0),
        },
    ));
    commands.spawn((
        KeyLight,
        DirectionalLight {
            illuminance: 6000.0,
            ..default()
        },
        Transform::from_xyz(3.0, 5.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 1500.0,
            ..default()
        },
        Transform::from_xyz(-4.0, -2.0, -3.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        Hud,
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(15.0),
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

/// Hold L to swing the main light around the model (grazing light shows bump detail).
fn swing_light(keys: Res<ButtonInput<KeyCode>>, clock: Res<crate::clock::AnimClock>, mut q: Query<&mut Transform, With<KeyLight>>) {
    if !keys.pressed(KeyCode::KeyL) {
        return;
    }
    for mut tf in &mut q {
        tf.rotate_around(Vec3::ZERO, Quat::from_rotation_y(clock.frame_dt() * 1.2));
    }
}

#[allow(clippy::type_complexity)]
fn input(
    keys: Res<ButtonInput<KeyCode>>,
    mut browser: ResMut<Browser>,
    team_parts: Query<(Option<&MeshMaterial3d<StandardMaterial>>, Option<&crate::render::hulls::HullSource>), With<TeamPart>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut orbit: Query<&mut Orbit>,
    teams: Res<TeamColours>,
) {
    let n = browser.models.len();
    if n == 0 {
        return;
    }
    let step: isize = if keys.just_pressed(KeyCode::ArrowRight) {
        1
    } else if keys.just_pressed(KeyCode::ArrowLeft) {
        -1
    } else if keys.just_pressed(KeyCode::PageDown) {
        10
    } else if keys.just_pressed(KeyCode::PageUp) {
        -10
    } else {
        0
    };
    if step != 0 {
        browser.index = (browser.index as isize + step).rem_euclid(n as isize) as usize;
        browser.dirty = true;
    }
    if keys.just_pressed(KeyCode::Home) {
        browser.index = 0;
        browser.dirty = true;
    }
    if keys.just_pressed(KeyCode::KeyT) {
        browser.team = (browser.team + 1) % teams.0.len();
        let c = teams.0[browser.team].1;
        for (m, hull) in &team_parts {
            // A part drawn with the hull look keeps its standard material as the source.
            let Some(handle) = m.map(|m| &m.0).or(hull.map(|h| &h.source)) else { continue };
            if let Some(mut mat) = materials.get_mut(handle) {
                mat.base_color = c;
            }
        }
    }
    if keys.just_pressed(KeyCode::KeyB) {
        browser.borg = !browser.borg;
        browser.dirty = true;
    }
    if keys.just_pressed(KeyCode::KeyN) {
        browser.normal_maps = !browser.normal_maps;
        browser.dirty = true;
    }
    if keys.just_pressed(KeyCode::KeyD) {
        browser.damaged = !browser.damaged;
        browser.dirty = true;
    }
    if keys.just_pressed(KeyCode::KeyR) {
        for mut o in &mut orbit {
            (o.focus, o.dist) = o.home;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn rebuild(
    mut commands: Commands,
    mut browser: ResMut<Browser>,
    vfs: Res<Mounted>,
    roots: Query<Entity, With<ModelRoot>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut cache: ResMut<TextureCache>,
    device: Option<Res<RenderDevice>>,
    tables: Res<SpriteTables>,
    mut orbit: Query<&mut Orbit>,
    mut particles: ResMut<crate::render::particles::Particles>,
    teams: Res<TeamColours>,
) {
    if !browser.dirty {
        return;
    }
    browser.dirty = false;
    particles.clear();
    for e in &roots {
        commands.entity(e).despawn();
    }
    let Some(name) = browser.models.get(browser.index).cloned() else {
        browser.info = "No models found under sod/".into();
        return;
    };
    let total = browser.models.len();
    let header = format!("{name}  ({}/{total})", browser.index + 1);
    let Some((path, fmt)) = sta_assets::find_model(&vfs.0, &name) else {
        browser.info = format!("{header}\nnot found");
        return;
    };
    let model = match sta_assets::load_model(&vfs.0, &path, fmt) {
        Ok(m) => m,
        Err(e) => {
            browser.info = format!("{header}\n{e}");
            return;
        }
    };
    let formats = device
        .map(|d| CompressedImageFormats::from_features(d.features()))
        .unwrap_or(CompressedImageFormats::BC);
    let root = commands.spawn((ModelRoot, Transform::IDENTITY, Visibility::default())).id();
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
        team: teams.0[browser.team].1,
        borg: browser.borg,
        sprites: &tables.0,
        damaged: browser.damaged,
        normal_maps: browser.normal_maps,
        parts: None,
        model_key: name.clone(),
        unlit: false,
        scale: 1.0,
    };
    let stats = model_scene::spawn(&mut ctx, &model, root);

    let layer = vfs.0.resolve(&path).map(|(l, _)| vfs.0.layers()[l].name().to_owned()).unwrap_or_default();
    let mut info = format!(
        "{header}\n{path}  [{layer}]  {:?} v{:.2}\n{} nodes, {} mesh parts, {} triangles, {} bump-mapped meshes, {} animated nodes, radius {:.1}",
        model.format,
        model.version,
        model.nodes.len(),
        stats.meshes,
        stats.triangles,
        stats.bump_meshes,
        stats.animated_nodes,
        stats.radius
    );
    info.push_str(&format!(
        "\n{} sprite nodes, {} texture flipbooks, {} particle emitters",
        stats.sprite_nodes, stats.flipbooks, stats.emitters
    ));
    if !stats.unknown_sprite_nodes.is_empty() {
        info.push_str(&format!("\nunknown sprite nodes: {}", stats.unknown_sprite_nodes.join(", ")));
    }
    if !stats.missing_textures.is_empty() {
        info.push_str(&format!("\nmissing textures: {}", stats.missing_textures.join(", ")));
    }
    for w in model.warnings.iter().take(4) {
        info.push_str(&format!("\nnote: {w}"));
    }
    browser.info = info;

    for mut o in &mut orbit {
        o.focus = stats.centre;
        o.dist = stats.radius * 2.6;
        o.home = (o.focus, o.dist);
    }
}

/// Which mouse buttons orbit and pan the camera. The viewer uses left/right;
/// the map uses middle to orbit and keeps left/right for selecting and orders.
#[derive(Resource, Clone, Copy)]
pub struct OrbitButtons {
    pub rotate: MouseButton,
    pub pan: Option<MouseButton>,
}

pub fn orbit_camera(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    config: Option<Res<OrbitButtons>>,
    mut q: Query<(&mut Orbit, &mut Transform)>,
) {
    let cfg = config.map(|c| *c).unwrap_or(OrbitButtons {
        rotate: MouseButton::Left,
        pan: Some(MouseButton::Right),
    });
    for (mut o, mut tf) in &mut q {
        let d = motion.delta;
        if buttons.pressed(cfg.rotate) {
            o.yaw -= d.x * 0.008;
            o.pitch = (o.pitch + d.y * 0.008).clamp(-1.5, 1.5);
        }
        if cfg.pan.is_some_and(|b| buttons.pressed(b)) {
            let right = tf.right();
            let up = tf.up();
            let k = o.dist * 0.0015;
            o.focus += (-right * d.x + up * d.y) * k;
        }
        let lines = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
        };
        if lines != 0.0 {
            o.dist = (o.dist * 0.88f32.powf(lines)).max(0.01);
        }
        let rot = Quat::from_euler(EulerRot::YXZ, o.yaw, -o.pitch, 0.0);
        tf.translation = o.focus + rot * Vec3::new(0.0, 0.0, o.dist);
        tf.look_at(o.focus, Vec3::Y);
    }
}

fn hud(browser: Res<Browser>, teams: Res<TeamColours>, mut q: Query<&mut Text, With<Hud>>) {
    if !browser.is_changed() {
        return;
    }
    for mut t in &mut q {
        t.0 = format!(
            "STA64 {} model viewer\n{}\n\n{}\nteam colour: {}   Borg: {}   damage: {}   normal maps: {}\n\n\
             Left/Right model  PgUp/PgDn x10  Home first  T team colour  B Borg  D damage  N normal maps  H hull lighting A/B\n\
             hold L swing the light  \
             left-drag orbit  right-drag pan  wheel zoom  R reset view",
            env!("CARGO_PKG_VERSION"),
            browser.mount_summary,
            browser.info,
            teams.0[browser.team].0,
            if browser.borg { "on" } else { "off" },
            if browser.damaged { "shown" } else { "hidden" },
            if browser.normal_maps { "on" } else { "off" },
        );
    }
}

#[cfg(test)]
mod team_colour_tests {
    use super::*;

    #[test]
    fn stock_palette_is_the_fallback_and_federation_is_pale() {
        let t = TeamColours(TeamColours::parse(STOCK_TEAMCOLOR));
        assert_eq!(t.0[0].0, "empty");
        assert_eq!(t.0[3].0, "blue");
        assert_eq!(t.0[t.default_index()].0, "white");
        let fed = t.0.iter().find(|(n, _)| n == "federation").unwrap().1.to_srgba();
        assert!((fed.red - 0.9).abs() < 1e-6 && fed.red == fed.green && fed.green == fed.blue);
    }
}
