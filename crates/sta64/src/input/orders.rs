//! Orders: right click moves the selection (Shift queues), or attacks an enemy
//! under the cursor; S (or Space) stops; F1..F3 set the formation. Commands go to [`CommandQueue`]; [`OrderIssued`] says what was
//! asked, for voice acks.

use std::collections::BTreeMap;

use bevy::prelude::*;
use sta_sim::command::{CommandKind, Order, Target};
use sta_sim::{EntityId, Vec3 as SimVec3};

use super::Pointer;
use super::keys::{Action, KeyBindings};
use super::select::{AimState, Targetable, own_selection, unit_under};
use crate::sim_link::{CommandQueue, ContentRes, OrderIssued, OrderKind, Selection, SimLink};

/// Where each ship was last ordered to go, until it arrives or is stopped,
/// for the waypoint line. The views do not carry a ship's target yet; RND-08
/// and SIM-02 replace this.
#[derive(Resource, Default)]
pub struct OrderTargets(pub BTreeMap<EntityId, SimVec3>);

/// Each unit's move target: the group keeps its offsets from `centre`.
pub(crate) fn formation_moves(picked: &[(EntityId, Vec3)], centre: Vec3, target: Vec3) -> Vec<(EntityId, Vec3)> {
    picked.iter().map(|&(id, p)| (id, target + (p - centre))).collect()
}

/// The cursor, its ray and the camera projection this frame, shared by
/// selection and orders. Filled by [`aim`]; tests set it.
#[derive(Resource, Default, Clone, Copy)]
pub struct Aim(pub Option<AimState>);

pub fn aim(pointer: Pointer, mut aim: ResMut<Aim>) {
    aim.0 = pointer.get().and_then(|((cam, gt), cursor, ray)| {
        Some(AimState { cursor, ray, clip_from_world: cam.clip_from_view() * gt.to_matrix().inverse(), size: cam.logical_viewport_size()? })
    });
}

#[allow(clippy::too_many_arguments)]
pub fn order(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<KeyBindings>,
    aim: Res<Aim>,
    units: Targetable,
    selection: Res<Selection>,
    link: Res<SimLink>,
    content: Option<Res<ContentRes>>,
    mut queue: ResMut<CommandQueue>,
    mut targets: ResMut<OrderTargets>,
    mut issued: MessageWriter<OrderIssued>,
) {
    // Only own units take orders; an inspected enemy is left alone.
    let own = own_selection(&selection, &link);
    if own.is_empty() {
        return;
    }
    if bindings.just_pressed(Action::Stop, &keys) {
        for id in &own {
            targets.0.remove(id);
        }
        queue.0.push(CommandKind::Order(Order::Stop { units: own.clone() }));
        issued.write(OrderIssued { units: own.clone(), kind: OrderKind::Stop });
    }
    let form = Action::ALL.into_iter().find_map(|a| match a {
        Action::Formation(kind) if bindings.just_pressed(a, &keys) => Some(kind),
        _ => None,
    });
    if let Some(kind) = form {
        queue.0.push(CommandKind::Order(Order::Formation { units: own.clone(), kind }));
        issued.write(OrderIssued { units: own.clone(), kind: OrderKind::Formation });
    }
    if !buttons.just_pressed(MouseButton::Right) {
        return;
    }
    let Some(AimState { cursor, ray, .. }) = aim.0 else { return };
    let me = link.player();
    let view = link.curr();
    // Right-click on a resource or an enemy gives a context order; on
    // anything else it moves.
    if let Some(order) = unit_under(cursor, ray, &units)
        .and_then(|t| super::context::context_order(content.as_deref().map(|c| &*c.0), view, me, &own, t))
    {
        let kind = OrderKind::of(&order);
        // A new order replaces the old destination line.
        clear_targets(&mut targets, &own);
        queue.0.push(CommandKind::Order(order));
        if let Some(kind) = kind {
            issued.write(OrderIssued { units: own.clone(), kind });
        }
        return;
    }
    let shift = bindings.pressed(Action::AddModifier, &keys);
    // Positions in the sim's space; the order point is on the plane at the
    // selection's mean height.
    let picked: Vec<(EntityId, Vec3)> =
        own.iter().filter_map(|&id| view.unit(id).map(|u| (id, Vec3::from(u.pos.to_f32())))).collect();
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
    if super::context::rally_builders(content.as_deref().map(|c| &*c.0), view, me, &own) {
        clear_targets(&mut targets, &own);
        queue.0.push(CommandKind::Order(Order::Rally { builders: own, at: Target::Point(SimVec3::from_f32(target.to_array())) }));
        return;
    }
    for (id, to) in formation_moves(&picked, centre, target) {
        let to = SimVec3::from_f32(to.to_array());
        if !shift {
            targets.0.insert(id, to);
        }
        queue.0.push(CommandKind::Order(Order::Move { units: vec![id], to, queue: shift }));
    }
    issued.write(OrderIssued { units: picked.iter().map(|p| p.0).collect(), kind: OrderKind::Move });
}

/// Forget the destination line of `units` (a new, non-move order replaced it).
pub(crate) fn clear_targets(targets: &mut OrderTargets, units: &[EntityId]) {
    for id in units {
        targets.0.remove(id);
    }
}

/// Which ships were seen under way since their target was set.
#[derive(Default)]
pub struct Underway(std::collections::BTreeSet<EntityId>);

/// Drop the targets of ships that arrived, are gone, or came to rest after
/// moving. The smooth movement model stops a ship short of its point (within
/// `tooCloseToTurn`), so "arrived" is "was moving and is now at rest", not
/// only "is exactly on the point"; a rig that deploys stops the same way.
pub fn prune_targets(link: Res<SimLink>, mut targets: ResMut<OrderTargets>, mut underway: Local<Underway>) {
    prune(link.curr(), &mut targets, &mut underway);
}

fn prune(view: &sta_sim::WorldView, targets: &mut OrderTargets, underway: &mut Underway) {
    targets.0.retain(|id, to| {
        let Some(u) = view.unit(*id) else { return false };
        if u.pos == *to {
            return false;
        }
        if u.speed > sta_sim::Fixed::ZERO {
            underway.0.insert(*id);
            return true;
        }
        !underway.0.contains(id)
    });
    underway.0.retain(|id| targets.0.contains_key(id));
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_sim::command::LocalRouter;
    use crate::render::units::SelectRadius;
    use crate::sim_link::SimEntity;
    use sta_sim::movement::FORM_WEDGE;
    use sta_sim::{Fixed, MoveStats, PlayerId, Sim};

    fn stats() -> MoveStats {
        MoveStats::from_f32(140.0, 100.0, 1.0, 1.0, 1.0)
    }

    /// Own ships a (origin) and b, enemy e, selection = [a]; render-space x
    /// equals sim x, render z is sim -z.
    fn app() -> (App, [EntityId; 3]) {
        let mut sim = Sim::new(1);
        let h = (Fixed::ZERO, Fixed::ONE);
        let a = sim.spawn(0, SimVec3::ZERO, h, stats());
        let b = sim.spawn(0, SimVec3::from_f32([500.0, 0.0, 0.0]), h, stats());
        let e = sim.spawn(1, SimVec3::from_f32([-500.0, 0.0, 0.0]), h, stats());
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<OrderIssued>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<CommandQueue>()
            .init_resource::<OrderTargets>()
            .init_resource::<KeyBindings>()
            .init_resource::<Aim>()
            .insert_resource(Selection(vec![a]))
            .insert_resource(SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0)))
            .add_systems(Update, order);
        for (id, x) in [(a, 0.0), (b, 500.0), (e, -500.0)] {
            app.world_mut().spawn((SimEntity(id), SelectRadius(50.0), GlobalTransform::from(Transform::from_xyz(x, 0.0, 0.0))));
        }
        (app, [a, b, e])
    }

    fn aim_at(app: &mut App, x: f32, z: f32) {
        let ray = Ray3d::new(Vec3::new(x, 1000.0, z), Dir3::NEG_Y);
        app.world_mut().resource_mut::<Aim>().0 = Some(AimState { cursor: Vec2::ZERO, ray, clip_from_world: Mat4::IDENTITY, size: Vec2::new(800.0, 600.0) });
    }

    fn click(app: &mut App, shift: bool) -> (Vec<CommandKind>, Vec<OrderIssued>) {
        if shift {
            app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::ShiftLeft);
        }
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Right);
        run(app)
    }

    fn run(app: &mut App) -> (Vec<CommandKind>, Vec<OrderIssued>) {
        app.update();
        let cmds = std::mem::take(&mut app.world_mut().resource_mut::<CommandQueue>().0);
        let msgs = app.world_mut().resource_mut::<Messages<OrderIssued>>().drain().collect();
        (cmds, msgs)
    }

    #[test]
    fn right_click_on_empty_space_moves() {
        let (mut app, [a, ..]) = app();
        aim_at(&mut app, 200.0, -300.0);
        let (cmds, msgs) = click(&mut app, false);
        let to = SimVec3::from_f32([200.0, 0.0, 300.0]);
        assert_eq!(cmds, [CommandKind::Order(Order::Move { units: vec![a], to, queue: false })]);
        assert_eq!((msgs.len(), msgs[0].kind, &msgs[0].units), (1, OrderKind::Move, &vec![a]));
    }

    #[test]
    fn shift_right_click_queues_the_move() {
        let (mut app, [a, ..]) = app();
        aim_at(&mut app, 200.0, -300.0);
        let (cmds, msgs) = click(&mut app, true);
        let to = SimVec3::from_f32([200.0, 0.0, 300.0]);
        assert_eq!(cmds, [CommandKind::Order(Order::Move { units: vec![a], to, queue: true })]);
        assert_eq!(msgs[0].kind, OrderKind::Move);
    }

    #[test]
    fn a_new_order_replaces_the_old_destination_line() {
        let (mut app, [a, _, e]) = app();
        app.world_mut().resource_mut::<OrderTargets>().0.insert(a, SimVec3::from_f32([200.0, 0.0, 300.0]));
        aim_at(&mut app, -500.0, 0.0);
        let (cmds, _) = click(&mut app, false);
        assert_eq!(cmds, [CommandKind::Order(Order::Attack { units: vec![a], target: e })]);
        assert!(app.world().resource::<OrderTargets>().0.is_empty());
    }

    #[test]
    fn a_ship_that_came_to_rest_short_of_its_point_loses_its_line() {
        let (app, [a, ..]) = app();
        let view = app.world().resource::<SimLink>().curr().clone();
        let mut targets = OrderTargets::default();
        let mut underway = Underway::default();
        targets.0.insert(a, SimVec3::from_f32([300.0, 0.0, 0.0]));
        // Not yet moving (just ordered): kept.
        prune(&view, &mut targets, &mut underway);
        assert_eq!(targets.0.len(), 1);
        // Seen under way, then at rest short of the point: dropped.
        let mut moving = (*view).clone();
        moving.units.iter_mut().find(|u| u.id == a).unwrap().speed = Fixed::from_int(50);
        prune(&moving, &mut targets, &mut underway);
        assert_eq!(targets.0.len(), 1);
        prune(&view, &mut targets, &mut underway);
        assert!(targets.0.is_empty());
    }

    #[test]
    fn right_click_on_an_enemy_attacks() {
        let (mut app, [a, _, e]) = app();
        aim_at(&mut app, -500.0, 0.0);
        let (cmds, msgs) = click(&mut app, false);
        assert_eq!(cmds, [CommandKind::Order(Order::Attack { units: vec![a], target: e })]);
        assert_eq!((msgs.len(), msgs[0].kind), (1, OrderKind::Attack));
    }

    #[test]
    fn right_click_on_an_own_unit_moves_instead() {
        let (mut app, [a, ..]) = app();
        aim_at(&mut app, 500.0, 0.0);
        let (cmds, msgs) = click(&mut app, false);
        let to = SimVec3::from_f32([500.0, 0.0, 0.0]);
        assert_eq!(cmds, [CommandKind::Order(Order::Move { units: vec![a], to, queue: false })]);
        assert_eq!(msgs[0].kind, OrderKind::Move);
    }

    #[test]
    fn s_stops_the_selection() {
        let (mut app, [a, ..]) = app();
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyS);
        let (cmds, msgs) = run(&mut app);
        assert_eq!(cmds, [CommandKind::Order(Order::Stop { units: vec![a] })]);
        assert_eq!(msgs[0].kind, OrderKind::Stop);
    }

    #[test]
    fn f_keys_set_the_formation() {
        let (mut app, [a, ..]) = app();
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::F3);
        let (cmds, msgs) = run(&mut app);
        assert_eq!(cmds, [CommandKind::Order(Order::Formation { units: vec![a], kind: FORM_WEDGE })]);
        assert_eq!(msgs[0].kind, OrderKind::Formation);
    }

    #[test]
    fn an_inspected_enemy_receives_no_order() {
        let (mut app, [_, _, e]) = app();
        app.world_mut().insert_resource(Selection(vec![e]));
        aim_at(&mut app, 200.0, -300.0);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyS);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::F1);
        let (cmds, msgs) = click(&mut app, false);
        assert!(cmds.is_empty() && msgs.is_empty(), "{cmds:?}");
    }

    #[test]
    fn rebinding_stop_moves_the_key() {
        use super::super::keys::Action;
        let (mut app, [a, ..]) = app();
        app.world_mut().resource_mut::<KeyBindings>().rebind(Action::Stop, KeyCode::KeyX);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyS);
        assert!(run(&mut app).0.is_empty());
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyX);
        assert_eq!(run(&mut app).0, [CommandKind::Order(Order::Stop { units: vec![a] })]);
    }

    #[test]
    fn formation_keeps_offsets_from_the_centre() {
        let picked = [(EntityId(1), Vec3::new(0.0, 0.0, 0.0)), (EntityId(2), Vec3::new(100.0, 0.0, 50.0))];
        let centre = Vec3::new(50.0, 0.0, 25.0);
        let moves = formation_moves(&picked, centre, Vec3::new(1000.0, 0.0, 0.0));
        assert_eq!(moves, [(EntityId(1), Vec3::new(950.0, 0.0, -25.0)), (EntityId(2), Vec3::new(1050.0, 0.0, 25.0))]);
    }
}
