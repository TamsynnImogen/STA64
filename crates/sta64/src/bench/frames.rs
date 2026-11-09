//! Frame capture for `--headless --frames <ticks>` (docs/TESTBENCH.md, "Frames").
//!
//! A windowless render app: one camera draws into an offscreen [`Image`], and
//! a [`Screenshot`] of that image is written as a PNG when the bench reaches
//! each requested tick. The bench holds the sim still while a frame settles
//! and is read back, so the frame is of exactly that tick whatever the GPU's
//! speed, and the same scene gives the same pixels on every run.
//!
//! What is drawn are stand-in shapes (a sphere for anything the sim calls a
//! resource, a box for the rest, in the owner's colour) placed from the bench
//! `WorldView`: the bench runs on fake content and has no install to load
//! models from. The stand-ins exercise the whole capture path (clock,
//! offscreen target, readback, encoding); the game's own renderers join the
//! headless app when they can run without an install.
//!
//! No adapter (no GPU, no software Vulkan): `sta64` exits [`NO_ADAPTER`] and
//! says so, and callers skip rather than fail.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
use sta_bench::parse::{BenchCommand, Target, parse_command};
use sta_sim::{EntityId, Tick};

use super::BenchRes;
use crate::app::LaunchConfig;

/// Exit code for "no GPU adapter here"; the test bench treats it as a skip.
pub const NO_ADAPTER: i32 = 77;

/// Is this a CPU/software rasteriser (WARP, llvmpipe, SwiftShader, ...)?
/// Those render frames far too slowly to be a useful bench, so they count as
/// "no GPU".
pub fn is_software_adapter(kind: impl std::fmt::Debug, name: &str) -> bool {
    // bevy does not re-export wgpu's `DeviceType`; its Debug name is `Cpu`.
    let n = name.to_ascii_lowercase();
    format!("{kind:?}") == "Cpu"
        || ["warp", "llvmpipe", "swiftshader", "lavapipe", "microsoft basic render"].iter().any(|s| n.contains(s))
}

/// Frames held at tick 0 before anything is captured, so the first draw is
/// not racing asset and pipeline setup.
const WARMUP: u32 = 5;
/// Frames the picture is held still before a capture is requested.
const SETTLE: u32 = 3;

/// Where the bench's frames go and when.
#[derive(Resource)]
pub struct Capture {
    ticks: Vec<Tick>,
    dir: PathBuf,
    target: Handle<Image>,
    next: usize,
    warmup: u32,
    settle: u32,
    waiting: bool,
    saved: Arc<AtomicUsize>,
}

/// What the capture lets this frame do.
pub enum Gate {
    /// Run ticks, but stop at this one.
    Run(Tick),
    /// Do not tick: a frame is settling or being read back.
    Hold,
}

impl Capture {
    pub fn new(cfg: &LaunchConfig, images: &mut Assets<Image>) -> Capture {
        let (w, h) = cfg.frame_size.unwrap_or((640, 360));
        let mut img = Image::new_target_texture(w, h, TextureFormat::Rgba8UnormSrgb, None);
        img.asset_usage = RenderAssetUsages::RENDER_WORLD;
        let mut ticks = cfg.frames.clone();
        ticks.sort_unstable();
        ticks.dedup();
        Capture {
            ticks,
            dir: cfg.frame_dir.clone().unwrap_or_else(|| PathBuf::from("frames")),
            target: images.add(img),
            next: 0,
            warmup: WARMUP,
            settle: SETTLE,
            waiting: false,
            saved: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// The last tick a frame is asked at.
    pub fn last_tick(&self) -> Tick {
        self.ticks.last().copied().unwrap_or(0)
    }

    pub fn done(&self) -> bool {
        self.next >= self.ticks.len()
    }

    /// Decide this frame; spawns the screenshot when its tick is reached.
    pub fn gate(&mut self, tick: Tick, commands: &mut Commands) -> Gate {
        if self.warmup > 0 {
            self.warmup -= 1;
            return Gate::Hold;
        }
        if self.waiting {
            if self.saved.load(Ordering::Acquire) > self.next {
                self.waiting = false;
                self.next += 1;
                self.settle = SETTLE;
            } else {
                return Gate::Hold;
            }
        }
        let Some(&at) = self.ticks.get(self.next) else {
            return Gate::Run(Tick::MAX);
        };
        if tick < at {
            return Gate::Run(at);
        }
        if self.settle > 0 {
            self.settle -= 1;
            return Gate::Hold;
        }
        let path = self.dir.join(format!("tick-{at:06}.png"));
        let _ = std::fs::create_dir_all(&self.dir);
        let saved = self.saved.clone();
        let shown = path.clone();
        commands
            .spawn(Screenshot::image(self.target.clone()))
            .observe(save_to_disk(path))
            .observe(move |c: On<ScreenshotCaptured>| {
                println!("frame tick {at} {} pixels {:016x}", shown.display(), fnv(c.image.data.as_deref().unwrap_or(&[])));
                saved.fetch_add(1, Ordering::Release);
            });
        self.waiting = true;
        Gate::Hold
    }
}

/// FNV-1a over the pixels: the "same frame" check the runner prints.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// The proxy camera, from the scene's `Camera=` and later `orbit` / `camera`.
#[derive(Resource)]
pub struct View {
    orbit: Option<(Target, f32, f32, f32)>,
    eye: Option<(Vec3, Target)>,
}

impl View {
    pub fn new(scene_camera: Option<&BenchCommand>) -> View {
        let mut v = View { orbit: None, eye: None };
        if let Some(c) = scene_camera {
            v.apply(c);
        }
        v
    }

    fn apply(&mut self, c: &BenchCommand) {
        match c {
            BenchCommand::Orbit { target, yaw, pitch, distance } => {
                self.orbit = Some((target.clone(), *yaw as f32, *pitch as f32, *distance as f32));
                self.eye = None;
            }
            BenchCommand::Camera { eye, look } => {
                self.eye = Some((Vec3::new(eye[0] as f32, eye[1] as f32, eye[2] as f32), look.clone()));
                self.orbit = None;
            }
            _ => {}
        }
    }

    /// Note a command line's effect on the camera (the bench itself answers
    /// "ok (no renderer at tier 1)" for these).
    pub fn observe(&mut self, line: &str) {
        if let Ok(Some(c)) = parse_command(line) {
            self.apply(&c.command);
        }
    }
}

/// A stand-in for one sim object.
#[derive(Component)]
struct Proxy;

#[derive(Resource, Default)]
struct Proxies(BTreeMap<EntityId, Entity>);

#[derive(Component)]
struct FrameCamera;

fn fixed(f: sta_sim::Fixed) -> f32 {
    (f.to_bits() as f64 / 4_294_967_296.0) as f32
}

pub fn add(app: &mut App, cfg: &LaunchConfig, scene_camera: Option<BenchCommand>) {
    let capture = Capture::new(cfg, &mut app.world_mut().resource_mut::<Assets<Image>>());
    let target = capture.target.clone();
    app.insert_resource(capture)
        .insert_resource(View::new(scene_camera.as_ref()))
        .init_resource::<Proxies>()
        .insert_resource(GlobalAmbientLight { brightness: 300.0, ..default() })
        .add_systems(Startup, move |mut commands: Commands| {
            commands.spawn((Camera3d::default(), RenderTarget::from(target.clone()), Msaa::Off, FrameCamera, Transform::default()));
            commands.spawn((
                DirectionalLight { illuminance: 8000.0, shadow_maps_enabled: false, ..default() },
                Transform::from_xyz(1.0, 2.0, 1.5).looking_at(Vec3::ZERO, Vec3::Y),
            ));
        })
        .add_systems(Update, sync_world.after(crate::app::GameSet::Bench));
}

/// `STA64_UI_RADAR=1`: the UI-05 radar over the frame, fed from the bench's
/// view (player 1, the scenes' own team, with the bench's fake content).
pub fn add_radar(app: &mut App, cfg: &LaunchConfig) {
    use crate::ui::radar::{RadarFeed, add_frame_overlay};
    let target = app.world().resource::<Capture>().target.clone();
    add_frame_overlay(app, target, cfg.frame_size.unwrap_or((640, 360)).1);
    let content = Arc::new(sta_bench::run::fake_content());
    app.add_systems(
        Update,
        (move |bench: ResMut<BenchRes>, mut feed: ResMut<RadarFeed>| {
            let mut bench = bench;
            feed.view = bench.0.get().view().clone();
            feed.player = sta_sim::PlayerId(1);
            feed.content = Some(content.clone());
        })
        .after(crate::app::GameSet::Bench)
        .before(crate::app::GameSet::Ui),
    );
}

#[allow(clippy::too_many_arguments)]
fn sync_world(
    mut commands: Commands,
    bench: ResMut<BenchRes>,
    view: Res<View>,
    mut proxies: ResMut<Proxies>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut transforms: Query<&mut Transform, (With<Proxy>, Without<FrameCamera>)>,
    mut camera: Query<&mut Transform, With<FrameCamera>>,
) {
    let mut bench = bench;
    let bench = bench.0.get();
    let world = bench.view().clone();
    let alive: Vec<EntityId> = world.units.iter().map(|u| u.id).collect();
    proxies.0.retain(|id, e| {
        let keep = alive.contains(id);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    let mut at: BTreeMap<EntityId, Vec3> = BTreeMap::new();
    for u in &world.units {
        let pos = Vec3::new(fixed(u.pos.x), fixed(u.pos.y), fixed(u.pos.z));
        at.insert(u.id, pos);
        let yaw = fixed(u.heading.0).atan2(fixed(u.heading.1));
        let t = Transform::from_translation(pos).with_rotation(Quat::from_rotation_y(yaw));
        if let Some(&e) = proxies.0.get(&u.id) {
            if let Ok(mut tr) = transforms.get_mut(e) {
                *tr = t;
            }
            continue;
        }
        let is_body = bench.sim.unit(u.id).is_some_and(|e| format!("{:?}", e.kind) == "Resource");
        let (mesh, colour) = if is_body {
            (meshes.add(Sphere::new(250.0).mesh().ico(5).unwrap()), Color::srgb(0.35, 0.55, 0.75))
        } else {
            let hue = 360.0 * f32::from(u.owner.0 % 8) / 8.0;
            (meshes.add(Cuboid::new(60.0, 30.0, 140.0)), Color::hsl(hue, 0.6, 0.55))
        };
        let e = commands
            .spawn((Proxy, Mesh3d(mesh), MeshMaterial3d(materials.add(colour)), t))
            .id();
        proxies.0.insert(u.id, e);
    }
    let resolve = |t: &Target| match t {
        Target::Point(p) => Some(Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32)),
        Target::Object(n) => bench.resolve(n).ok().and_then(|id| at.get(&id).copied()),
    };
    let Ok(mut cam) = camera.single_mut() else { return };
    let placed = if let Some((target, yaw, pitch, dist)) = &view.orbit {
        resolve(target).map(|c| {
            let (y, p) = (yaw.to_radians(), pitch.to_radians());
            (c + *dist * Vec3::new(y.sin() * p.cos(), p.sin(), y.cos() * p.cos()), c)
        })
    } else if let Some((eye, look)) = &view.eye {
        resolve(look).map(|c| (*eye, c))
    } else {
        Some((Vec3::new(0.0, 600.0, 900.0), Vec3::ZERO))
    };
    if let Some((eye, look)) = placed {
        *cam = Transform::from_translation(eye).looking_at(look, Vec3::Y);
    }
}
