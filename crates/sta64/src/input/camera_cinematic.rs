//! Scripted camera shots from mission presentation requests (UI-30, UI-36).
//!
//! What a mission asks of the camera arrives as `SimEvent::Present` (see
//! `sta_sim::present`). Every request goes through the same shot model the mission
//! runtime keeps ([`sta_mission::cineractive::Cineractive`]): the A1/A2 `CineractiveShot`
//! and `CineractiveCameraOffset` calls, A2 `CineractiveConstantSpeed`, the MMM
//! `Cineractive:lookFrom` / `lookAt` / `setCameraOffset` requests. So the camera plays
//! the shot the script's `isCineractiveDone` waits on, on the same sim ticks: eye and
//! target follow their path (segments per second of game time, Hermite curve), object
//! or point, the offset is applied in the eye's frame, and a new shot cuts.
//!
//! - `Presentation::Cineractive { on }`: on takes the camera from the player (the map's
//!   own input systems still run, but their result is replaced every frame by the shot's
//!   pose) and remembers where it was; off gives it back there: the original's
//!   `CineractiveEnd` pops its view, so the strategy camera returns exactly as it was
//!   (A1 and A2 executables, static). Before the script places its first shot, the camera
//!   holds where it was.
//! - [`ScriptReply::CineractiveReleased`] (the host's forced end: the caps of
//!   `ui::presentation`, a script fault, the mission's end) gives it back too. A player's
//!   skip ([`ScriptReply::CineractiveSkipped`]) does not: it sets `isCineractiveCancel`
//!   and the script ends the cineractive (fed1 plays its zoom-out first).
//! - A `Presentation::Shot` outside a cineractive moves the camera once (at most once a
//!   second) and holds nothing; `A2.CenterCamera(entity)` moves the focus there.
//!
//! The paths are the map's AiPaths in game space ([`ScenePaths`], filled by the mission
//! host from the launch). A shot is timed from the tick the script raised it (one tick
//! before the view that carries its event) and placed at the interpolated sim time. A frame
//! that runs several ticks at once stamps their requests with its last tick (`SimMsg` carries
//! no tick): the camera then trails the script's timing by those ticks, display only.
//!
//! **Not consumed, listed once in [`Unhandled`]**: `SetCameraZoom` (A2 `SetZoomLevel`,
//! not wired to the orbit camera), offsets outside a cineractive.

use bevy::prelude::*;
use sta_mission::cineractive::{Cineractive, V3};
use sta_sim::SimEvent;
use sta_sim::present::{Arg, Presentation, Spot};

use crate::sim_link::{SimLink, SimMsg};
use crate::ui::presentation::{ScriptReply, Unhandled};
use crate::viewer::Orbit;

/// The map's named paths in game space, file order (the mission's `A2.paths` / `A1` paths).
#[derive(Resource, Default, Debug, Clone)]
pub struct ScenePaths(pub Vec<(String, Vec<[f32; 3]>)>);

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
    /// The pose the camera is held at (the last one the shots gave).
    pub target: Option<Pose>,
    /// Shots placed (cuts and moves), for tests and the bench.
    pub shots: u32,
    /// When (real seconds) the last shot outside a cineractive moved the camera.
    loose_at: Option<f32>,
    /// The shot model, fed the same requests as the mission's.
    pub model: Cineractive,
}

/// A shot outside a cineractive moves the camera at most this often (real seconds): a script that
/// issues one every tick would otherwise pin the camera with nothing the player could skip.
const LOOSE_SHOT_EVERY_SECS: f32 = 1.0;

fn bevy(p: V3) -> Vec3 {
    crate::render::units::to_bevy([p[0] as f32, p[1] as f32, p[2] as f32])
}

/// A place in Bevy space, if it can be told (shots outside a cineractive).
fn spot(s: &Spot, link: Option<&SimLink>, paths: &ScenePaths) -> Option<Vec3> {
    match s {
        Spot::Entity(id) => {
            let u = link?.curr().unit(*id)?;
            Some(crate::render::units::to_bevy(u.pos.to_f32()))
        }
        Spot::Position(p) => Some(crate::render::units::to_bevy(p.to_f32())),
        Spot::PathPoint { path, index } => paths.0.iter().find(|(n, _)| n == path)?.1.get(*index as usize).map(|p| crate::render::units::to_bevy(*p)),
        _ => None,
    }
}

fn release(cam: &mut CinematicCamera, orbit: &mut Query<&mut Orbit>) {
    cam.locked = false;
    cam.target = None;
    if let (Some(saved), Some(mut o)) = (cam.saved.take(), orbit.iter_mut().next()) {
        (o.focus, o.yaw, o.pitch, o.dist) = (saved.focus, saved.yaw, saved.pitch, saved.dist);
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
    mut replies: MessageReader<ScriptReply>,
    time: Res<Time<Real>>,
) {
    // The host ended the cineractive (cap, fault, mission end): the camera is the player's again.
    if replies.read().any(|r| *r == ScriptReply::CineractiveReleased) && cam.locked {
        release(&mut cam, &mut orbit);
        cam.model.observe(&Presentation::Cineractive { on: false, skippable: false }, 0, &[]);
    }
    // The script raised these on the tick before the view that carries them.
    let tick = link.as_deref().map_or(0, |l| l.curr().tick.saturating_sub(1));
    for SimMsg(e) in events.read() {
        let SimEvent::Present(p) = e else { continue };
        let placed = cam.model.shot_placed.then(|| cam.model.shot.clone());
        cam.model.observe(p, tick, &paths.0);
        if cam.model.shot_placed && placed != Some(cam.model.shot.clone()) {
            cam.shots += 1;
        }
        let current = orbit.iter().next().map(of);
        match p {
            Presentation::Cineractive { on, .. } => {
                if *on && !cam.locked {
                    cam.locked = true;
                    cam.saved = current;
                    cam.target = current;
                } else if !*on && cam.locked {
                    release(&mut cam, &mut orbit);
                }
            }
            // A shot outside cineractive mode is one move of the camera, not a hold: the
            // scripts bracket shots in cineractive mode, and a hold nothing brackets would have
            // no end the player could count on.
            Presentation::Shot { from, at, .. } if !cam.locked => {
                let (Some(f), Some(a)) = (spot(from, link.as_deref(), &paths), spot(at, link.as_deref(), &paths)) else {
                    unhandled.note("Shot with a path spot or a vanished entity");
                    continue;
                };
                let now = time.elapsed_secs();
                if cam.loose_at.is_some_and(|at| now - at < LOOSE_SHOT_EVERY_SECS) {
                    unhandled.note("Shot outside a cineractive, more than once a second");
                    continue;
                }
                cam.loose_at = Some(now);
                let pose = pose_looking(f, a);
                if let Some(mut o) = orbit.iter_mut().next() {
                    (o.focus, o.yaw, o.pitch, o.dist) = (pose.focus, pose.yaw, pose.pitch, pose.dist);
                }
            }
            Presentation::Call { name, args } if name == "A2.CenterCamera" || name == "Camera.center" => {
                // An entity (A2 `CenterCamera`, MMM `Camera:center(entity)`) or a place (`Camera:center(Vector)`).
                let place = args.iter().find_map(|a| match a {
                    Arg::Entity(id) => spot(&Spot::Entity(*id), link.as_deref(), &paths),
                    // A place keeps the camera's current height: a script's `Vector(x, 0, z)` names
                    // the ground point, not a height (MMM's own handling not measured).
                    Arg::Position(p) => {
                        let mut at = crate::render::units::to_bevy(p.to_f32());
                        at.y = cam.saved.as_ref().filter(|_| cam.locked).or(current.as_ref()).map_or(at.y, |c| c.focus.y);
                        Some(at)
                    }
                    _ => None,
                });
                match (place, orbit.iter_mut().next()) {
                    // During a cineractive the strategy camera behind it moves (the original's
                    // RTS camera is not the one the shots drive): it comes back there.
                    (Some(at), Some(_)) if cam.locked => {
                        if let Some(saved) = cam.saved.as_mut() {
                            saved.focus = at;
                        }
                    }
                    (Some(at), Some(mut o)) => o.focus = at,
                    _ => {
                        unhandled.note(name);
                    }
                }
            }
            Presentation::CameraOffset { .. } if !cam.locked => {
                unhandled.note("CameraOffset outside a cineractive");
            }
            Presentation::Call { name, .. } if name.ends_with(".SetCameraZoom") || (name.ends_with(".CenterCamera") && name != "A2.CenterCamera") => {
                unhandled.note(name);
            }
            _ => {}
        }
    }
}

/// While a cineractive holds the camera, put it at the shot's pose, over what input just did.
fn hold(link: Option<Res<SimLink>>, clock: Option<Res<crate::clock::AnimClock>>, mut cam: ResMut<CinematicCamera>, mut q: Query<(&mut Orbit, &mut Transform)>) {
    if !cam.locked {
        return;
    }
    let alpha = clock.map_or(1.0, |c| c.alpha().clamp(0.0, 1.0));
    // The interpolated sim time, in ticks: units are drawn between the last two views.
    let now = link.as_deref().map_or(0.0, |l| l.curr().tick as f64 - 1.0 + f64::from(alpha));
    let locate = |id| {
        let (p, h) = link.as_deref()?.interpolate(id, alpha)?;
        Some(([f64::from(p[0]), f64::from(p[1]), f64::from(p[2])], [f64::from(h[0]), 0.0, f64::from(h[1])]))
    };
    if let Some((eye, look)) = cam.model.pose(now, &locate) {
        cam.target = Some(pose_looking(bevy(eye), bevy(look)));
    }
    let Some(pose) = cam.target else { return };
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
            .add_message::<ScriptReply>()
            // After the map's pan and orbit systems, so the shot, not the player's input
            // of this frame, is what is drawn.
            .add_systems(Update, (consume, hold.after(crate::viewer::orbit_camera)).chain().after(crate::app::GameSet::SimLink).before(crate::app::GameSet::Render));
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
        let (from, at) = (sv(100, 20, 30), sta_core::Vec3::ZERO);
        send(&mut app, Presentation::Shot { from: Spot::Position(from), at: Spot::Position(at), seconds: Fixed::ZERO });
        let during = orbit(&mut app);
        assert_ne!(during, before);
        let tf = *app.world_mut().query::<&Transform>().single(app.world()).unwrap();
        // x is kept, z mirrored (sim z 30 is Bevy -30).
        assert!(tf.translation.distance(Vec3::new(100.0, 20.0, -30.0)) < 1e-2, "{:?}", tf.translation);
        // The player's input cannot move it while locked.
        app.world_mut().query::<&mut Orbit>().single_mut(app.world_mut()).unwrap().dist = 5.0;
        app.update();
        assert_eq!(orbit(&mut app), during);
        send(&mut app, Presentation::Cineractive { on: false, skippable: false });
        assert_eq!(orbit(&mut app), before);
    }

    #[test]
    fn a_shot_outside_cineractive_mode_moves_the_camera_once_and_holds_nothing() {
        let mut app = app();
        let before = orbit(&mut app);
        send(&mut app, Presentation::Shot { from: Spot::Position(sv(100, 20, 30)), at: Spot::Position(sta_core::Vec3::ZERO), seconds: Fixed::ZERO });
        assert_ne!(orbit(&mut app), before);
        assert!(!app.world().resource::<CinematicCamera>().locked);
        // The player can move it away at once, and a second shot at once does not pin it back.
        app.world_mut().query::<&mut Orbit>().single_mut(app.world_mut()).unwrap().dist = 5.0;
        send(&mut app, Presentation::Shot { from: Spot::Position(sv(7, 7, 7)), at: Spot::Position(sta_core::Vec3::ZERO), seconds: Fixed::ZERO });
        assert_eq!(orbit(&mut app).dist, 5.0);
        assert_eq!(app.world().resource::<Unhandled>().count("Shot outside a cineractive, more than once a second"), 1);
    }

    #[test]
    fn releasing_the_cineractive_gives_the_camera_back() {
        let mut app = app();
        let before = orbit(&mut app);
        send(&mut app, Presentation::Cineractive { on: true, skippable: true });
        send(&mut app, Presentation::Shot { from: Spot::Position(sv(5, 5, 5)), at: Spot::Position(sta_core::Vec3::ZERO), seconds: Fixed::ZERO });
        assert_ne!(orbit(&mut app), before);
        app.world_mut().write_message(ScriptReply::CineractiveReleased);
        app.update();
        assert_eq!(orbit(&mut app), before);
        assert!(!app.world().resource::<CinematicCamera>().locked);
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
        // Game space: the Bevy camera lands at z mirrored.
        app.world_mut().resource_mut::<ScenePaths>().0.push(("cam".into(), vec![[0.0, 0.0, 50.0], [10.0, 10.0, 10.0]]));
        send(&mut app, Presentation::Cineractive { on: true, skippable: true });
        send(&mut app, Presentation::Shot { from: Spot::PathPoint { path: "cam".into(), index: 1 }, at: Spot::PathPoint { path: "cam".into(), index: 0 }, seconds: Fixed::ZERO });
        let tf = *app.world_mut().query::<&Transform>().single(app.world()).unwrap();
        assert!(tf.translation.distance(Vec3::new(10.0, 10.0, -10.0)) < 1e-2, "{:?}", tf.translation);
    }

    #[test]
    fn an_a1_path_shot_is_played_from_the_shot_model() {
        let mut app = app();
        app.world_mut().resource_mut::<ScenePaths>().0.push(("ZOOM".into(), vec![[0.0, 50.0, 0.0], [100.0, 50.0, 0.0], [200.0, 50.0, 0.0]]));
        app.world_mut().resource_mut::<ScenePaths>().0.push(("Camera_1".into(), vec![[100.0, 0.0, 500.0]]));
        send(&mut app, Presentation::Cineractive { on: true, skippable: true });
        // `CineractiveShot('ZOOM', 0.8, 'Camera_1', 0)`: no sim link here, so sim time stays at the
        // shot's own tick and the eye is at the path's first point.
        let shot = Presentation::Call { name: "A1.CineractiveShot".into(), args: vec![Arg::Text("ZOOM".into()), Arg::Num(Fixed::from_f64(0.8)), Arg::Text("Camera_1".into()), Arg::Int(0)] };
        send(&mut app, shot);
        let cam = app.world().resource::<CinematicCamera>();
        assert!(cam.locked);
        assert_eq!(cam.shots, 1);
        let tf = *app.world_mut().query::<&Transform>().single(app.world()).unwrap();
        assert!(tf.translation.distance(Vec3::new(0.0, 50.0, 0.0)) < 1e-2, "{:?}", tf.translation);
        // It looks at Camera_1's point (Bevy z mirrored).
        let ahead = tf.forward().as_vec3();
        let want = (Vec3::new(100.0, 0.0, -500.0) - tf.translation).normalize();
        assert!(ahead.dot(want) > 0.999, "{ahead:?} {want:?}");
    }

    #[test]
    fn a_skip_leaves_the_camera_to_the_script_and_a_release_gives_it_back() {
        let mut app = app();
        let before = orbit(&mut app);
        send(&mut app, Presentation::Cineractive { on: true, skippable: true });
        send(&mut app, Presentation::Shot { from: Spot::Position(sv(5, 5, 5)), at: Spot::Position(sta_core::Vec3::ZERO), seconds: Fixed::ZERO });
        app.world_mut().write_message(ScriptReply::CineractiveSkipped);
        app.update();
        assert!(app.world().resource::<CinematicCamera>().locked, "the script ends it after a skip");
        app.world_mut().write_message(ScriptReply::CineractiveReleased);
        app.update();
        assert_eq!(orbit(&mut app), before);
        assert!(!app.world().resource::<CinematicCamera>().locked);
        assert!(!app.world().resource::<CinematicCamera>().model.active);
    }
}
