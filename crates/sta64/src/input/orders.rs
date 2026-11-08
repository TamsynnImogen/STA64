//! Orders: right click moves the selection, keeping its formation; Space
//! stops it. Commands go to [`CommandQueue`]; [`OrderIssued`] says what was
//! asked, for voice acks.

use std::collections::BTreeMap;

use bevy::prelude::*;
use sta_sim::command::{CommandKind, Order};
use sta_sim::{EntityId, Vec3 as SimVec3};

use super::Pointer;
use crate::sim_link::{CommandQueue, OrderIssued, OrderKind, Selection, SimLink};

/// Where each ship was last ordered to go, until it arrives or is stopped,
/// for the waypoint line. The views do not carry a ship's target yet; RND-08
/// and SIM-02 replace this.
#[derive(Resource, Default)]
pub struct OrderTargets(pub BTreeMap<EntityId, SimVec3>);

/// Each unit's move target: the group keeps its offsets from `centre`.
fn formation_moves(picked: &[(EntityId, Vec3)], centre: Vec3, target: Vec3) -> Vec<(EntityId, Vec3)> {
    picked.iter().map(|&(id, p)| (id, target + (p - centre))).collect()
}

#[allow(clippy::too_many_arguments)]
pub fn order(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    pointer: Pointer,
    selection: Res<Selection>,
    link: Res<SimLink>,
    mut queue: ResMut<CommandQueue>,
    mut targets: ResMut<OrderTargets>,
    mut issued: MessageWriter<OrderIssued>,
) {
    if keys.just_pressed(KeyCode::Space) && !selection.0.is_empty() {
        for id in &selection.0 {
            targets.0.remove(id);
        }
        queue.0.push(CommandKind::Order(Order::Stop { units: selection.0.clone() }));
        issued.write(OrderIssued { units: selection.0.clone(), kind: OrderKind::Stop });
    }
    if !buttons.just_pressed(MouseButton::Right) {
        return;
    }
    let Some((_, _, ray)) = pointer.get() else { return };
    // Positions in the sim's space; the order point is on the plane at the
    // selection's mean height.
    let picked: Vec<(EntityId, Vec3)> =
        selection.0.iter().filter_map(|&id| link.curr().unit(id).map(|u| (id, Vec3::from(u.pos.to_f32())))).collect();
    if picked.is_empty() {
        return;
    }
    let centre = picked.iter().fold(Vec3::ZERO, |a, (_, p)| a + *p) / picked.len() as f32;
    let dir = *ray.direction;
    if dir.y.abs() < 1e-4 {
        return;
    }
    let t = (centre.y - ray.origin.y) / dir.y;
    if t <= 0.0 {
        return;
    }
    let hit = ray.origin + dir * t;
    let target = Vec3::new(hit.x, centre.y, -hit.z);
    for (id, to) in formation_moves(&picked, centre, target) {
        let to = SimVec3::from_f32(to.to_array());
        targets.0.insert(id, to);
        queue.0.push(CommandKind::Order(Order::Move { units: vec![id], to, queue: false }));
    }
    issued.write(OrderIssued { units: picked.iter().map(|p| p.0).collect(), kind: OrderKind::Move });
}

/// Forget targets of ships that arrived (the sim snaps them onto it) or are gone.
pub fn prune_targets(link: Res<SimLink>, mut targets: ResMut<OrderTargets>) {
    targets.0.retain(|id, to| link.curr().unit(*id).is_some_and(|u| u.pos != *to));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formation_keeps_offsets_from_the_centre() {
        let picked = [(EntityId(1), Vec3::new(0.0, 0.0, 0.0)), (EntityId(2), Vec3::new(100.0, 0.0, 50.0))];
        let centre = Vec3::new(50.0, 0.0, 25.0);
        let moves = formation_moves(&picked, centre, Vec3::new(1000.0, 0.0, 0.0));
        assert_eq!(moves, [(EntityId(1), Vec3::new(950.0, 0.0, -25.0)), (EntityId(2), Vec3::new(1050.0, 0.0, 25.0))]);
    }
}
