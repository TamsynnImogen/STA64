//! Model viewer: browse every model visible through the mounted install.
//!
//! Controls: Left/Right previous/next model, PageUp/PageDown jump 10, Home first,
//! T cycle team colour, B toggle Borg `_b` textures, left-drag orbit, right-drag pan, wheel zoom, R reset view.

use crate::sod_scene::{self, SpawnCtx, TeamPart};
use crate::sprites::{self, SpriteTables};
use crate::textures::{Loader, TextureCache};
use bevy::image::CompressedImageFormats;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use sta_assets::ModelFormat;

#[derive(Resource)]
pub struct Mounted(pub sta_vfs::Vfs);

#[derive(Resource)]
pub struct Browser {
    pub models: Vec<String>,
    pub index: usize,
    pub team: usize,
    pub borg: bool,
    /// Set when the shown model must be rebuilt.
    pub dirty: bool,
    pub mount_summary: String,
    pub info: String,
}

const TEAM_COLOURS: [(&str, Color); 6] = [
    ("blue", Color::srgb(0.25, 0.45, 1.0)),
    ("red", Color::srgb(1.0, 0.2, 0.15)),
    ("green", Color::srgb(0.2, 0.9, 0.3)),
    ("yellow", Color::srgb(1.0, 0.85, 0.2)),
    ("purple", Color::srgb(0.7, 0.3, 1.0)),
    ("white", Color::WHITE),
];

#[derive(Component)]
struct ModelRoot;

#[derive(Component)]
struct Hud;

#[derive(Component)]
struct Orbit {
    focus: Vec3,
    yaw: f32,
    pitch: f32,
    dist: f32,
    home: (Vec3, f32),
}

pub struct ViewerPlugin;

impl Plugin for ViewerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TextureCache>()
            .insert_resource(ClearColor(Color::srgb(0.01, 0.01, 0.025)))
            .insert_resource(GlobalAmbientLight {
                color: Color::WHITE,
                brightness: 250.0,
                ..default()
            })
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (input, rebuild, orbit_camera, hud, sod_scene::animate, sprites::animate, sprites::face_camera).chain(),
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

fn input(
    keys: Res<ButtonInput<KeyCode>>,
    mut browser: ResMut<Browser>,
    mut team_parts: Query<&MeshMaterial3d<StandardMaterial>, With<TeamPart>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut orbit: Query<&mut Orbit>,
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
        browser.team = (browser.team + 1) % TEAM_COLOURS.len();
        let c = TEAM_COLOURS[browser.team].1;
        for m in &mut team_parts {
            if let Some(mut mat) = materials.get_mut(&m.0) {
                mat.base_color = c;
            }
        }
    }
    if keys.just_pressed(KeyCode::KeyB) {
        browser.borg = !browser.borg;
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
) {
    if !browser.dirty {
        return;
    }
    browser.dirty = false;
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
    if fmt != ModelFormat::Sod {
        browser.info = format!("{header}\n{path}\n{fmt:?} models are not wired into the viewer yet");
        return;
    }
    let sod = match sta_assets::load_sod(&vfs.0, &path) {
        Ok(s) => s,
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
        team: TEAM_COLOURS[browser.team].1,
        borg: browser.borg,
        sprites: &tables.0,
    };
    let stats = sod_scene::spawn(&mut ctx, &sod, root);

    let layer = vfs.0.resolve(&path).map(|(l, _)| vfs.0.layers()[l].name().to_owned()).unwrap_or_default();
    let mut info = format!(
        "{header}\n{path}  [{layer}]  {:?} v{:.2}\n{} nodes, {} mesh parts, {} triangles, {} bump-mapped meshes, {} animated nodes, radius {:.1}",
        sod.container,
        sod.version,
        sod.nodes.len(),
        stats.meshes,
        stats.triangles,
        stats.bump_meshes,
        stats.animated_nodes,
        stats.radius
    );
    info.push_str(&format!(
        "\n{} sprite nodes, {} texture flipbooks, {} particle emitters (not drawn yet)",
        stats.sprite_nodes, stats.flipbooks, stats.emitters
    ));
    if !stats.unknown_sprite_nodes.is_empty() {
        info.push_str(&format!("\nunknown sprite nodes: {}", stats.unknown_sprite_nodes.join(", ")));
    }
    if !stats.missing_textures.is_empty() {
        info.push_str(&format!("\nmissing textures: {}", stats.missing_textures.join(", ")));
    }
    browser.info = info;

    for mut o in &mut orbit {
        o.focus = stats.centre;
        o.dist = stats.radius * 2.6;
        o.home = (o.focus, o.dist);
    }
}

fn orbit_camera(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut q: Query<(&mut Orbit, &mut Transform)>,
) {
    for (mut o, mut tf) in &mut q {
        let d = motion.delta;
        if buttons.pressed(MouseButton::Left) {
            o.yaw -= d.x * 0.008;
            o.pitch = (o.pitch + d.y * 0.008).clamp(-1.5, 1.5);
        }
        if buttons.pressed(MouseButton::Right) {
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

fn hud(browser: Res<Browser>, mut q: Query<&mut Text, With<Hud>>) {
    if !browser.is_changed() {
        return;
    }
    for mut t in &mut q {
        t.0 = format!(
            "STA64 {} model viewer\n{}\n\n{}\nteam colour: {}   Borg textures: {}\n\n\
             Left/Right model  PgUp/PgDn x10  Home first  T team colour  B Borg textures\n\
             left-drag orbit  right-drag pan  wheel zoom  R reset view",
            env!("CARGO_PKG_VERSION"),
            browser.mount_summary,
            browser.info,
            TEAM_COLOURS[browser.team].0,
            if browser.borg { "on" } else { "off" },
        );
    }
}
