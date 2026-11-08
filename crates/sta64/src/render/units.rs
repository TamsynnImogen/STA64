//! Ships in the map view: placement from the sim's views, and the selection
//! overlay.
//!
//! - The sim link steps the sim (`GameSet::SimLink`); each frame this places
//!   every [`SimEntity`] between the last two views at the animation clock's
//!   alpha (`GameSet::Interpolate`).
//! - Selection rings and the line to a moving ship's target are gizmos
//!   (`GameSet::Render`). RND-08 replaces them with the real overlays.
//!
//! The sim works in the map's left-handed space; the renderer mirrors Z.

use bevy::prelude::*;

use crate::app::GameSet;
use crate::clock::AnimClock;
use crate::input::orders::OrderTargets;
use crate::sim_link::{Selection, SimEntity, SimLink};

/// Selection radius in world units, on the entity drawn for a sim unit.
#[derive(Component)]
pub struct SelectRadius(pub f32);

pub struct UnitsRenderPlugin;

impl Plugin for UnitsRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, place.in_set(GameSet::Interpolate).run_if(resource_exists::<SimLink>))
            .add_systems(Update, rings.in_set(GameSet::Render).run_if(resource_exists::<SimLink>));
    }
}

pub fn to_bevy(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[0], p[1], -p[2])
}

/// Place each unit between its last two ticks.
fn place(link: Res<SimLink>, clock: Res<AnimClock>, mut q: Query<(&SimEntity, &mut Transform)>) {
    let alpha = clock.alpha().clamp(0.0, 1.0);
    for (u, mut tf) in &mut q {
        let Some((pos, h)) = link.interpolate(u.0, alpha) else { continue };
        tf.translation = to_bevy(pos);
        // The model's front (+Z in the files) is Bevy's -Z after the mirror, which
        // is what `looking_to` points along the heading.
        tf.rotation = Transform::IDENTITY.looking_to(Vec3::new(h[0], 0.0, -h[1]), Vec3::Y).rotation;
    }
}

fn rings(mut gizmos: Gizmos, selection: Res<Selection>, targets: Res<OrderTargets>, units: Query<(&SimEntity, &SelectRadius, &GlobalTransform)>) {
    let flat = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let colour = Color::srgb(0.3, 1.0, 0.4);
    for (u, radius, gt) in &units {
        if !selection.0.contains(&u.0) {
            continue;
        }
        gizmos.circle(Isometry3d::new(gt.translation(), flat), radius.0, colour);
        if let Some(t) = targets.0.get(&u.0) {
            gizmos.line(gt.translation(), to_bevy(t.to_f32()), Color::srgba(0.3, 1.0, 0.4, 0.4));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::configure_sets;
    use crate::sim_link::{CommandQueue, SimLinkPlugin};
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;
    use sta_sim::command::{CommandKind, LocalRouter, Order};
    use sta_sim::{EntityId, Fixed, MoveStats, PlayerId, Sim, Vec3 as SimVec3};

    /// A ship at the origin facing +Z, ordered to `to` through the command
    /// queue, drawn by `place` from the sim link on the animation clock.
    fn rig(to: [f32; 3]) -> (App, EntityId, Entity) {
        let mut sim = Sim::new(1);
        let stats = MoveStats::from_f32(140.0, 100.0, 1.0, 1.0, 1.0);
        let id = sim.spawn(1, SimVec3::ZERO, (Fixed::ZERO, Fixed::ONE), stats);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0)))
            .add_plugins(SimLinkPlugin)
            .insert_resource(SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId::BENCH))
            .add_systems(Update, place.in_set(GameSet::Interpolate));
        configure_sets(&mut app);
        app.world_mut().resource_mut::<CommandQueue>().0.push(CommandKind::Order(Order::Move {
            units: vec![id],
            to: SimVec3::from_f32(to),
            queue: false,
        }));
        let e = app.world_mut().spawn((SimEntity(id), Transform::IDENTITY)).id();
        (app, id, e)
    }

    /// A ship cruising in a straight line must move the same distance every
    /// rendered frame (sim at 30 Hz, frames at 60 Hz, interpolated between ticks).
    #[test]
    fn straight_line_motion_is_even_per_frame() {
        let (mut app, _, e) = rig([0.0, 0.0, 100_000.0]);
        let mut zs = Vec::new();
        for _ in 0..180 {
            app.update();
            zs.push(app.world().get::<Transform>(e).unwrap().translation.z);
        }
        // Skip the first second (acceleration), then compare frame steps.
        let steps: Vec<f32> = zs.windows(2).skip(60).map(|w| (w[1] - w[0]).abs()).collect();
        let (lo, hi) = steps.iter().fold((f32::MAX, 0f32), |(l, h), &s| (l.min(s), h.max(s)));
        let expect = 140.0 / 60.0;
        assert!(lo > expect * 0.8 && hi < expect * 1.2, "uneven frame steps: min {lo}, max {hi}, expected ~{expect}");
    }

    /// With uneven frame times the on-screen speed (step / frame time) must stay
    /// steady too.
    #[test]
    fn motion_follows_uneven_frames() {
        let (mut app, _, e) = rig([0.0, 0.0, 100_000.0]);
        let frames = [0.007, 0.023, 0.016, 0.031, 0.011, 0.019, 0.042, 0.009];
        let mut speeds = Vec::new();
        let mut last = 0.0f32;
        for i in 0..400 {
            let dt = frames[i % frames.len()];
            app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(dt)));
            app.update();
            let z = app.world().get::<Transform>(e).unwrap().translation.z;
            if i > 120 {
                speeds.push((z - last).abs() / dt as f32);
            }
            last = z;
        }
        let (lo, hi) = speeds.iter().fold((f32::MAX, 0f32), |(l, h), &s| (l.min(s), h.max(s)));
        assert!(lo > 140.0 * 0.85 && hi < 140.0 * 1.15, "on-screen speed varies: {lo}..{hi}");
    }

    /// A move order through the command queue is stepped by the sim link and
    /// drawn between the last two views, ending on the target (mirrored in Z).
    #[test]
    fn sim_link_drives_a_unit_to_its_target_with_interpolation() {
        let target = [30.0, 0.0, 200.0];
        let (mut app, id, e) = rig(target);
        let mut between = 0;
        let mut arrived_at = None;
        for frame in 0..600 {
            app.update();
            let link = app.world().resource::<SimLink>();
            let (prev, curr) = (link.prev().unit(id).unwrap().pos.to_f32(), link.curr().unit(id).unwrap().pos.to_f32());
            let alpha = app.world().resource::<AnimClock>().alpha();
            let at = app.world().get::<Transform>(e).unwrap().translation;
            // On the segment between the views, at alpha along it.
            let want = to_bevy(prev).lerp(to_bevy(curr), alpha);
            assert!(at.distance(want) < 1e-3, "frame {frame}: drawn {at}, expected {want}");
            if prev != curr && alpha > 0.1 && alpha < 0.9 {
                between += 1;
                assert!(at.distance(to_bevy(prev)) > 1e-4 && at.distance(to_bevy(curr)) > 1e-4, "frame {frame}: drawn on a view, not between");
            }
            if curr == target && arrived_at.is_none() {
                arrived_at = Some(frame);
            }
            if arrived_at.is_some_and(|f| frame > f + 6) {
                break;
            }
        }
        assert!(arrived_at.is_some(), "never reached the target");
        assert!(between > 10, "only {between} frames drawn between two views");
        assert_eq!(app.world().get::<Transform>(e).unwrap().translation, to_bevy(target));
    }
}
