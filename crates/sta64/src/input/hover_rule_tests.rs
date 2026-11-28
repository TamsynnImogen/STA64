//! The hover pick follows the original's rule where the content follows it, and the pick by ray elsewhere
//! (#1164). The claim holder's own tests in `hover.rs` build no content: they stay on the pick by ray and are
//! unchanged.

use super::*;
use crate::render::units::SelectRadius;
use crate::sim_link::{Selection, SimEntity};
use sta_content::Profile;
use sta_sim::command::{Command, LocalRouter};
use sta_sim::source::WorldSource;
use sta_sim::view::{UnitFlags, UnitView, WorldView};
use sta_sim::{ContentKey, Fixed, PlayerId, TickOutput, Vec3 as SimVec3};
use std::sync::Arc;

struct Frozen(Arc<WorldView>);

impl WorldSource for Frozen {
    fn step(&mut self, _: &[Command]) -> TickOutput {
        TickOutput { view: self.0.clone(), events: Vec::new() }
    }

    fn view(&self) -> Arc<WorldView> {
        self.0.clone()
    }
}

fn unit(speed: f32) -> UnitView {
    UnitView {
        id: EntityId(1),
        def: ContentKey::NONE,
        owner: PlayerId(0),
        pos: SimVec3::ZERO,
        heading: (Fixed::ZERO, Fixed::ONE),
        speed: Fixed::from_f64(f64::from(speed)),
        hull: [Fixed::ONE; 2],
        shields: [[Fixed::ZERO; 2]; 4],
        crew: None,
        energy: None,
        autonomy: None,
        flags: UnitFlags::SELECTABLE,
        progress: None,
    }
}

fn content(profile: Option<Profile>) -> Option<Arc<sta_content::ContentDb>> {
    profile.map(|p| Arc::new(sta_content::ContentDb::builder().profile(p).build().unwrap()))
}

/// One selectable unit at `(0, 0, -100)` seen by player 0, on content of `profile`.
fn app(profile: Option<Profile>, speed: f32) -> App {
    let view = Arc::new(WorldView { units: vec![unit(speed)], visibility: vec![(PlayerId(0), vec![EntityId(1)])], ..Default::default() });
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<HoveredUnit>()
        .init_resource::<Aim>()
        .init_resource::<HudPointer>()
        .insert_resource(Selection(vec![]))
        .insert_resource(SimLink::new(Box::new(Frozen(view.clone())), Box::new(LocalRouter::new()), PlayerId(0)))
        .insert_resource(HudFeed { view, player: PlayerId(0), selected: vec![], content: content(profile), ..Default::default() })
        .add_systems(Update, track);
    app.world_mut().spawn((SimEntity(EntityId(1)), SelectRadius(1.0), GlobalTransform::from(Transform::from_xyz(0.0, 0.0, -100.0))));
    app
}

/// A right-handed perspective (depth 0..1) for an 800 x 600 view, written out so the tests depend on no library
/// constructor.
fn projection() -> Mat4 {
    let (near, far) = (0.1f32, 1000.0f32);
    let f = 1.0 / 0.5f32.tan();
    Mat4::from_cols(
        Vec4::new(f / (800.0 / 600.0), 0.0, 0.0, 0.0),
        Vec4::new(0.0, f, 0.0, 0.0),
        Vec4::new(0.0, 0.0, far / (near - far), -1.0),
        Vec4::new(0.0, 0.0, near * far / (near - far), 0.0),
    )
}

const CENTRE: Vec2 = Vec2::new(400.0, 300.0);

/// The pointer at `cursor` (window pixels) with the ray through the unit, so the pick by ray would take it.
fn point_at(app: &mut App, cursor: Vec2) {
    app.insert_resource(Aim(Some(AimState {
        cursor,
        ray: Ray3d::new(Vec3::ZERO, Dir3::NEG_Z),
        clip_from_world: projection(),
        size: Vec2::new(800.0, 600.0),
    })));
}

fn hovered(app: &mut App) -> Option<EntityId> {
    app.update();
    app.world().resource::<HoveredUnit>().0
}

/// The profiles that follow the rule: the stock Armada II profile as read, the Fleet Operations family on the
/// same engine line (INTERIM, not read for them). Armada I, STA64's own content and no content keep the pick by
/// ray; the same pointer, far from the unit on screen but with the ray through it, tells them apart.
#[test]
fn only_the_armada_ii_line_follows_the_rule_and_the_other_profiles_keep_the_ray_pick() {
    let far = Vec2::new(700.0, 60.0);
    for (profile, follows) in [
        (Some(Profile::Sta2Classic), true),
        (Some(Profile::Fo3), true),
        (Some(Profile::Fo4), true),
        (Some(Profile::Sta1Classic), false),
        (Some(Profile::Native), false),
        (None, false),
    ] {
        let mut app = app(profile, 0.0);
        point_at(&mut app, far);
        assert_eq!(hovered(&mut app), (!follows).then_some(EntityId(1)), "{profile:?}: pointer far from the unit, ray through it");
        // Near the unit on screen: hovered under the rule; the ray pick also takes it.
        point_at(&mut app, CENTRE);
        assert_eq!(hovered(&mut app), Some(EntityId(1)), "{profile:?}");
    }
}

/// The pointer at `cursor` for a window of `size` (logical pixels), with the ray through the unit.
fn point_in(app: &mut App, cursor: Vec2, size: Vec2) {
    app.insert_resource(Aim(Some(AimState { cursor, ray: Ray3d::new(Vec3::ZERO, Dir3::NEG_Z), clip_from_world: projection(), size })));
}

/// The fixed radius is in canvas units and follows the window through the real derivation, at two window sizes
/// and in either HUD mode: a pointer within 0.8 radii of a small unit's centre hovers it, one 1.5 radii away does
/// not, on a small window and on a large one. (The window's size is what `track` reads; no resource carries the
/// scale, so a switch of HUD mode cannot leave a stale one.)
#[test]
fn the_fixed_radius_follows_the_window_through_the_real_derivation() {
    for size in [Vec2::new(800.0, 600.0), Vec2::new(1600.0, 1200.0)] {
        let radius = super::super::hover_pick::MIN_RADIUS_PX * super::super::hover_pick::canvas_unit(size);
        for (factor, expected) in [(0.8, true), (1.5, false)] {
            let mut app = app(Some(Profile::Sta2Classic), 0.0);
            // The unit's centre on this window: the projection fills the window, so it is at the middle.
            point_in(&mut app, size / 2.0 + Vec2::new(factor * radius, 0.0), size);
            assert_eq!(hovered(&mut app).is_some(), expected, "window {size:?}, {factor} radii away");
        }
    }
}

/// A fast mover stays hovered when the pointer leaves it over empty space, for a moment; a still ship does not.
#[test]
fn a_fast_mover_stays_hovered_over_empty_space_and_a_still_ship_does_not() {
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;
    for (speed, stays) in [(200.0, true), (0.0, false)] {
        let mut app = app(Some(Profile::Sta2Classic), speed);
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(100)));
        point_at(&mut app, CENTRE);
        assert_eq!(hovered(&mut app), Some(EntityId(1)));
        point_at(&mut app, Vec2::new(700.0, 60.0));
        app.update();
        assert_eq!(hovered(&mut app).is_some(), stays, "speed {speed}");
        if stays {
            for _ in 0..20 {
                app.update();
            }
            assert_eq!(app.world().resource::<HoveredUnit>().0, None, "held for a limited time only");
        }
    }
}

/// The speed-scaled tolerance reaches a fast mover from farther away than a still one.
#[test]
fn a_fast_mover_is_picked_from_farther_away() {
    let probe = |speed: f32, dx: f32| {
        let mut app = app(Some(Profile::Sta2Classic), speed);
        point_at(&mut app, CENTRE + Vec2::new(dx, 0.0));
        hovered(&mut app)
    };
    let edge = (0..300).map(|px| px as f32).find(|px| probe(0.0, *px).is_none()).unwrap();
    assert_eq!(probe(0.0, edge), None);
    assert_eq!(probe(500.0, edge), Some(EntityId(1)));
}
