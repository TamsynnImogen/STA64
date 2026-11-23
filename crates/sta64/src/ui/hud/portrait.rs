//! A visible selected-unit portrait can re-centre the main camera.

use bevy::picking::{Pickable, events::PointerClick, pointer::PointerButton};
use bevy::prelude::*;
use sta_sim::{EntityId, view::UnitFlags};

use super::{avatar::ChoiceState, model::HudFeed};
use crate::{input::select::CameraFocus, ui::notices::Tooltip};

#[derive(Component)]
pub(super) struct FocusPortrait(EntityId);

pub(super) fn bind(id: EntityId, name: &str, details: Option<&str>) -> (FocusPortrait, Tooltip) {
    let mut tip = Tooltip::new(name);
    if let Some(details) = details {
        tip = tip.verbose(details).stock_class_details();
    }
    (FocusPortrait(id), tip)
}

/// An unresolved Fleet Ops backdrop or wireframe layer must not take clicks.
pub(super) fn sync_pickable(
    mut portraits: Query<(&mut Pickable, Has<ImageNode>, &Visibility), With<FocusPortrait>>,
) {
    for (mut picking, has_image, visibility) in &mut portraits {
        let next = if has_image && *visibility != Visibility::Hidden {
            Pickable::default()
        } else {
            Pickable::IGNORE
        };
        if *picking != next {
            *picking = next;
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn on_click(
    mut click: On<PointerClick>,
    portraits: Query<(&FocusPortrait, &Pickable)>,
    feed: Res<HudFeed>,
    lifecycle: Option<Res<crate::platform::LifecycleGate>>,
    menu: Option<Res<crate::ui::ingame_menu::InGameMenu>>,
    scene: Option<Res<crate::frontend::scene::MenuScene>>,
    avatar: Res<ChoiceState>,
    cine: Option<Res<crate::ui::presentation::CineractiveState>>,
    cheat: Option<Res<crate::cheats::input::CheatInput>>,
    mut focus: MessageWriter<CameraFocus>,
) {
    let Ok((&FocusPortrait(id), picking)) = portraits.get(click.entity) else {
        return;
    };
    if *picking == Pickable::IGNORE {
        return;
    }
    click.propagate(false);
    if click.button != PointerButton::Primary
        || lifecycle.is_some_and(|gate| !gate.is_foreground())
        || menu.is_some_and(|m| m.open)
        || scene.is_some()
        || avatar.is_open()
        || crate::ui::presentation::cutscene_running(cine)
        || cheat.is_some_and(|c| c.open || c.blocked_this_frame)
        || feed.selected.as_slice() != [id]
    {
        return;
    }
    let Some(unit) = feed.view.visible_to(feed.player).into_iter().find(|unit| {
        unit.id == id
            && unit.flags.contains(UnitFlags::SELECTABLE)
            && !unit.flags.contains(UnitFlags::DYING)
    }) else {
        return;
    };
    let [x, y, z] = unit.pos.to_f32();
    focus.write(CameraFocus(Vec3::new(x, y, -z)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::picking::{
        backend::HitData,
        events::Pointer,
        pointer::{Location, PointerId},
    };
    use sta_sim::{WorldSource, fake::ScriptedSource};

    fn click(app: &mut App, entity: Entity, button: PointerButton) {
        let camera = app.world_mut().spawn_empty().id();
        app.world_mut().trigger(PointerClick {
            entity,
            pointer: Pointer::new(
                PointerId::Mouse,
                Location {
                    target: bevy::camera::NormalizedRenderTarget::None {
                        width: 1,
                        height: 1,
                    },
                    position: Vec2::ZERO,
                },
            ),
            button,
            hit: HitData::new(camera, 0.0, None, None),
            duration: std::time::Duration::ZERO,
            count: 1,
        });
    }

    #[test]
    fn only_visible_selected_portraits_focus_the_camera_on_primary_click() {
        let db = sta_content::fake::db();
        let mut source = ScriptedSource::from_json5(
            "{units:[{id:1,def:'fx_cruiser',owner:0,rails:[{at:0,pos:{x:90,y:4,z:30}}]}]}",
            &db,
        )
        .unwrap();
        let view = source.step(&[]).view;
        let id = view.units[0].id;
        let mut app = App::new();
        app.insert_resource(HudFeed {
            view,
            selected: vec![id],
            ..default()
        })
        .init_resource::<ChoiceState>()
        .add_message::<CameraFocus>()
        .add_observer(on_click)
        .add_systems(Update, sync_pickable);
        let art = app
            .world_mut()
            .spawn((FocusPortrait(id), Pickable::IGNORE, Visibility::Inherited))
            .id();
        click(&mut app, art, PointerButton::Primary);
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<CameraFocus>>()
                .drain()
                .count(),
            0
        );
        app.world_mut().entity_mut(art).insert(ImageNode::default());
        app.update();
        click(&mut app, art, PointerButton::Secondary);
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<CameraFocus>>()
                .drain()
                .count(),
            0
        );
        click(&mut app, art, PointerButton::Primary);
        let focused: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<CameraFocus>>()
            .drain()
            .map(|m| m.0)
            .collect();
        assert_eq!(focused, [Vec3::new(90.0, 4.0, -30.0)]);
        app.world_mut().resource_mut::<HudFeed>().selected.clear();
        click(&mut app, art, PointerButton::Primary);
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<CameraFocus>>()
                .drain()
                .count(),
            0
        );
    }
}
