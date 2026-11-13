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
//!   text (frame rate, map info) is removed.
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
use crate::app::{GameSet, LaunchConfig};
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

    app.add_systems(Update, (adopt_views, aim_orbit).chain().in_set(GameSet::Bench).after(bench_frame))
        .add_systems(Update, (retarget, hide_text))
        .add_systems(PostUpdate, place_camera.before(TransformSystems::Propagate));
    Ok(())
}

/// The frame's offscreen image.
#[derive(Resource)]
struct Target(Handle<Image>);

/// Camera already pointed at the frame's image.
#[derive(Component)]
struct Retargeted;

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
    let view = bench.0.get().view();
    if !Arc::ptr_eq(link.curr(), view) {
        link.feed(view.clone());
    }
}

/// The scene's camera as the map view's own orbit (focus, yaw, pitch, distance in render
/// space), so its camera system and the sky that follows the camera both use it.
fn aim_orbit(bench: ResMut<BenchRes>, view: Res<View>, mut orbit: Query<&mut crate::viewer::Orbit>) {
    let mut bench = bench;
    let Some((eye, look)) = view.pose(bench.0.get()) else { return };
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
fn place_camera(bench: ResMut<BenchRes>, view: Res<View>, mut main: Query<(&mut Transform, &Camera), With<Retargeted>>) {
    let mut bench = bench;
    let bench = bench.0.get();
    let Some((eye, look)) = view.pose(bench) else { return };
    // The main camera is the one with the highest order (the background camera, a child
    // of it, draws first and follows it through the transform hierarchy).
    let Some(mut best) = main.iter_mut().max_by_key(|(_, c)| c.order) else { return };
    *best.0 = Transform::from_translation(eye).looking_at(look, Vec3::Y);
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_sim::WorldSource;

    /// With `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`, `STA64_TEST_MAP`, default the install's
    /// first map): the map view draws the BZN's objects against the ids of *its own* sim of
    /// the map (`map::build_sim`), and the bench's views (which replace that sim as the
    /// link's source) must name the same objects by the same ids, with the same definition,
    /// owner and place, or the map's ships would be drawn as another unit. Unit by unit.
    #[test]
    fn the_bench_sim_of_a_map_has_the_map_views_entity_ids() {
        let Ok(root) = std::env::var("STA64_TEST_INSTALL") else { return };
        let mod_name = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), mod_name.as_deref()).expect("mount");
        let map_name = std::env::var("STA64_TEST_MAP").unwrap_or_else(|_| crate::map::browser(&vfs, None, String::new()).maps[0].clone());
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
