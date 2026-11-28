//! Frames from the install's own content for paired scenes (TOOL-17;
//! docs/TESTBENCH.md, "Paired scenes", "Frames from the install").
//!
//! `--headless --frames <ticks> --install-content` builds the scene on the
//! install's content (`sta_bench::pair::install_config`: the content database
//! and the scene's `Map=`, converted as the map view converts it) and draws it
//! with the **map view's own systems**, not a second loader:
//!
//! - [`crate::map::MapPlugin`] reads the BZN named by `Map=` and spawns what the
//!   client spawns for it: the map's own objects with their models, the sky
//!   cube or six faces, the starfield, the nebulae, planets and rock fields,
//!   the map's lights. It also builds its own sim of the map; the bench's sim
//!   (the map plus the scene's objects and commands) replaces it as the link's
//!   source (the link is fed the bench's views, [`adopt_views`]).
//! - [`crate::render::units::UnitsRenderPlugin`] draws every unit of the bench's
//!   views with its real hull (the scene's own units included), the effects
//!   plugins draw weapons, impacts and specials from the bench's events.
//! - The cameras the map view makes draw into the frame's offscreen image
//!   ([`retarget`]); the scene's `Camera=`/`orbit`/`camera` places the main one
//!   ([`place_camera`]), the same pose the stand-in frames use. The map view's
//!   text (frame rate, map info) is removed. `--frame-camera fly` moves that pose
//!   along a fixed path on the anim clock ([`fly_pose`]), so frames of a moving
//!   camera are the same on every run; `--frame-camera still|fly` also prints, at each
//!   frame's tick, how many billboard nebulae hold the eye ([`report_nebulae`]).
//!
//! No window, no display server and no audio device: the app is `sta64`'s
//! windowless headless app (an offscreen render target; `DefaultPlugins` without
//! winit and without Bevy's audio), and nothing here plays a sound. The frame
//! gate in [`super::frames::Capture`] holds the sim while a frame settles and
//! is read back, so a frame is exactly the requested tick.

use std::sync::Arc;

use bevy::camera::RenderTarget;
use bevy::prelude::*;
use sta_bench::parse::{BenchCommand, SceneFile};
use sta_bench::run::RunConfig;

use super::frames::{Capture, View};
use super::{BenchRes, bench_frame};
use crate::app::{FrameCamera, GameSet, LaunchConfig};
use crate::clock::AnimClock;
use crate::map::MatchEntity;
use crate::sim_link::{BenchDriven, SimLink};
use crate::viewer::{Mounted, TeamColours};

/// The bench's run config on the install `--install-root`/`--mod` name, with the
/// scene's `Map=` loaded.
pub(super) fn install_config(cfg: &LaunchConfig, scene: &SceneFile) -> Result<RunConfig, String> {
    let root = cfg.install_root.as_ref().ok_or("--install-content needs --install-root <install>")?;
    let maps: Vec<String> = scene.scene.map.iter().cloned().collect();
    sta_bench::pair::install_config(root, cfg.active_mod.as_deref(), &maps)
}

/// Everything `--install-content` adds to the headless frame app.
pub(super) fn add(app: &mut App, cfg: &LaunchConfig, scene_camera: Option<BenchCommand>) -> Result<(), String> {
    let root = cfg.install_root.as_ref().ok_or("--install-content needs --install-root <install>")?;
    let vfs = sta_vfs::Vfs::mount_install(root, cfg.active_mod.as_deref()).map_err(|e| format!("{}: {e:?}", root.display()))?;
    let text = std::fs::read_to_string(cfg.scene.as_deref().ok_or("--install-content needs --scene")?).map_err(|e| e.to_string())?;
    let scene = SceneFile::parse(&text).map_err(|e| e.to_string())?;

    let capture = Capture::new(cfg, &mut app.world_mut().resource_mut::<Assets<Image>>());
    app.insert_resource(Target(capture.target().clone())).insert_resource(capture).insert_resource(View::new(scene_camera.as_ref()));

    // What `--map <name>` inserts before the map view runs (lib.rs), from the same mount.
    let summary = format!("bench pair view: {} mod {}", root.display(), cfg.active_mod.as_deref().unwrap_or("-"));
    let maps = crate::map::browser(&vfs, scene.scene.map.as_deref(), summary);
    if scene.scene.map.as_deref().is_some_and(|m| maps.maps.get(maps.index).map(String::as_str) != Some(&m.to_ascii_lowercase())) {
        return Err(format!("Map={}: not found in the install's bzn/", scene.scene.map.as_deref().unwrap_or("")));
    }
    let rts = sta_content::layered::cfg_h(&vfs, "RTS_CFG.h");
    let table = sta_assets::load_sprite_table(&vfs);
    app.insert_resource(crate::render::sprites::SpriteTables(Arc::new(table)))
        .insert_resource(TeamColours::load(&vfs))
        .insert_resource(crate::map::MapSeat(None))
        .insert_resource(crate::map::CameraLimits::from_cfg(rts.as_ref()))
        .insert_resource(Mounted(vfs))
        .insert_resource(maps)
        // The bench steps its own sim; the link only holds the views it is fed.
        .insert_resource(BenchDriven);

    // The map view and what the client adds around it for a window (plugins.rs).
    app.add_plugins(crate::map::MapPlugin)
        .add_plugins(crate::render::units::UnitsRenderPlugin)
        .add_plugins(crate::render::fx::FxMaterialsPlugin)
        .add_plugins((
            crate::render::fx::weapons::WeaponFxPlugin,
            crate::render::fx::specials::SpecialsFxPlugin,
            crate::render::fx::impacts::ImpactFxPlugin,
        ))
        .add_plugins(crate::render::fog::FogPlugin)
        .add_plugins(crate::render::hulls::HullPlugin)
        .add_plugins(crate::render::shadows::ShadowPlugin)
        .add_plugins(crate::render::post::PostFxPlugin::from_cfg(cfg))
        .add_plugins((crate::render::cull::CullPlugin, crate::render::profile::ProfilePlugin, crate::render::shader_shadow::ShaderShadowPlugin));

    if let Some(view) = crate::render::wireframe::WireframeView::from_cfg(cfg) {
        app.add_plugins(crate::render::wireframe::WireframeViewPlugin(view));
    }

    if sprite_reference() {
        // ADR 0009 S6: the frame-identity run draws sprite nodes through `main`'s per-node
        // materials (adoption off, the switch the GPU test's reference path uses), to be compared
        // byte for byte with the default run, which draws them through the shared look materials.
        app.world_mut().resource_mut::<crate::render::sprites::material::SpriteLooks>().adopt = false;
        eprintln!("sprite nodes: per-node materials (reference path, {SPRITE_REFERENCE}=1)");
    }

    app.add_systems(Update, (adopt_views, aim_orbit).chain().in_set(GameSet::Bench).after(bench_frame))
        .add_systems(Update, (retarget, hide_text))
        .add_systems(PostUpdate, place_camera.before(TransformSystems::Propagate));
    if let Some(camera) = cfg.frame_camera {
        app.insert_resource(FrameCameraRun { fly: camera == FrameCamera::Fly, ticks: cfg.frames.clone(), reported: None })
            .add_systems(PostUpdate, report_nebulae.after(TransformSystems::Propagate));
    }
    Ok(())
}

/// `--frame-camera`: whether the pose flies, and the frames' ticks for the nebula report.
#[derive(Resource)]
pub(super) struct FrameCameraRun {
    fly: bool,
    ticks: Vec<u64>,
    reported: Option<u64>,
}

/// The scene's pose `(eye, look)` moved along `--frame-camera fly`'s path at anim time `t`
/// (seconds): the eye swings about the look point (up to 0.6 rad either way) while its distance
/// to it shrinks to a tenth and grows back, so the camera turns and moves on every frame the
/// clock advances and passes near the look point. At `t = 0` it is the scene's pose. Anim time
/// stands while the bench holds a frame, and in bench mode it is the same on every run, so the
/// frames are too.
pub fn fly_pose(eye: Vec3, look: Vec3, t: f32) -> (Vec3, Vec3) {
    let turn = Quat::from_rotation_y(0.6 * (0.9 * t).sin());
    let s = 0.55 + 0.45 * (0.7 * t).cos();
    (look + turn * ((eye - look) * s), look)
}

/// The scene's pose for this frame, flown when `--frame-camera fly` asks for it.
fn frame_pose(view: &View, bench: &sta_bench::run::Bench, run: Option<&FrameCameraRun>, clock: &AnimClock) -> Option<(Vec3, Vec3)> {
    let (eye, look) = view.pose(bench)?;
    Some(if run.is_some_and(|r| r.fly) { fly_pose(eye, look, clock.now() as f32) } else { (eye, look) })
}

/// At the first frame of each requested tick: the eye, and how many billboard nebulae
/// (`NebulaBoards`) hold it, with the nearest one's edge, on stderr. Tells a frame run whether
/// its camera is inside a nebula.
fn report_nebulae(
    bench: ResMut<BenchRes>,
    mut run: ResMut<FrameCameraRun>,
    main: Query<(&GlobalTransform, &Camera), With<Retargeted>>,
    nebulae: Query<(&GlobalTransform, &crate::render::nebula::NebulaBoards)>,
) {
    let mut bench = bench;
    let tick = bench.0.get().tick();
    if run.reported == Some(tick) || !run.ticks.contains(&tick) {
        return;
    }
    let Some((cam, _)) = main.iter().max_by_key(|(_, c)| c.order) else { return };
    run.reported = Some(tick);
    let eye = cam.translation();
    let mut inside = 0;
    let mut nearest: Option<(f32, Vec3, f32)> = None;
    for (g, b) in &nebulae {
        let (c, edge) = (g.translation(), g.translation().distance(eye) - b.radius);
        inside += usize::from(edge < 0.0);
        if nearest.is_none_or(|(e, _, _)| edge < e) {
            nearest = Some((edge, c, b.radius));
        }
    }
    let near = nearest.map_or_else(String::new, |(e, c, r)| format!("; nearest edge {e:.1} (centre {c}, radius {r})"));
    eprintln!("frame camera: tick {tick} eye {eye} inside {inside} of {} billboard nebulae{near}", nebulae.iter().count());
}

/// The variable that turns the reference path on in a headless install-content run (frames or a
/// performance run): `1` draws sprite nodes through their own `StandardMaterial`s, as before ADR
/// 0009, instead of the shared look materials. A bench switch for the frame-identity test
/// (`tests/sprite_frame_identity.rs`); the game never reads it.
pub const SPRITE_REFERENCE: &str = "STA64_SPRITE_REFERENCE";

fn sprite_reference() -> bool {
    std::env::var(SPRITE_REFERENCE).as_deref() == Ok("1")
}

/// The frame's offscreen image.
#[derive(Resource)]
struct Target(Handle<Image>);

/// Camera already pointed at the frame's image.
#[derive(Component)]
pub(super) struct Retargeted;

#[allow(clippy::type_complexity)]
/// The map view's window cameras draw into the frame's image, not a window.
fn retarget(mut commands: Commands, target: Res<Target>, mut cameras: Query<(Entity, &mut RenderTarget), (With<Camera3d>, Without<Retargeted>)>) {
    for (e, mut rt) in &mut cameras {
        // Only the cameras that draw to a window: the shadow pass has its own image target.
        if !matches!(*rt, RenderTarget::Window(_)) {
            continue;
        }
        *rt = RenderTarget::from(target.0.clone());
        commands.entity(e).insert(Retargeted);
    }
}

/// The map view's frame-rate and map-info texts are not part of a frame.
fn hide_text(mut commands: Commands, texts: Query<Entity, (With<MatchEntity>, With<Text>)>) {
    for e in &texts {
        commands.entity(e).despawn();
    }
}

/// The link the map view made holds the views of its own sim of the map; once it
/// exists (and after every tick, `bench_frame` feeds it as well) it is given the
/// bench's current view, so units, effects and the map's own objects (which the
/// bench's sim also holds, `MapObjects=keep`) are drawn from the bench's world.
fn adopt_views(bench: ResMut<BenchRes>, link: Option<ResMut<SimLink>>) {
    let Some(mut link) = link else { return };
    let mut bench = bench;
    let bench = bench.0.get();
    let view = bench.view();
    let production = bench.sim.production_view();
    let cargo = bench.sim.cargo_view();
    if !Arc::ptr_eq(link.real_curr(), view) || link.production() != production || link.cargo() != cargo {
        link.feed_with_production_and_cargo(view.clone(), production, cargo);
    }
}

/// The scene's camera as the map view's own orbit (focus, yaw, pitch, distance in render
/// space), so its camera system and the sky that follows the camera both use it.
fn aim_orbit(bench: ResMut<BenchRes>, view: Res<View>, run: Option<Res<FrameCameraRun>>, clock: Res<AnimClock>, mut orbit: Query<&mut crate::viewer::Orbit>) {
    let mut bench = bench;
    let Some((eye, look)) = frame_pose(&view, bench.0.get(), run.as_deref(), &clock) else { return };
    let d = eye - look;
    let dist = d.length();
    if dist < 1e-3 {
        return;
    }
    for mut o in &mut orbit {
        o.focus = look;
        o.dist = dist;
        o.pitch = (d.y / dist).clamp(-1.0, 1.0).asin();
        o.yaw = d.x.atan2(d.z);
    }
}

/// The scene's camera, on the map view's main camera, after its own orbit
/// controls have run (they stay on the map's default view when the scene sets
/// none).
pub(super) fn place_camera(
    bench: ResMut<BenchRes>,
    view: Res<View>,
    run: Option<Res<FrameCameraRun>>,
    clock: Res<AnimClock>,
    mut main: Query<(&mut Transform, &Camera), With<Retargeted>>,
) {
    let mut bench = bench;
    let bench = bench.0.get();
    let Some((eye, look)) = frame_pose(&view, bench, run.as_deref(), &clock) else { return };
    // The main camera is the one with the highest order (the background camera, a child
    // of it, draws first and follows it through the transform hierarchy).
    let Some(mut best) = main.iter_mut().max_by_key(|(_, c)| c.order) else { return };
    *best.0 = Transform::from_translation(eye).looking_at(look, Vec3::Y);
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_sim::WorldSource;

    /// The fly path starts at the scene's pose, moves and turns on every bench frame step, keeps
    /// looking at the same point, comes within a tenth of the distance of it and never closer, and
    /// gives the same pose for the same time.
    #[test]
    fn the_fly_path_moves_every_frame_from_the_scenes_pose() {
        let (eye, look) = (Vec3::new(300.0, 1500.0, -700.0), Vec3::new(100.0, 0.0, 200.0));
        assert_eq!(fly_pose(eye, look, 0.0), (eye, look));
        let dist = eye.distance(look);
        let (mut prev, mut closest) = (eye, dist);
        for frame in 1..1200 {
            let t = frame as f32 / 60.0;
            let (e, l) = fly_pose(eye, look, t);
            assert_eq!(l, look);
            assert_ne!(e, prev, "frame {frame}");
            assert_eq!(fly_pose(eye, look, t).0.to_array().map(f32::to_bits), e.to_array().map(f32::to_bits));
            closest = closest.min(e.distance(look));
            prev = e;
        }
        assert!(closest < dist * 0.11 && closest > dist * 0.099, "{closest} of {dist}");
    }

    /// With `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`, `STA64_TEST_MAP`, default the install's
    /// first map): the map view draws the BZN's objects against the ids of *its own* sim of
    /// the map (`map::build_sim`), and the bench's views (which replace that sim as the
    /// link's source) must name the same objects by the same ids, with the same definition,
    /// owner and place, or the map's ships would be drawn as another unit. Unit by unit.
    #[test]
    fn the_bench_sim_of_a_map_has_the_map_views_entity_ids() {
        let Some(found) = crate::testsupport::install_with_mod(sta_testkit::Kind::Any) else { return };
        let root = found.root_string();
        let mod_name = found.active_mod.clone();
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), mod_name.as_deref()).expect("mount");
        let Some(map_name) = sta_testkit::a_map(&vfs) else { return };
        let mut catalogue = crate::map::Catalogue::default();
        let c = catalogue.content(&vfs).expect("content");
        let (db, index) = (c.db.clone(), c.index.clone());
        let data = vfs.read(&format!("bzn/{map_name}.bzn")).or_else(|| vfs.read(&format!("bzn/{map_name}.sol"))).expect("map file").expect("read");
        let bzn = sta_bzn::parse(&data).expect("parse");
        let side = crate::map::build_sim(&bzn, &map_name, &db, &index, &vfs, None, None);
        let theirs = side.sim.view();

        let cfg = sta_bench::pair::install_config(std::path::Path::new(&root), mod_name.as_deref(), std::slice::from_ref(&map_name)).expect("config");
        let scene = SceneFile::parse(&format!("[Scene]\nMap={map_name}\nMapObjects=keep\n")).expect("scene");
        let bench = sta_bench::run::Bench::build(Arc::new(cfg), &scene).expect("bench");
        let ours = bench.view();

        assert!(!theirs.units.is_empty(), "{map_name} has units");
        let mut bad = Vec::new();
        for u in &theirs.units {
            match ours.unit(u.id) {
                Some(o) if o.def == u.def && o.owner == u.owner && o.pos == u.pos => {}
                other => bad.push((u.id, u.def, other.map(|o| o.def))),
            }
        }
        assert!(bad.is_empty(), "{} of {} units of {map_name} differ between the map view's sim and the bench's: {:?}", bad.len(), theirs.units.len(), &bad[..bad.len().min(5)]);
    }
}
