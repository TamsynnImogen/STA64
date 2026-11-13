//! Scripted camera shots from mission presentation requests (UI-30).
//!
//! What a mission asks of the camera arrives as `SimEvent::Present` (see
//! `sta_sim::present`). This module consumes
//!
//! - `Presentation::Cineractive { on, .. }`: on takes the camera from the player
//!   (the map's own input systems still run, but their result is replaced every
//!   frame by the shot's pose) and remembers where it was; off gives it back there;
//! - `Presentation::Shot { from, at, seconds }` (A2 `Cineractive.lookFrom` / `lookAt`,
//!   A1 `CineractiveShot`): the camera looks from `from` at `at`. `seconds` 0 puts it
//!   there on the tick the request applies; more glides to it over that many seconds
//!   of real time (the glide is display only: the pose it ends in is the request's);
//! - `Call { name: "A2.CenterCamera" | "A1.CenterCamera", .. }` with an entity: the
//!   camera's focus moves to the entity, distance and angles kept (the call's name
//!   says what it does; it is not a shot and leaves the player in control).
//!
//! A pose is the map camera's own `Orbit` (focus, yaw, pitch, distance), which any
//! "look from here at there" is exactly; nothing here moves the camera any other way.
//!
//! **Not consumed, and listed once in [`Unhandled`]**: `CameraOffset` and the calls
//! `SetCameraZoom`, `CineractiveConstantSpeed`, `CineractiveCameraOffset` and
//! `CineractiveShot`: the LUA-05 notes (docs/LUA05_CAMERA_REQUESTS.md) keep their
//! numbers raw because what the offset's flag, the zoom's unit or a path float
//! mean was not established (not measured), and a guess here would put wrong
//! shots into every replay. A `Spot::Path` (a whole path travelled over the shot) is
//! the same. A `Spot::PathPoint` needs the map's paths: the host fills
//! [`ScenePaths`] (the map loader does not yet), and without the point the shot
//! is listed as unhandled.

use std::collections::BTreeMap;

use bevy::prelude::*;
use sta_sim::SimEvent;
use sta_sim::present::{Arg, Presentation, Spot};

use crate::sim_link::{SimLink, SimMsg};
use crate::ui::presentation::Unhandled;
use crate::viewer::Orbit;

/// Map paths by name: points in Bevy space (the host fills it from the map).
#[derive(Resource, Default, Debug, Clone)]
pub struct ScenePaths(pub BTreeMap<String, Vec<Vec3>>);

/// A camera pose, as the map's `Orbit` holds it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub focus: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
}

/// The pose that looks from `from` at `at` (Bevy space), as `Orbit` places the camera:
/// `from = focus + R(yaw, pitch) * (0, 0, dist)`.
pub fn pose_looking(from: Vec3, at: Vec3) -> Pose {
    let d = from - at;
    let flat = d.x.hypot(d.z);
    Pose { focus: at, yaw: d.x.atan2(d.z), pitch: d.y.atan2(flat).clamp(-1.5, 1.5), dist: d.length().max(0.01) }
}

/// Where `pose` puts the camera.
pub fn camera_translation(p: &Pose) -> Vec3 {
    p.focus + Quat::from_euler(EulerRot::YXZ, p.yaw, -p.pitch, 0.0) * Vec3::new(0.0, 0.0, p.dist)
}

fn of(o: &Orbit) -> Pose {
    Pose { focus: o.focus, yaw: o.yaw, pitch: o.pitch, dist: o.dist }
}

/// The scripted camera's state.
#[derive(Resource, Default, Debug, Clone)]
pub struct CinematicCamera {
    /// Cineractive mode is on: the shot, not the player, has the camera.
    pub locked: bool,
    /// The pose to give back when it ends.
    saved: Option<Pose>,
    /// The pose the camera is held at.
    pub target: Option<Pose>,
    /// A glide to `target`: from, and seconds left of `total`.
    glide: Option<(Pose, f32, f32)>,
    /// Shots that came before a pose could be computed, for tests and the bench.
    pub shots: u32,
}

fn lerp_pose(a: &Pose, b: &Pose, t: f32) -> Pose {
    let mut dyaw = (b.yaw - a.yaw) % std::f32::consts::TAU;
    if dyaw > std::f32::consts::PI {
        dyaw -= std::f32::consts::TAU;
    } else if dyaw < -std::f32::consts::PI {
        dyaw += std::f32::consts::TAU;
    }
    Pose { focus: a.focus.lerp(b.focus, t), yaw: a.yaw + dyaw * t, pitch: a.pitch + (b.pitch - a.pitch) * t, dist: a.dist + (b.dist - a.dist) * t }
}

/// A place in Bevy space, if it can be told.
fn spot(s: &Spot, link: Option<&SimLink>, paths: &ScenePaths) -> Option<Vec3> {
    match s {
        Spot::Entity(id) => {
            let u = link?.curr().unit(*id)?;
            Some(crate::render::units::to_bevy(u.pos.to_f32()))
        }
        Spot::Position(p) => Some(crate::render::units::to_bevy(p.to_f32())),
        Spot::PathPoint { path, index } => paths.0.get(path)?.get(*index as usize).copied(),
        Spot::Path(_) => None,
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn consume(
    mut events: MessageReader<SimMsg>,
    link: Option<Res<SimLink>>,
    paths: Res<ScenePaths>,
    mut cam: ResMut<CinematicCamera>,
    mut unhandled: ResMut<Unhandled>,
    mut orbit: Query<&mut Orbit>,
) {
    for SimMsg(e) in events.read() {
        let SimEvent::Present(p) = e else { continue };
        let current = orbit.iter().next().map(|o| of(&o));
        match p {
            Presentation::Cineractive { on, .. } => {
                if *on && !cam.locked {
                    cam.locked = true;
                    cam.saved = current;
                    cam.target = current;
                } else if !*on && cam.locked {
                    cam.locked = false;
                    cam.target = None;
                    cam.glide = None;
                    if let (Some(saved), Some(mut o)) = (cam.saved.take(), orbit.iter_mut().next()) {
                        (o.focus, o.yaw, o.pitch, o.dist) = (saved.focus, saved.yaw, saved.pitch, saved.dist);
                    }
                }
            }
            Presentation::Shot { from, at, seconds } => {
                let (Some(f), Some(a)) = (spot(from, link.as_deref(), &paths), spot(at, link.as_deref(), &paths)) else {
                    unhandled.note("Shot with a path spot or a vanished entity");
                    continue;
                };
                let pose = pose_looking(f, a);
                cam.shots += 1;
                let secs = seconds.to_f32();
                // A shot outside cineractive mode takes the camera until the next one
                // ends it; the A2 scripts always bracket shots in cineractive mode.
                if !cam.locked {
                    cam.locked = true;
                    cam.saved = current;
                }
                cam.glide = (secs > 0.0).then(|| (cam.target.or(current).unwrap_or(pose), secs, secs));
                cam.target = Some(pose);
            }
            Presentation::Call { name, args } if name == "A2.CenterCamera" || name == "A1.CenterCamera" => {
                let entity = args.iter().find_map(|a| if let Arg::Entity(id) = a { Some(*id) } else { None });
                match (entity.and_then(|id| spot(&Spot::Entity(id), link.as_deref(), &paths)), orbit.iter_mut().next()) {
                    (Some(at), Some(mut o)) => o.focus = at,
                    _ => {
                        unhandled.note(name);
                    }
                }
            }
            Presentation::CameraOffset { .. } => {
                unhandled.note("CameraOffset");
            }
            Presentation::Call { name, .. } if is_camera_call(name) => {
                unhandled.note(name);
            }
            _ => {}
        }
    }
}

/// Camera calls this module knows it does not act on (see the module docs).
fn is_camera_call(name: &str) -> bool {
    matches!(
        name.split_once('.').map_or(name, |x| x.1),
        "SetCameraZoom" | "CineractiveConstantSpeed" | "CineractiveCameraOffset" | "CineractiveShot" | "CenterCamera"
    )
}

/// While a shot holds the camera, put it at the shot's pose, over what input just did.
fn hold(time: Res<Time<Real>>, mut cam: ResMut<CinematicCamera>, mut q: Query<(&mut Orbit, &mut Transform)>) {
    if !cam.locked {
        return;
    }
    let Some(target) = cam.target else { return };
    let pose = match cam.glide {
        Some((from, left, total)) => {
            let left = (left - time.delta_secs()).max(0.0);
            cam.glide = (left > 0.0).then_some((from, left, total));
            lerp_pose(&from, &target, 1.0 - left / total)
        }
        None => target,
    };
    for (mut o, mut tf) in &mut q {
        (o.focus, o.yaw, o.pitch, o.dist) = (pose.focus, pose.yaw, pose.pitch, pose.dist);
        tf.translation = camera_translation(&pose);
        tf.look_at(pose.focus, Vec3::Y);
    }
}

pub struct CinematicCameraPlugin;

impl Plugin for CinematicCameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CinematicCamera>()
            .init_resource::<ScenePaths>()
            .init_resource::<Unhandled>()
            .add_systems(Update, (consume, hold).chain().after(crate::app::GameSet::SimLink).before(crate::app::GameSet::Render));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_core::Fixed;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_message::<SimMsg>().add_plugins(CinematicCameraPlugin);
        app.world_mut().spawn((
            Orbit { focus: Vec3::new(1.0, 0.0, 2.0), yaw: 0.3, pitch: 0.7, dist: 400.0, home: (Vec3::ZERO, 1.0) },
            Transform::default(),
        ));
        app
    }

    fn sv(x: i32, y: i32, z: i32) -> sta_core::Vec3 {
        sta_core::Vec3 { x: Fixed::from_int(x), y: Fixed::from_int(y), z: Fixed::from_int(z) }
    }

    fn send(app: &mut App, p: Presentation) {
        app.world_mut().write_message(SimMsg(SimEvent::Present(p)));
        app.update();
    }

    fn orbit(app: &mut App) -> Pose {
        of(app.world_mut().query::<&Orbit>().single(app.world()).unwrap())
    }

    #[test]
    fn pose_round_trips_through_the_orbit_placement() {
        for (from, at) in [(Vec3::new(10.0, 5.0, -3.0), Vec3::new(-2.0, 1.0, 4.0)), (Vec3::new(0.0, 100.0, 40.0), Vec3::ZERO), (Vec3::new(-50.0, -20.0, -50.0), Vec3::new(5.0, 5.0, 5.0))] {
            let p = pose_looking(from, at);
            assert!(camera_translation(&p).distance(from) < 1e-3, "{from:?}");
        }
    }

    #[test]
    fn a_shot_in_cineractive_mode_takes_the_camera_and_off_gives_it_back() {
        let mut app = app();
        let before = orbit(&mut app);
        send(&mut app, Presentation::Cineractive { on: true, skippable: false });
        let (from, at) = (sv(100, 20, 0), sta_core::Vec3::ZERO);
        send(&mut app, Presentation::Shot { from: Spot::Position(from), at: Spot::Position(at), seconds: Fixed::ZERO });
        let during = orbit(&mut app);
        assert_ne!(during, before);
        let tf = *app.world_mut().query::<&Transform>().single(app.world()).unwrap();
        // x is kept, z mirrored: the shot looks from +x, and the camera sits there.
        assert!(tf.translation.distance(Vec3::new(100.0, 20.0, 0.0)) < 1e-2, "{:?}", tf.translation);
        // The player's input cannot move it while locked.
        app.world_mut().query::<&mut Orbit>().single_mut(app.world_mut()).unwrap().dist = 5.0;
        app.update();
        assert_eq!(orbit(&mut app), during);
        send(&mut app, Presentation::Cineractive { on: false, skippable: false });
        assert_eq!(orbit(&mut app), before);
    }

    #[test]
    fn the_same_requests_give_the_same_pose() {
        let run = || {
            let mut app = app();
            send(&mut app, Presentation::Cineractive { on: true, skippable: true });
            let spot = |x| Spot::Position(sv(x, 7, -9));
            send(&mut app, Presentation::Shot { from: spot(30), at: spot(-4), seconds: Fixed::ZERO });
            orbit(&mut app)
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn requests_it_cannot_place_are_listed_once() {
        let mut app = app();
        send(&mut app, Presentation::Shot { from: Spot::Path("p".into()), at: Spot::Position(sta_core::Vec3::ZERO), seconds: Fixed::ZERO });
        send(&mut app, Presentation::Call { name: "A2.SetCameraZoom".into(), args: vec![Arg::Num(Fixed::from_int(2))] });
        send(&mut app, Presentation::Call { name: "A2.SetCameraZoom".into(), args: vec![Arg::Num(Fixed::from_int(3))] });
        let u = app.world().resource::<Unhandled>();
        assert_eq!(u.count("A2.SetCameraZoom"), 2);
        assert!(!app.world().resource::<CinematicCamera>().locked);
    }

    #[test]
    fn a_path_point_shot_uses_the_hosts_paths() {
        let mut app = app();
        app.world_mut().resource_mut::<ScenePaths>().0.insert("cam".into(), vec![Vec3::new(0.0, 0.0, 50.0), Vec3::new(10.0, 10.0, 10.0)]);
        send(&mut app, Presentation::Shot { from: Spot::PathPoint { path: "cam".into(), index: 1 }, at: Spot::PathPoint { path: "cam".into(), index: 0 }, seconds: Fixed::ZERO });
        let tf = *app.world_mut().query::<&Transform>().single(app.world()).unwrap();
        assert!(tf.translation.distance(Vec3::new(10.0, 10.0, 10.0)) < 1e-2);
    }
}
