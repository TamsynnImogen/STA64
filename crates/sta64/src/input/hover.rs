//! The live unit under the map cursor, independent of the current selection.

use bevy::prelude::*;
use sta_sim::EntityId;

use super::orders::Aim;
use super::select::{AimState, Targetable, unit_under};
use crate::frontend::scene::MenuScene;
use crate::sim_link::SimLink;
use crate::ui::hud::{HudFeed, HudPointer};

/// The current live map hover, when one is valid.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoveredUnit(pub Option<EntityId>);

/// Track the current map hover without changing selection or issuing orders.
pub fn track(
    aim: Option<Res<Aim>>,
    pointer: Option<Res<HudPointer>>,
    link: Option<Res<SimLink>>,
    menu_scene: Option<Res<MenuScene>>,
    feed: Option<Res<HudFeed>>,
    units: Targetable,
    mut hovered: ResMut<HoveredUnit>,
) {
    let next = resolve(
        aim.and_then(|a| a.0),
        pointer.is_some_and(|p| p.0),
        menu_scene.is_some(),
        link.as_deref(),
        feed.as_deref(),
        &units,
    );
    if hovered.0 != next {
        hovered.0 = next;
    }
}

fn resolve(
    aim: Option<AimState>,
    over_hud: bool,
    menu_open: bool,
    link: Option<&SimLink>,
    feed: Option<&HudFeed>,
    units: &Targetable<'_, '_>,
) -> Option<EntityId> {
    if !can_track(aim, over_hud, menu_open, link, feed) {
        return None;
    }
    let AimState { cursor, ray, .. } = aim?;
    let link = link?;
    let feed = feed?;
    let id = unit_under(cursor, ray, units)?;
    let live = link.curr().unit(id)?;
    if !live.flags.contains(sta_sim::view::UnitFlags::SELECTABLE)
        || live.flags.contains(sta_sim::view::UnitFlags::DYING)
    {
        return None;
    }
    feed.view
        .visible_to(link.player())
        .iter()
        .any(|u| u.id == id)
        .then_some(id)
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
            energy: None,
            flags,
            progress: None,
        }
    }

    fn fixture_app(view: WorldView) -> App {
        let view = Arc::new(view);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<HoveredUnit>()
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
                content: None,
            })
            .add_systems(Update, track);
        app
    }

    fn aim(cursor: Vec2, origin: Vec3, direction: Dir3) -> Aim {
        Aim(Some(AimState {
            cursor,
            ray: Ray3d::new(origin, direction),
            clip_from_world: Mat4::IDENTITY,
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
            .insert_resource(aim(Vec2::ZERO, Vec3::ZERO, Dir3::NEG_Z));
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
            .insert_resource(aim(Vec2::ZERO, Vec3::ZERO, Dir3::NEG_Z));
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
    fn cursor_leave_missing_and_invalid_units_clear_hover() {
        let mut app = fixture_app(visible_world(UnitFlags::SELECTABLE));
        root(&mut app, 1, Vec3::new(0.0, 0.0, -100.0));
        app.world_mut().resource_mut::<HoveredUnit>().0 = Some(EntityId(1));
        app.update();
        assert_eq!(app.world().resource::<HoveredUnit>().0, None); // no cursor/ray

        app.world_mut().resource_mut::<HoveredUnit>().0 = Some(EntityId(1));
        app.world_mut()
            .insert_resource(aim(Vec2::ZERO, Vec3::ZERO, Dir3::NEG_Z));
        app.world_mut().remove_resource::<SimLink>();
        app.update();
        assert_eq!(app.world().resource::<HoveredUnit>().0, None);

        let mut stale = fixture_app(WorldView::default());
        root(&mut stale, 1, Vec3::new(0.0, 0.0, -100.0));
        stale.world_mut().resource_mut::<HoveredUnit>().0 = Some(EntityId(1));
        stale
            .world_mut()
            .insert_resource(aim(Vec2::ZERO, Vec3::ZERO, Dir3::NEG_Z));
        stale.update();
        assert_eq!(stale.world().resource::<HoveredUnit>().0, None);

        let mut invalid = fixture_app(visible_world(UnitFlags::DYING));
        root(&mut invalid, 1, Vec3::new(0.0, 0.0, -100.0));
        invalid.world_mut().resource_mut::<HoveredUnit>().0 = Some(EntityId(1));
        invalid
            .world_mut()
            .insert_resource(aim(Vec2::ZERO, Vec3::ZERO, Dir3::NEG_Z));
        invalid.update();
        assert_eq!(invalid.world().resource::<HoveredUnit>().0, None);

        let mut not_selectable = fixture_app(visible_world(UnitFlags::default()));
        root(&mut not_selectable, 1, Vec3::new(0.0, 0.0, -100.0));
        not_selectable.world_mut().resource_mut::<HoveredUnit>().0 = Some(EntityId(1));
        not_selectable
            .world_mut()
            .insert_resource(aim(Vec2::ZERO, Vec3::ZERO, Dir3::NEG_Z));
        not_selectable.update();
        assert_eq!(not_selectable.world().resource::<HoveredUnit>().0, None);
    }

    #[test]
    fn menu_and_missing_link_clear_hover() {
        let mut app = fixture_app(visible_world(UnitFlags::SELECTABLE));
        root(&mut app, 1, Vec3::new(0.0, 0.0, -100.0));
        let ray = aim(Vec2::ZERO, Vec3::ZERO, Dir3::NEG_Z).0;
        let link = app.world().resource::<SimLink>();
        let feed = app.world().resource::<HudFeed>();
        assert!(!can_track(ray, false, true, Some(link), Some(feed)));
        app.world_mut().resource_mut::<HoveredUnit>().0 = Some(EntityId(1));
        app.world_mut().remove_resource::<SimLink>();
        app.world_mut()
            .insert_resource(aim(Vec2::ZERO, Vec3::ZERO, Dir3::NEG_Z));
        app.update();
        assert_eq!(app.world().resource::<HoveredUnit>().0, None);
    }
}
