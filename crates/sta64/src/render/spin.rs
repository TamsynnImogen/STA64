//! Visual spin of resource rocks and planets (#1047).
//!
//! Rule: an object of the resource-rock class (`classLabel = scrap`: moons, comets, rock scrap)
//! turns about a fixed tilted axis of its own at a rate from the definition's `rotationSpeed`
//! (no key: [`ROCK_DEFAULT_RATE`]; zero: still). A planet (`classLabel = planet`) turns about
//! its own vertical axis from its own `rotationSpeed` (no key: [`PLANET_DEFAULT_RATE`]).
//! The turn follows the match's game time, the sim's tick plus the interpolation alpha, so it stands while
//! the game is paused, starts from phase zero with each match, and every client (and a loaded save) shows
//! the same pose at the same tick.
//! Grade: read from the executable, static; that a moon tumbles with its shell and that its glow
//! halo does not turn: measured on one install; the numeric rate: not measured.
//!
//! STA64 derives each object's factors deterministically from a stable object key within ranges
//! ([`rock_turn`], [`planet_turn`]). The ranges are read from the executable, static; the per-object
//! draw, the axis direction and the sense are STA64's (interim).
//!
//! The turn is a visual transform of a pivot entity below the object (or, for a planet, of the
//! planet's own entity, which no sim heading drives). No sim state, no digest and no file format
//! is involved: the sim does not know the rotation; picking and the pick radius are spherical
//! and use the object's position; `render::units::place` writes the heading to the object's own
//! entity, never to the pivot. A glow halo is a [`Billboard`](crate::render::sprites::Billboard)
//! that `face_camera` re-aims against its parents' rotation, so it keeps facing the camera.

use crate::clock::AnimClock;
use crate::sim_link::SimLink;
use bevy::prelude::*;
use sta_sim::TICK_HZ;

/// Rate in rad/s of a resource rock whose definition has no `rotationSpeed` (read from the
/// executable, static).
pub const ROCK_DEFAULT_RATE: f32 = 0.2;
/// Rate in rad/s of a planet whose definition has no `rotationSpeed` (read from the executable,
/// static).
pub const PLANET_DEFAULT_RATE: f32 = 0.07;
/// Share of planets that turn the other way (read from the executable, static).
const PLANET_REVERSED: f32 = 0.1;

/// An object that turns or spins stands still while this is on it: a still copy of a thing out of sight does not
/// animate (rule: read from the executable, static; #1146). A [`Turn`] is a function of game time, so it resumes at
/// the angle game time gives.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct SpinPaused;

/// A turn about `axis` (in the parent's frame) at `rate` rad/s of game time, applied on top of
/// `base`.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct Turn {
    pub base: Quat,
    pub axis: Vec3,
    pub rate: f32,
}

impl Turn {
    /// The rotation after `seconds` of game time.
    pub fn at(&self, seconds: f64) -> Quat {
        let angle = (self.rate as f64 * seconds) % std::f64::consts::TAU;
        self.base * Quat::from_axis_angle(self.axis, angle as f32)
    }
}

/// A stable key for a map object: its id and place, never a run-dependent value.
pub fn object_key(id: i32, at: [f32; 3]) -> u64 {
    let mut k = crate::clock::splitmix64(id as u32 as u64);
    for v in at {
        k = crate::clock::splitmix64(k ^ v.to_bits() as u64);
    }
    k
}

/// A draw in [-1, 1] for `key` and `salt`.
fn draw(key: u64, salt: u64) -> f32 {
    let bits = crate::clock::splitmix64(key ^ salt.wrapping_mul(0xd6e8_feb8_6659_fd93)) >> 40;
    bits as f32 / (1u64 << 23) as f32 - 1.0
}

/// The axis and rate (rad/s) of a rock with `rotation_speed` (the key's value, `None` when the
/// definition has none), or `None` when it stands still. The angular velocity's components are
/// 0.5 r1, 0.7 r2 + 0.1 and 0.5 r3 + 0.2 times the speed, r in [-1, 1] (ranges: read from the
/// executable, static), in the models' frame; the world is mirrored in Z.
pub fn rock_turn(key: u64, rotation_speed: Option<f32>) -> Option<(Vec3, f32)> {
    let rs = rotation_speed.unwrap_or(ROCK_DEFAULT_RATE);
    if rs == 0.0 {
        return None;
    }
    let w = Vec3::new(0.5 * draw(key, 1), 0.7 * draw(key, 2) + 0.1, -(0.5 * draw(key, 3) + 0.2)) * rs;
    let rate = w.length();
    (rate > 0.0).then(|| (w / rate, rate))
}

/// The signed rate (rad/s) about a planet's vertical axis, or `None` when it stands still:
/// (1.0 + 0.4 r) times the speed, reversed for about one planet in ten (read from the
/// executable, static; which planets reverse is STA64's draw).
pub fn planet_turn(key: u64, rotation_speed: Option<f32>) -> Option<f32> {
    let rs = rotation_speed.unwrap_or(PLANET_DEFAULT_RATE);
    if rs == 0.0 {
        return None;
    }
    let w = (1.0 + 0.4 * draw(key, 4)) * rs;
    Some(if draw(key, 5) * 0.5 + 0.5 < PLANET_REVERSED { -w } else { w })
}

/// The entity a rock's model is spawned under: a pivot child of `parent` carrying a [`Turn`] when
/// `class` is the resource-rock class and the speed is not zero, else `parent` itself. `key` is
/// the object's stable key; `rotation_speed` the definition's `rotationSpeed` (none: the default).
pub fn rock_pivot(commands: &mut Commands, parent: Entity, class: &str, rotation_speed: Option<f32>, key: u64) -> Entity {
    if !class.eq_ignore_ascii_case("scrap") {
        return parent;
    }
    let Some((axis, rate)) = rock_turn(key, rotation_speed) else { return parent };
    commands.spawn((Transform::IDENTITY, Visibility::default(), Turn { base: Quat::IDENTITY, axis, rate }, ChildOf(parent))).id()
}

/// Game seconds of a match at sim tick `tick` and interpolation `alpha` of the next.
pub fn game_seconds(tick: u64, alpha: f32) -> f64 {
    (tick as f64 + alpha.clamp(0.0, 1.0) as f64) / TICK_HZ as f64
}

/// Turn every [`Turn`] to the match's game time. Without a sim (a viewer) the animation clock's own tick stands in.
pub fn turn(clock: Res<AnimClock>, link: Option<Res<SimLink>>, mut q: Query<(&Turn, &mut Transform), Without<SpinPaused>>) {
    let now = match link {
        Some(l) => game_seconds(l.curr().tick, clock.alpha()),
        None => clock.now(),
    };
    for (t, mut tf) in &mut q {
        let want = t.at(now);
        if tf.rotation != want {
            tf.rotation = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ClockMode;
    use crate::render::sprites::Billboard;
    use sta_sim::TICK_HZ;

    /// A world that stands at one view.
    struct Held(std::sync::Arc<sta_sim::WorldView>);

    impl sta_sim::WorldSource for Held {
        fn view(&self) -> std::sync::Arc<sta_sim::WorldView> {
            self.0.clone()
        }
        fn step(&mut self, _: &[sta_sim::command::Command]) -> sta_sim::TickOutput {
            sta_sim::TickOutput { view: self.0.clone(), events: Vec::new() }
        }
    }

    fn view_at(tick: u64) -> std::sync::Arc<sta_sim::WorldView> {
        std::sync::Arc::new(sta_sim::WorldView { tick, ..Default::default() })
    }

    /// An app with the turn system, a match at tick 0 and a clock that moves only when the test says.
    fn rig() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .insert_resource(AnimClock::new(ClockMode::Bench { step: 1.0 / TICK_HZ as f64 }, 1))
            .insert_resource(SimLink::new(Box::new(Held(view_at(0))), Box::new(sta_sim::command::LocalRouter::new()), sta_sim::PlayerId::BENCH))
            .add_systems(Update, (turn, crate::render::sprites::face_camera).chain());
        app
    }

    /// `ticks` frames of the match, one sim tick each unless the game is paused (a paused sim runs none).
    fn run(app: &mut App, ticks: u32) {
        for _ in 0..ticks {
            let paused = {
                let mut clock = app.world_mut().resource_mut::<AnimClock>();
                let n = clock.begin_frame(0.0);
                for _ in 0..n {
                    clock.ticked();
                }
                clock.end_frame();
                clock.paused()
            };
            if !paused {
                let mut link = app.world_mut().resource_mut::<SimLink>();
                let next = link.curr().tick + 1;
                link.feed(view_at(next));
            }
            app.update();
        }
    }

    fn rotation(app: &App, e: Entity) -> Quat {
        app.world().get::<Transform>(e).unwrap().rotation
    }

    #[test]
    fn a_rock_turns_by_rate_times_game_time_about_its_axis() {
        let mut app = rig();
        let (axis, rate) = rock_turn(7, Some(0.3)).unwrap();
        let e = app.world_mut().spawn((Transform::IDENTITY, Turn { base: Quat::IDENTITY, axis, rate })).id();
        run(&mut app, 60);
        let t = app.world().resource::<AnimClock>().now();
        assert!((t - 2.0).abs() < 1e-6, "two seconds of game time: {t}");
        let (got_axis, got_angle) = rotation(&app, e).to_axis_angle();
        let want = rate * 2.0;
        assert!((got_angle - want).abs() < 1e-4, "angle {got_angle} vs {want}");
        assert!(got_axis.dot(axis) > 0.9999, "axis {got_axis} vs {axis}");
    }

    #[test]
    fn the_rate_is_the_speed_scaled_by_the_draw_and_the_default_stands_in_for_no_key() {
        for key in 0..200u64 {
            // Components at speed 1: |wx| <= 0.5, wy in [-0.6, 0.8], -wz in [-0.3, 0.7].
            let (a, r) = rock_turn(key, Some(1.0)).unwrap();
            let w = a * r;
            assert!(w.x.abs() <= 0.5 + 1e-5 && (-0.6 - 1e-5..=0.8 + 1e-5).contains(&w.y) && (-0.3 - 1e-5..=0.7 + 1e-5).contains(&-w.z));
            // The same key at another speed is the same draw scaled.
            let (a2, r2) = rock_turn(key, Some(0.3)).unwrap();
            assert!((r2 - 0.3 * r).abs() < 1e-5 && a2.dot(a) > 0.9999);
            // No key: the default speed.
            let (b, rb) = rock_turn(key, None).unwrap();
            let (c, rc) = rock_turn(key, Some(ROCK_DEFAULT_RATE)).unwrap();
            assert!((rb - rc).abs() < 1e-7 && b.dot(c) > 0.9999);
        }
    }

    #[test]
    fn zero_speed_stands_still() {
        assert_eq!(rock_turn(3, Some(0.0)), None);
        assert_eq!(planet_turn(3, Some(0.0)), None);
    }

    #[test]
    fn the_draw_is_stable_and_differs_between_objects() {
        assert_eq!(rock_turn(11, Some(0.3)), rock_turn(11, Some(0.3)));
        assert_eq!(object_key(5, [1.0, 2.0, 3.0]), object_key(5, [1.0, 2.0, 3.0]));
        assert_ne!(object_key(5, [1.0, 2.0, 3.0]), object_key(5, [1.0, 2.0, 4.0]));
        let a = rock_turn(object_key(1, [0.0; 3]), Some(0.3)).unwrap().0;
        let b = rock_turn(object_key(2, [0.0; 3]), Some(0.3)).unwrap().0;
        assert!(a.dot(b) < 0.9999);
    }

    #[test]
    fn a_planet_turns_about_the_vertical_axis_at_the_declared_speed_within_its_range() {
        let (mut reversed, n) = (0, 2000);
        for key in 0..n {
            let w = planet_turn(key, Some(0.5)).unwrap();
            assert!((0.5 * 0.6 - 1e-5..=0.5 * 1.4 + 1e-5).contains(&w.abs()), "{w}");
            reversed += (w < 0.0) as u32;
        }
        let share = reversed as f32 / n as f32;
        assert!((0.05..0.15).contains(&share), "about one in ten reverses: {share}");
        let w = planet_turn(9, None).unwrap().abs();
        assert!((PLANET_DEFAULT_RATE * 0.6..=PLANET_DEFAULT_RATE * 1.4 + 1e-6).contains(&w));

        let mut app = rig();
        let base = Quat::from_rotation_x(0.3);
        let e = app.world_mut().spawn((Transform::IDENTITY.with_rotation(base), Turn { base, axis: Vec3::Y, rate: 0.5 })).id();
        run(&mut app, 30);
        let want = base * Quat::from_rotation_y(0.5);
        assert!(rotation(&app, e).angle_between(want) < 1e-4);
    }

    #[test]
    fn a_paused_game_stands_still() {
        let mut app = rig();
        let e = app.world_mut().spawn((Transform::IDENTITY, Turn { base: Quat::IDENTITY, axis: Vec3::Y, rate: 1.0 })).id();
        run(&mut app, 15);
        let before = rotation(&app, e);
        assert!(before.angle_between(Quat::IDENTITY) > 0.1);
        app.world_mut().resource_mut::<AnimClock>().set_paused(true);
        run(&mut app, 45);
        assert_eq!(rotation(&app, e), before);
        app.world_mut().resource_mut::<AnimClock>().set_paused(false);
        run(&mut app, 15);
        assert!(rotation(&app, e).angle_between(before) > 0.1);
    }

    #[test]
    fn a_fresh_match_starts_at_phase_zero_whatever_the_app_has_played() {
        let mut app = rig();
        // An app that has already played matches: its animation clock's own tick is far ahead.
        for _ in 0..500 {
            app.world_mut().resource_mut::<AnimClock>().ticked();
        }
        let base = Quat::from_rotation_x(0.3);
        let e = app.world_mut().spawn((Transform::IDENTITY, Turn { base, axis: Vec3::Y, rate: 1.0 })).id();
        app.update();
        assert_eq!(rotation(&app, e), base);
    }

    #[test]
    fn the_pose_is_a_function_of_the_sim_tick() {
        let turn_at = |extra_clock_ticks: u32| {
            let mut app = rig();
            for _ in 0..extra_clock_ticks {
                app.world_mut().resource_mut::<AnimClock>().ticked();
            }
            let e = app.world_mut().spawn((Transform::IDENTITY, Turn { base: Quat::IDENTITY, axis: Vec3::Y, rate: 0.7 })).id();
            // A loaded save or a second player's client reaches tick 45 at once.
            app.world_mut().resource_mut::<SimLink>().feed(view_at(45));
            app.update();
            rotation(&app, e)
        };
        let want = Quat::from_rotation_y(0.7 * 1.5);
        assert!(turn_at(0).angle_between(want) < 1e-4);
        assert!(turn_at(900).angle_between(want) < 1e-4);
        assert_eq!(game_seconds(45, 0.0), 1.5);
        assert_eq!(game_seconds(45, 0.5), 45.5 / TICK_HZ as f64);
    }

    #[test]
    fn the_glow_halo_keeps_facing_the_camera_while_the_rock_turns() {
        let mut app = rig();
        let cam_rot = Quat::from_rotation_y(0.4);
        app.world_mut().spawn((Camera3d::default(), Transform::IDENTITY.with_rotation(cam_rot)));
        let axis = Vec3::new(1.0, 1.0, 0.0).normalize();
        let pivot = app.world_mut().spawn((Transform::IDENTITY, Turn { base: Quat::IDENTITY, axis, rate: 1.0 })).id();
        let halo = app.world_mut().spawn((Transform::IDENTITY, Billboard, ChildOf(pivot))).id();
        let world = |app: &App, e: Entity| app.world().get::<GlobalTransform>(e).unwrap().compute_transform().rotation;
        run(&mut app, 3);
        let first = world(&app, pivot);
        run(&mut app, 30);
        // The rock has turned by a full radian; the halo's world rotation is the camera's, to within
        // the one frame (1/30 rad here) the billboard trails the parent's propagated turn by.
        assert!(world(&app, pivot).angle_between(first) > 0.9);
        assert!(world(&app, halo).angle_between(cam_rot) < 0.05, "{}", world(&app, halo).angle_between(cam_rot));
    }
}
