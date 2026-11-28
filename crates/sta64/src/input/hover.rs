//! The live unit under the map cursor, independent of the current selection.

use bevy::prelude::*;
use sta_sim::{EntityId, PlayerId};

use super::hover_pick::{Candidate, Sticky, pick};
use super::orders::Aim;
use super::select::{AimState, target_seen};
use crate::frontend::scene::MenuScene;
use crate::render::overlays::{ModelBounds, project_bounds};
use crate::render::strategic::Strategic;
use crate::render::units::SelectRadius;
use crate::sim_link::{SimEntity, SimLink};
use crate::ui::hud::{HudFeed, HudPointer};

/// The current live map hover, when one is valid.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoveredUnit(pub Option<EntityId>);

/// The Armada II hover readout: how long the pointer has rested on one team's units, for the
/// health bars (issue 1046). The bars appear above [`HoverReadout::BARS_ABOVE`] and brighten with
/// the fade. Only the Armada II look reads it; every other look ignores it.
///
/// Rules (grades: "static" is read from the Armada II executable):
/// - `fade` grows by `dt / FADE_SECONDS` each display tick and is capped at 1 [static].
/// - It resets to 0 when the hovered unit's team changes [static]. "Nothing hovered" and a neutral
///   (unowned) unit are one team, which shows no bars [static; measured on an unowned planet]: the
///   fade runs between them and shows nothing. Moving straight to another unit of the same team
///   keeps it [static].
/// - The clock is real time, not sim time: it keeps running while the game is paused [measured at
///   normal speed]. Its behaviour at other game speeds was not measured: interim, real time is used.
/// - Within the frame of a team change the fade is 0 and does not also grow; the order inside one
///   tick is not established [interim, one frame].
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct HoverReadout {
    /// The hovered unit.
    pub unit: Option<EntityId>,
    /// Its team (owner); `None` while nothing is hovered or the unit is neutral (unowned).
    pub team: Option<PlayerId>,
    /// 0 to 1.
    pub fade: f32,
}

impl HoverReadout {
    /// Seconds for the fade to run from 0 to 1 [static].
    pub const FADE_SECONDS: f32 = 2.0;
    /// Bars are drawn only above this fade, so from 1.0 s on [static, measured].
    pub const BARS_ABOVE: f32 = 0.5;

    /// One display tick of `dt` seconds with the unit under the pointer and its team (`None` for
    /// nothing hovered or a neutral unit).
    pub fn tick(&mut self, dt: f32, unit: Option<EntityId>, team: Option<PlayerId>) {
        self.unit = unit;
        if team != self.team {
            self.team = team;
            self.fade = 0.0;
        } else {
            self.fade = (self.fade + dt.max(0.0) / Self::FADE_SECONDS).min(1.0);
        }
    }

    /// Whether the bars are up at all.
    pub fn bars_up(&self) -> bool {
        self.unit.is_some() && self.team.is_some() && self.fade > Self::BARS_ABOVE
    }
}

/// Run the readout on real time, so it goes on while the game is paused.
pub fn tick_readout(time: Option<Res<Time<Real>>>, hovered: Option<Res<HoveredUnit>>, link: Option<Res<SimLink>>, mut readout: ResMut<HoverReadout>) {
    let dt = time.map_or(0.0, |t| t.delta_secs());
    let unit = hovered.and_then(|h| h.0).and_then(|id| link.as_deref()?.curr().unit(id).map(|u| (id, u.owner)));
    // The original's team 0 is STA64's neutral owner: the same team as "nothing hovered".
    let team = unit.map(|(_, owner)| owner).filter(|o| *o != sta_sim::world::NEUTRAL);
    readout.tick(dt, unit.map(|(id, _)| id), team);
}

/// What the hover pick reads of an object: its sim id, size, place and (when the model is replaced by a
/// symbol) its icon.
pub type HoverUnits<'w, 's> = Query<
    'w,
    's,
    (
        &'static SimEntity,
        &'static SelectRadius,
        &'static GlobalTransform,
        Option<&'static Strategic>,
        Option<&'static ModelBounds>,
    ),
>;

/// Track the current map hover without changing selection or issuing orders. The pick is the
/// original's rule ([`super::hover_pick`]): nearest projected centre, accepted within a radius or
/// inside the speed-scaled box, held for a moment on a fast mover over empty space.
#[allow(clippy::too_many_arguments)]
pub fn track(
    aim: Option<Res<Aim>>,
    pointer: Option<Res<HudPointer>>,
    link: Option<Res<SimLink>>,
    menu_scene: Option<Res<MenuScene>>,
    feed: Option<Res<HudFeed>>,
    time: Option<Res<Time<Real>>>,
    units: HoverUnits,
    mut sticky: ResMut<Sticky>,
    mut hovered: ResMut<HoveredUnit>,
) {
    let aim = aim.and_then(|a| a.0);
    let tracking = can_track(aim, pointer.is_some_and(|p| p.0), menu_scene.is_some(), link.as_deref(), feed.as_deref());
    let next = match (tracking, aim, link.as_deref(), feed.as_deref()) {
        (true, Some(aim), Some(link), Some(feed)) => {
            let picked = pick(&candidates(&aim, link, feed, &units), aim.cursor);
            let dt = time.map_or(0.0, |t| t.delta_secs());
            sticky.resolve(dt, picked, |id| eligible(link, feed, id).then(|| link.curr().unit(id).map_or(0.0, |u| u.speed.to_f32())))
        }
        _ => {
            *sticky = Sticky::default();
            None
        }
    };
    if hovered.0 != next {
        hovered.0 = next;
    }
}

/// Whether `id` may be hovered at all: selectable, not dying, in the view and visible to the viewer
/// (the original's candidate filter: drawn, visible, not being removed, not unpickable).
fn eligible(link: &SimLink, feed: &HudFeed, id: EntityId) -> bool {
    link.curr().unit(id).is_some_and(|live| {
        live.flags.contains(sta_sim::view::UnitFlags::SELECTABLE) && !live.flags.contains(sta_sim::view::UnitFlags::DYING)
    }) && feed.view.unit(id).is_some()
        && target_seen(&feed.view, Some(link.player()), id)
}

/// The eligible objects with their projected centre, bounding-sphere radius and container rectangle.
fn candidates(aim: &AimState, link: &SimLink, feed: &HudFeed, units: &HoverUnits) -> Vec<Candidate> {
    // The world direction of screen x, to turn a world radius into pixels at the object's depth.
    let right = aim.clip_from_world.row(0).truncate().normalize_or_zero();
    let mut out = Vec::new();
    for (entity, radius, gt, strategic, bounds) in units {
        let id = entity.0;
        if !eligible(link, feed, id) {
            continue;
        }
        let speed = link.curr().unit(id).map_or(0.0, |u| u.speed.to_f32());
        // A symbol replaces the model: its icon is what is under the pointer.
        if let Some(s) = strategic.filter(|s| s.symbol) {
            if s.shown > 0.5 {
                let half = Vec2::splat(s.size_px * 0.6);
                out.push(Candidate { id, centre: s.screen, radius: half.x, rect: Rect::from_center_half_size(s.screen, half), speed });
            }
            continue;
        }
        let centre_world = gt.transform_point(strategic.map_or(Vec3::ZERO, |s| s.centre));
        let Some(centre) = aim.project(centre_world) else { continue };
        let radius_px = aim.project(centre_world + right * radius.0).map_or(0.0, |p| p.distance(centre));
        let model = bounds.copied().unwrap_or_else(|| ModelBounds::fallback(radius.0));
        let rect = project_bounds(model, gt.to_matrix(), aim.clip_from_world, aim.size)
            .map_or_else(|| Rect::from_center_half_size(centre, Vec2::splat(radius_px)), |(r, _)| r);
        out.push(Candidate { id, centre, radius: radius_px, rect, speed });
    }
    out
}

fn can_track(
    aim: Option<AimState>,
    over_hud: bool,
    menu_open: bool,
    link: Option<&SimLink>,
    feed: Option<&HudFeed>,
) -> bool {
    aim.is_some() && !over_hud && !menu_open && link.is_some() && feed.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::strategic::Strategic;
    use crate::render::units::SelectRadius;
    use crate::sim_link::{Selection, SimEntity};
    use sta_sim::command::{Command, LocalRouter};
    use sta_sim::source::WorldSource;
    use sta_sim::view::{UnitFlags, UnitView, WorldView};
    use sta_sim::{ContentKey, Fixed, PlayerId, TickOutput, Vec3 as SimVec3};
    use std::sync::Arc;

    struct Frozen(Arc<WorldView>);

    impl WorldSource for Frozen {
        fn step(&mut self, _: &[Command]) -> TickOutput {
            TickOutput {
                view: self.0.clone(),
                events: Vec::new(),
            }
        }

        fn view(&self) -> Arc<WorldView> {
            self.0.clone()
        }
    }

    fn unit(id: u32, flags: UnitFlags) -> UnitView {
        UnitView {
            id: EntityId(id),
            def: ContentKey::NONE,
            owner: PlayerId(0),
            pos: SimVec3::ZERO,
            heading: (Fixed::ZERO, Fixed::ONE),
            speed: Fixed::ZERO,
            hull: [Fixed::ONE; 2],
            shields: [[Fixed::ZERO; 2]; 4],
            crew: None,
            energy: None, autonomy: None,
            flags,
            progress: None,
        }
    }

    fn fixture_app(view: WorldView) -> App {
        let view = Arc::new(view);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<HoveredUnit>()
            .init_resource::<Sticky>()
            .init_resource::<Aim>()
            .init_resource::<HudPointer>()
            .insert_resource(Selection(vec![EntityId(99)]))
            .insert_resource(SimLink::new(
                Box::new(Frozen(view.clone())),
                Box::new(LocalRouter::new()),
                PlayerId(0),
            ))
            .insert_resource(HudFeed {
                view,
                player: PlayerId(0),
                selected: vec![EntityId(99)],
                content: None, ..Default::default()
            })
            .add_systems(Update, track);
        app
    }

    /// The screen centre of the 800 x 600 test view.
    const CENTRE: Vec2 = Vec2::new(400.0, 300.0);

    /// A right-handed perspective (depth 0..1) for the 800 x 600 view, vertical field of view 1 radian,
    /// written out so the tests do not depend on a library constructor.
    fn test_projection() -> Mat4 {
        let (near, far) = (0.1f32, 1000.0f32);
        let f = 1.0 / (0.5f32).tan();
        let aspect = 800.0 / 600.0;
        Mat4::from_cols(
            Vec4::new(f / aspect, 0.0, 0.0, 0.0),
            Vec4::new(0.0, f, 0.0, 0.0),
            Vec4::new(0.0, 0.0, far / (near - far), -1.0),
            Vec4::new(0.0, 0.0, near * far / (near - far), 0.0),
        )
    }

    /// A camera at the origin looking down -Z through a perspective view: a unit at (0, 0, -d) is under
    /// the screen centre, and the clip matrix projects every other point consistently with `cursor`.
    fn aim(cursor: Vec2, origin: Vec3, direction: Dir3) -> Aim {
        Aim(Some(AimState {
            cursor,
            ray: Ray3d::new(origin, direction),
            clip_from_world: test_projection(),
            size: Vec2::new(800.0, 600.0),
        }))
    }

    fn root(app: &mut App, id: u32, position: Vec3) {
        app.world_mut().spawn((
            SimEntity(EntityId(id)),
            SelectRadius(12.0),
            GlobalTransform::from(Transform::from_translation(position)),
        ));
    }

    fn visible_world(flags: UnitFlags) -> WorldView {
        WorldView {
            units: vec![unit(1, flags)],
            visibility: vec![(PlayerId(0), vec![EntityId(1)])],
            ..Default::default()
        }
    }

    #[test]
    fn body_pick_tracks_live_actor_without_changing_selection() {
        let mut app = fixture_app(visible_world(UnitFlags::SELECTABLE));
        root(&mut app, 1, Vec3::new(0.0, 0.0, -100.0));
        app.world_mut()
            .insert_resource(aim(CENTRE, Vec3::ZERO, Dir3::NEG_Z));
        app.update();
        assert_eq!(app.world().resource::<HoveredUnit>().0, Some(EntityId(1)));
        assert_eq!(app.world().resource::<Selection>().0, [EntityId(99)]);
    }

    #[test]
    fn existing_strategic_symbol_picker_tracks_the_icon() {
        let mut app = fixture_app(visible_world(UnitFlags::SELECTABLE));
        let mut symbol =
            Strategic::new(crate::render::strategic::Shape::Circle, Color::WHITE, 20.0);
        symbol.symbol = true;
        symbol.shown = 1.0;
        symbol.screen = Vec2::new(120.0, 80.0);
        symbol.size_px = 24.0;
        app.world_mut().spawn((
            SimEntity(EntityId(1)),
            SelectRadius(1.0),
            GlobalTransform::from(Transform::from_xyz(500.0, 0.0, 500.0)),
            symbol,
        ));
        app.world_mut()
            .insert_resource(aim(Vec2::new(120.0, 80.0), Vec3::ZERO, Dir3::NEG_Z));
        app.update();
        assert_eq!(app.world().resource::<HoveredUnit>().0, Some(EntityId(1)));
    }

    #[test]
    fn hud_and_fog_block_hover_even_when_a_render_target_remains() {
        let mut app = fixture_app(visible_world(UnitFlags::SELECTABLE));
        root(&mut app, 1, Vec3::new(0.0, 0.0, -100.0));
        app.world_mut().resource_mut::<HoveredUnit>().0 = Some(EntityId(1));
        app.world_mut()
            .insert_resource(aim(CENTRE, Vec3::ZERO, Dir3::NEG_Z));
        app.world_mut().resource_mut::<HudPointer>().0 = true;
        app.update();
        assert_eq!(app.world().resource::<HoveredUnit>().0, None);

        app.world_mut().resource_mut::<HudPointer>().0 = false;
        app.world_mut().resource_mut::<HudFeed>().view = Arc::new(WorldView {
            units: vec![unit(1, UnitFlags::SELECTABLE)],
            visibility: vec![(PlayerId(0), Vec::new())],
            ..Default::default()
        });
        app.world_mut().resource_mut::<HoveredUnit>().0 = Some(EntityId(1));
        app.update();
        assert_eq!(app.world().resource::<HoveredUnit>().0, None);
    }

    #[test]
    fn hover_reaches_visible_selectable_unit_behind_unseen_or_invalid_one() {
        for (front_flags, front_visible) in [(UnitFlags::SELECTABLE, false), (UnitFlags::default(), true)] {
            let mut view = WorldView {
                units: vec![unit(1, front_flags), unit(2, UnitFlags::SELECTABLE)],
                visibility: vec![(PlayerId(0), if front_visible { vec![EntityId(1), EntityId(2)] } else { vec![EntityId(2)] })],
                ..Default::default()
            };
            view.units.sort_by_key(|u| u.id);
            let mut app = fixture_app(view);
            root(&mut app, 1, Vec3::new(0.0, 0.0, -50.0));
            root(&mut app, 2, Vec3::new(0.0, 0.0, -100.0));
            app.insert_resource(aim(CENTRE, Vec3::ZERO, Dir3::NEG_Z));
            app.update();
            assert_eq!(app.world().resource::<HoveredUnit>().0, Some(EntityId(2)));
        }
    }

    #[test]
    fn cursor_leave_missing_and_invalid_units_clear_hover() {
        let mut app = fixture_app(visible_world(UnitFlags::SELECTABLE));
        root(&mut app, 1, Vec3::new(0.0, 0.0, -100.0));
        app.world_mut().resource_mut::<HoveredUnit>().0 = Some(EntityId(1));
        app.update();
        assert_eq!(app.world().resource::<HoveredUnit>().0, None); // no cursor/ray

        app.world_mut().resource_mut::<HoveredUnit>().0 = Some(EntityId(1));
        app.world_mut()
            .insert_resource(aim(CENTRE, Vec3::ZERO, Dir3::NEG_Z));
        app.world_mut().remove_resource::<SimLink>();
        app.update();
        assert_eq!(app.world().resource::<HoveredUnit>().0, None);

        let mut stale = fixture_app(WorldView::default());
        root(&mut stale, 1, Vec3::new(0.0, 0.0, -100.0));
        stale.world_mut().resource_mut::<HoveredUnit>().0 = Some(EntityId(1));
        stale
            .world_mut()
            .insert_resource(aim(CENTRE, Vec3::ZERO, Dir3::NEG_Z));
        stale.update();
        assert_eq!(stale.world().resource::<HoveredUnit>().0, None);

        let mut invalid = fixture_app(visible_world(UnitFlags::DYING));
        root(&mut invalid, 1, Vec3::new(0.0, 0.0, -100.0));
        invalid.world_mut().resource_mut::<HoveredUnit>().0 = Some(EntityId(1));
        invalid
            .world_mut()
            .insert_resource(aim(CENTRE, Vec3::ZERO, Dir3::NEG_Z));
        invalid.update();
        assert_eq!(invalid.world().resource::<HoveredUnit>().0, None);

        let mut not_selectable = fixture_app(visible_world(UnitFlags::default()));
        root(&mut not_selectable, 1, Vec3::new(0.0, 0.0, -100.0));
        not_selectable.world_mut().resource_mut::<HoveredUnit>().0 = Some(EntityId(1));
        not_selectable
            .world_mut()
            .insert_resource(aim(CENTRE, Vec3::ZERO, Dir3::NEG_Z));
        not_selectable.update();
        assert_eq!(not_selectable.world().resource::<HoveredUnit>().0, None);
    }

    #[test]
    fn menu_and_missing_link_clear_hover() {
        let mut app = fixture_app(visible_world(UnitFlags::SELECTABLE));
        root(&mut app, 1, Vec3::new(0.0, 0.0, -100.0));
        let ray = aim(CENTRE, Vec3::ZERO, Dir3::NEG_Z).0;
        let link = app.world().resource::<SimLink>();
        let feed = app.world().resource::<HudFeed>();
        assert!(!can_track(ray, false, true, Some(link), Some(feed)));
        app.world_mut().resource_mut::<HoveredUnit>().0 = Some(EntityId(1));
        app.world_mut().remove_resource::<SimLink>();
        app.world_mut()
            .insert_resource(aim(CENTRE, Vec3::ZERO, Dir3::NEG_Z));
        app.update();
        assert_eq!(app.world().resource::<HoveredUnit>().0, None);
    }
    /// Where a unit at `(x, 0, -depth)` is on the test screen: `aim` projects it, so the tests place the
    /// pointer from the same matrix.
    fn on_screen(position: Vec3) -> Vec2 {
        let AimState { clip_from_world, size, .. } = aim(CENTRE, Vec3::ZERO, Dir3::NEG_Z).0.unwrap();
        AimState { cursor: CENTRE, ray: Ray3d::new(Vec3::ZERO, Dir3::NEG_Z), clip_from_world, size }.project(position).unwrap()
    }

    fn world_with_speed(speed: f32) -> WorldView {
        let mut u = unit(1, UnitFlags::SELECTABLE);
        u.speed = Fixed::from_f64(f64::from(speed));
        WorldView { units: vec![u], visibility: vec![(PlayerId(0), vec![EntityId(1)])], ..Default::default() }
    }

    /// The pick follows the screen-space rule: a still ship is hovered near its centre and not far beside
    /// it, and a faster one is hovered farther out (the tolerance grows with its speed).
    #[test]
    fn the_pick_uses_the_radius_and_the_speed_scaled_box() {
        let centre_world = Vec3::new(0.0, 0.0, -100.0);
        let at = on_screen(centre_world);
        // 12 world units at depth 100 is about 40 pixels: a big box, so only the sphere/box test decides.
        let beside = |px: f32| at + Vec2::new(px, 0.0);
        let hovered = |speed: f32, pointer: Vec2| {
            let mut app = fixture_app(world_with_speed(speed));
            root(&mut app, 1, centre_world);
            app.insert_resource(aim(pointer, Vec3::ZERO, Dir3::NEG_Z));
            app.update();
            app.world().resource::<HoveredUnit>().0
        };
        // Within the fixed radius: hovered.
        assert_eq!(hovered(0.0, beside(super::super::hover_pick::MIN_RADIUS_PX - 1.0)), Some(EntityId(1)));
        // Far outside the sphere and the box: not hovered, however it moves.
        assert_eq!(hovered(0.0, beside(400.0)), None);
        // Just outside the still tolerance but inside the one a fast mover has.
        let probe = (0..200).map(|i| i as f32).find(|px| hovered(0.0, beside(*px)).is_none()).unwrap();
        assert_eq!(hovered(0.0, beside(probe)), None);
        assert_eq!(hovered(500.0, beside(probe)), Some(EntityId(1)), "a fast mover is picked from farther away");
    }

    /// A fast mover stays hovered when the pointer leaves it over empty space, for a moment; a still ship
    /// does not.
    #[test]
    fn a_fast_mover_stays_hovered_over_empty_space_and_a_still_ship_does_not() {
        use bevy::time::TimeUpdateStrategy;
        use std::time::Duration;
        let centre_world = Vec3::new(0.0, 0.0, -100.0);
        let on = on_screen(centre_world);
        let empty = on + Vec2::new(300.0, 0.0);
        for (speed, stays) in [(200.0, true), (0.0, false)] {
            let mut app = fixture_app(world_with_speed(speed));
            app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(100)));
            root(&mut app, 1, centre_world);
            app.insert_resource(aim(on, Vec3::ZERO, Dir3::NEG_Z));
            app.update();
            assert_eq!(app.world().resource::<HoveredUnit>().0, Some(EntityId(1)));
            app.insert_resource(aim(empty, Vec3::ZERO, Dir3::NEG_Z));
            app.update();
            app.update();
            assert_eq!(app.world().resource::<HoveredUnit>().0.is_some(), stays, "speed {speed}");
            if stays {
                for _ in 0..20 {
                    app.update();
                }
                assert_eq!(app.world().resource::<HoveredUnit>().0, None, "held for a limited time only");
            }
        }
    }

    #[test]
    fn readout_timing_team_and_pause_rules() {
        let (a, b, c) = ((EntityId(1), PlayerId(1)), (EntityId(2), PlayerId(1)), (EntityId(3), PlayerId(2)));
        let mut r = HoverReadout::default();
        let t = |r: &mut HoverReadout, dt: f32, h: Option<(EntityId, PlayerId)>| r.tick(dt, h.map(|x| x.0), h.map(|x| x.1));
        t(&mut r, 0.1, Some(a));
        // First sight of a team is a change from "nothing": the fade starts at 0.
        assert_eq!(r.fade, 0.0);
        for _ in 0..99 {
            t(&mut r, 0.01, Some(a));
        }
        assert!(!r.bars_up(), "0.99 s: absent (fade {})", r.fade);
        t(&mut r, 0.02, Some(a));
        assert!(r.bars_up(), "1.01 s: present (fade {})", r.fade);
        // Full exactly when 2 s have run, not before.
        let mut r = HoverReadout::default();
        t(&mut r, 0.0, Some(a));
        t(&mut r, 1.0, Some(a));
        assert_eq!(r.fade, 0.5);
        assert!(!r.bars_up(), "exactly 1.0 s: not yet");
        t(&mut r, 0.99, Some(a));
        assert!(r.fade < 1.0);
        t(&mut r, 0.01, Some(a));
        assert!((r.fade - 1.0).abs() < 1e-6, "full at 2.0 s: {}", r.fade);
        t(&mut r, 5.0, Some(a));
        assert_eq!(r.fade, 1.0, "capped");
        // A same-team handover keeps the fade and the bars.
        t(&mut r, 0.01, Some(b));
        assert_eq!((r.unit, r.fade), (Some(b.0), 1.0));
        // Another team resets it.
        t(&mut r, 0.01, Some(c));
        assert!(!r.bars_up());
        assert_eq!(r.fade, 0.0);
        t(&mut r, 1.5, Some(c));
        assert!(r.bars_up());
        // Nothing hovered is a team of its own: the fade resets and nothing is up.
        t(&mut r, 0.01, None);
        assert!(!r.bars_up());
        assert_eq!((r.unit, r.team, r.fade), (None, None, 0.0));
        t(&mut r, 10.0, None);
        assert!(!r.bars_up(), "no unit, no bars");
        // The next hover starts from 0 again.
        t(&mut r, 0.01, Some(a));
        assert_eq!(r.fade, 0.0);
    }

    #[test]
    fn neutral_units_are_the_team_of_nothing_and_show_no_bars() {
        let mut r = HoverReadout::default();
        // A neutral unit hovered for ages: the fade runs but nothing is up.
        r.tick(0.0, Some(EntityId(9)), None);
        r.tick(5.0, Some(EntityId(9)), None);
        assert_eq!(r.fade, 1.0);
        assert!(!r.bars_up());
        // Nothing to neutral does not reset the fade.
        r.tick(0.1, None, None);
        assert_eq!(r.fade, 1.0);
        r.tick(0.1, Some(EntityId(9)), None);
        assert_eq!(r.fade, 1.0);
        // From neutral to an own unit it is a team change: reset.
        r.tick(0.1, Some(EntityId(1)), Some(PlayerId(1)));
        assert_eq!(r.fade, 0.0);
        // Own to neutral resets as well.
        r.tick(3.0, Some(EntityId(1)), Some(PlayerId(1)));
        assert!(r.bars_up());
        r.tick(0.1, Some(EntityId(9)), None);
        assert!(!r.bars_up());
        assert_eq!(r.fade, 0.0);
    }

    #[test]
    fn the_system_maps_a_neutral_owner_to_no_team() {
        use bevy::time::TimeUpdateStrategy;
        use std::time::Duration;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(100)))
            .init_resource::<HoverReadout>()
            .insert_resource(HoveredUnit(Some(EntityId(1))))
            .add_systems(Update, tick_readout);
        let view = WorldView { units: vec![{ let mut u = unit(1, UnitFlags::SELECTABLE); u.owner = sta_sim::world::NEUTRAL; u }], ..Default::default() };
        app.insert_resource(SimLink::new(Box::new(Frozen(Arc::new(view))), Box::new(LocalRouter::new()), PlayerId(0)));
        for _ in 0..30 {
            app.update();
        }
        let r = *app.world().resource::<HoverReadout>();
        assert_eq!((r.unit, r.team), (Some(EntityId(1)), None));
        assert!(!r.bars_up(), "a hovered neutral unit shows no bars");
    }

    #[test]
    fn readout_runs_on_real_time_while_the_sim_is_paused() {
        use bevy::time::TimeUpdateStrategy;
        use std::time::Duration;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(100)))
            .init_resource::<HoverReadout>()
            .insert_resource(HoveredUnit(Some(EntityId(1))))
            .add_systems(Update, tick_readout);
        // Pause the virtual (game) clock: the readout must not care.
        app.world_mut().resource_mut::<Time<Virtual>>().pause();
        let view = WorldView { units: vec![{ let mut u = unit(1, UnitFlags::SELECTABLE); u.owner = PlayerId(3); u }], ..Default::default() };
        app.insert_resource(SimLink::new(Box::new(Frozen(Arc::new(view))), Box::new(LocalRouter::new()), PlayerId(0)));
        for _ in 0..16 {
            app.update();
        }
        let r = *app.world().resource::<HoverReadout>();
        assert_eq!(r.team, Some(PlayerId(3)));
        assert!(r.bars_up(), "fade {} after real time", r.fade);
    }
}
