//! Orders: right click moves the selection (Shift queues), or attacks an enemy
//! under the cursor; S (or Space) stops; F1..F3 set the formation. Commands go to [`CommandQueue`]; [`OrderIssued`] says what was
//! asked, for voice acks.

use std::collections::BTreeMap;

use bevy::prelude::*;
use sta_sim::command::{CommandKind, Order};
use sta_sim::{EntityId, Vec3 as SimVec3};

use sta_sim::movement::{FORM_COLUMN, FORM_LINE, FORM_WEDGE};

use super::Pointer;
use crate::render::strategic::Strategic;
use crate::render::units::SelectRadius;
use crate::sim_link::{CommandQueue, OrderIssued, OrderKind, Selection, SimEntity, SimLink};

/// Where each ship was last ordered to go, until it arrives or is stopped,
/// for the waypoint line. The views do not carry a ship's target yet; RND-08
/// and SIM-02 replace this.
#[derive(Resource, Default)]
pub struct OrderTargets(pub BTreeMap<EntityId, SimVec3>);

/// Each unit's move target: the group keeps its offsets from `centre`.
fn formation_moves(picked: &[(EntityId, Vec3)], centre: Vec3, target: Vec3) -> Vec<(EntityId, Vec3)> {
    picked.iter().map(|&(id, p)| (id, target + (p - centre))).collect()
}

/// Keys for orders without a click. UI-01 owns rebinding; this is the table it edits.
#[derive(Resource, Debug, Clone)]
pub struct OrderKeys {
    pub stop: Vec<KeyCode>,
    /// Formation keys and the `movement::FORM_*` kind each asks for.
    pub formations: Vec<(KeyCode, u8)>,
}

impl Default for OrderKeys {
    fn default() -> Self {
        OrderKeys {
            stop: vec![KeyCode::KeyS, KeyCode::Space],
            formations: vec![(KeyCode::F1, FORM_LINE), (KeyCode::F2, FORM_COLUMN), (KeyCode::F3, FORM_WEDGE)],
        }
    }
}

/// The cursor and the ray under it, this frame. Filled by [`aim`]; tests set it.
#[derive(Resource, Default, Clone, Copy)]
pub struct Aim(pub Option<(Vec2, Ray3d)>);

pub fn aim(pointer: Pointer, mut aim: ResMut<Aim>) {
    aim.0 = pointer.get().map(|(_, c, r)| (c, r));
}

type Targetable<'w, 's> = Query<'w, 's, (&'static SimEntity, &'static SelectRadius, &'static GlobalTransform, Option<&'static Strategic>)>;

/// The unit under the cursor: a strategic icon, else the nearest sphere on the ray.
fn unit_under(cursor: Vec2, ray: Ray3d, units: &Targetable) -> Option<EntityId> {
    let icon = units
        .iter()
        .filter_map(|(u, _, _, s)| {
            let s = s.filter(|s| s.shown > 0.5)?;
            let d = s.screen.distance(cursor);
            (d <= s.size_px * 0.6).then_some((d, u.0))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0));
    if let Some((_, id)) = icon {
        return Some(id);
    }
    let mut best: Option<(f32, EntityId)> = None;
    for (u, radius, gt, _) in units {
        let to = gt.translation() - ray.origin;
        let t = to.dot(*ray.direction);
        if t < 0.0 {
            continue;
        }
        if (to - *ray.direction * t).length() <= radius.0 && best.is_none_or(|(bt, _)| t < bt) {
            best = Some((t, u.0));
        }
    }
    best.map(|b| b.1)
}

#[allow(clippy::too_many_arguments)]
pub fn order(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    order_keys: Res<OrderKeys>,
    aim: Res<Aim>,
    units: Targetable,
    selection: Res<Selection>,
    link: Res<SimLink>,
    mut queue: ResMut<CommandQueue>,
    mut targets: ResMut<OrderTargets>,
    mut issued: MessageWriter<OrderIssued>,
) {
    if selection.0.is_empty() {
        return;
    }
    if order_keys.stop.iter().any(|k| keys.just_pressed(*k)) {
        for id in &selection.0 {
            targets.0.remove(id);
        }
        queue.0.push(CommandKind::Order(Order::Stop { units: selection.0.clone() }));
        issued.write(OrderIssued { units: selection.0.clone(), kind: OrderKind::Stop });
    }
    if let Some(&(_, kind)) = order_keys.formations.iter().find(|(k, _)| keys.just_pressed(*k)) {
        queue.0.push(CommandKind::Order(Order::Formation { units: selection.0.clone(), kind }));
        issued.write(OrderIssued { units: selection.0.clone(), kind: OrderKind::Formation });
    }
    if !buttons.just_pressed(MouseButton::Right) {
        return;
    }
    let Some((cursor, ray)) = aim.0 else { return };
    let me = link.player();
    let view = link.curr();
    // Right-click on an enemy attacks it; on anything else it moves.
    if let Some(target) = unit_under(cursor, ray, &units).filter(|t| view.unit(*t).is_some_and(|u| u.owner != me)) {
        queue.0.push(CommandKind::Order(Order::Attack { units: selection.0.clone(), target }));
        issued.write(OrderIssued { units: selection.0.clone(), kind: OrderKind::Attack });
        return;
    }
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    // Positions in the sim's space; the order point is on the plane at the
    // selection's mean height.
    let picked: Vec<(EntityId, Vec3)> =
        selection.0.iter().filter_map(|&id| view.unit(id).map(|u| (id, Vec3::from(u.pos.to_f32())))).collect();
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
        if !shift {
            targets.0.insert(id, to);
        }
        queue.0.push(CommandKind::Order(Order::Move { units: vec![id], to, queue: shift }));
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
    use sta_sim::command::LocalRouter;
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
            .init_resource::<OrderKeys>()
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
        app.world_mut().resource_mut::<Aim>().0 = Some((Vec2::ZERO, ray));
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
    fn formation_keeps_offsets_from_the_centre() {
        let picked = [(EntityId(1), Vec3::new(0.0, 0.0, 0.0)), (EntityId(2), Vec3::new(100.0, 0.0, 50.0))];
        let centre = Vec3::new(50.0, 0.0, 25.0);
        let moves = formation_moves(&picked, centre, Vec3::new(1000.0, 0.0, 0.0));
        assert_eq!(moves, [(EntityId(1), Vec3::new(950.0, 0.0, -25.0)), (EntityId(2), Vec3::new(1050.0, 0.0, 25.0))]);
    }
}
