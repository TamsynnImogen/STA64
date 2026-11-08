//! Selection: left click selects (Shift adds), left drag selects by box.
//!
//! A click picks the unit whose strategic icon is under the cursor, else the
//! nearest unit whose sphere the cursor ray passes through. The result goes
//! to [`Selection`]; nothing is marked on the entities.

use bevy::prelude::*;
use sta_sim::EntityId;

use super::Pointer;
use crate::render::strategic::Strategic;
use crate::render::units::SelectRadius;
use crate::sim_link::{Selection, SelectionChanged, SimEntity};

/// The drag rectangle shown while box-selecting.
#[derive(Component)]
pub struct SelectBox;

#[derive(Resource, Default)]
pub struct Drag {
    start: Option<Vec2>,
}

pub fn spawn_select_box(mut commands: Commands) {
    commands.spawn((
        SelectBox,
        Node { position_type: PositionType::Absolute, ..default() },
        BackgroundColor(Color::srgba(0.3, 1.0, 0.4, 0.15)),
        Visibility::Hidden,
    ));
}

#[allow(clippy::too_many_arguments)]
pub fn select(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    pointer: Pointer,
    units: Query<(&SimEntity, &SelectRadius, &GlobalTransform, Option<&Strategic>)>,
    mut selection: ResMut<Selection>,
    mut changed: MessageWriter<SelectionChanged>,
    mut drag: ResMut<Drag>,
    mut boxes: Query<(&mut Node, &mut Visibility), With<SelectBox>>,
) {
    let Some((cam, cursor, ray)) = pointer.get() else { return };
    if buttons.just_pressed(MouseButton::Left) {
        drag.start = Some(cursor);
    }
    let Some(start) = drag.start else { return };
    let dragging = start.distance(cursor) > 6.0;
    if let Ok((mut node, mut vis)) = boxes.single_mut() {
        if dragging && buttons.pressed(MouseButton::Left) {
            let (lo, hi) = (start.min(cursor), start.max(cursor));
            node.left = Val::Px(lo.x);
            node.top = Val::Px(lo.y);
            node.width = Val::Px(hi.x - lo.x);
            node.height = Val::Px(hi.y - lo.y);
            *vis = Visibility::Visible;
        } else {
            *vis = Visibility::Hidden;
        }
    }
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    drag.start = None;
    let add = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let before = selection.0.clone();
    if !add {
        selection.0.clear();
    }
    let mut pick = |id: EntityId| {
        if !selection.0.contains(&id) {
            selection.0.push(id);
        }
    };
    if dragging {
        let (lo, hi) = (start.min(cursor), start.max(cursor));
        for (u, _, gt, _) in &units {
            if let Ok(p) = cam.0.world_to_viewport(cam.1, gt.translation())
                && p.cmpge(lo).all()
                && p.cmple(hi).all()
            {
                pick(u.0);
            }
        }
    } else {
        // A unit whose strategic icon is under the cursor (nearest icon centre)...
        let icon = units
            .iter()
            .filter_map(|(u, _, _, s)| {
                let s = s.filter(|s| s.shown > 0.5)?;
                let d = s.screen.distance(cursor);
                (d <= s.size_px * 0.6).then_some((d, u.0))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, id)) = icon {
            pick(id);
        } else {
            // ...else the nearest unit whose sphere the ray passes through.
            let mut best: Option<(f32, EntityId)> = None;
            for (u, radius, gt, _) in &units {
                let to = gt.translation() - ray.origin;
                let t = to.dot(*ray.direction);
                if t < 0.0 {
                    continue;
                }
                let miss = (to - *ray.direction * t).length();
                if miss <= radius.0 && best.is_none_or(|(bt, _)| t < bt) {
                    best = Some((t, u.0));
                }
            }
            if let Some((_, id)) = best {
                pick(id);
            }
        }
    }
    if selection.0 != before {
        changed.write(SelectionChanged);
    }
}
