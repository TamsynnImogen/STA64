//! Orders: right click moves the selection (Shift queues), or attacks an enemy
//! under the cursor; S (or Space) stops; F1..F3 set the formation. Commands go to [`CommandQueue`]; [`OrderIssued`] says what was
//! asked, for voice acks.

use std::collections::BTreeMap;

use bevy::prelude::*;
use sta_sim::command::{CommandKind, Order, Target};
use sta_sim::{EntityId, Vec3 as SimVec3};

use super::Pointer;
use super::keys::{Action, KeyBindings};
use super::select::{AimState, Targetable, own_selection, target_seen, unit_under_where};
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
    local: Option<Res<crate::render::units::LocalPlayer>>,
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
    if let Some(order) = unit_under_where(cursor, ray, &units, |t| target_seen(view, local.as_deref().and_then(|l| l.0), t))
        .and_then(|t| super::context::context_order(content.as_deref().map(|c| &*c.0), view, me, &own, t))
    {
        let kind = OrderKind::of(&order);
        // The whole selection gets the order; only those that carry it out
        // lose their destination line and acknowledge it.
        let takers = sta_sim::context::takers(content.as_deref().map(|c| &*c.0), view, &order);
        clear_targets(&mut targets, &takers);
        queue.0.push(CommandKind::Order(order));
        if let Some(kind) = kind.filter(|_| !takers.is_empty()) {
            issued.write(OrderIssued { units: takers, kind });
        }
        return;
    }
    let ground = Ground {
        content: content.as_deref().map(|c| &*c.0),
        view,
        me,
        own: &own,
        queue_mod: bindings.pressed(Action::AddModifier, &keys),
        warp: world_warp(&keys, &bindings, content.as_deref().map(|c| &*c.0)),
    };
    let mut acks = Vec::new();
    ground_order(&ground, GroundAt::Ray(ray), &mut OrderOut { queue: &mut queue, targets: &mut targets, issued: &mut acks });
    issued.write_batch(acks);
}

/// Where a ground order is aimed: the ray under the main view's pointer, or a point of the map
/// (the minimap's click, sim x and z).
#[derive(Clone, Copy)]
pub(crate) enum GroundAt {
    Ray(Ray3d),
    Map(Vec2),
}

impl GroundAt {
    /// The sim-space point on the plane at height `y` (the selection's mean height).
    fn on_plane(self, y: f32) -> Option<Vec3> {
        match self {
            GroundAt::Ray(ray) => {
                let dir = *ray.direction;
                if dir.y.abs() < 1e-4 {
                    return None;
                }
                let t = (y - ray.origin.y) / dir.y;
                if t <= 0.0 {
                    return None;
                }
                let hit = ray.origin + dir * t;
                Some(Vec3::new(hit.x, y, -hit.z))
            }
            GroundAt::Map(p) => Some(Vec3::new(p.x, y, p.y)),
        }
    }
}

/// What a ground order is decided from.
pub(crate) struct Ground<'a> {
    pub content: Option<&'a sta_content::ContentDb>,
    pub view: &'a sta_sim::WorldView,
    pub me: sta_sim::PlayerId,
    /// The own selection.
    pub own: &'a [EntityId],
    /// The queue modifier is held.
    pub queue_mod: bool,
    /// A plain move goes out as a warp move (the minimap's rule); rallies are unchanged.
    pub warp: bool,
}

/// Where a ground order goes.
pub(crate) struct OrderOut<'a> {
    pub queue: &'a mut CommandQueue,
    pub targets: &'a mut OrderTargets,
    /// The acknowledgements, for the caller to write.
    pub issued: &'a mut Vec<OrderIssued>,
}

/// The order a right-click on the ground gives: a rally for producers, else a formation move
/// to the point (a warp move when `warp`). The one path of the main view's click and of the
/// minimap's, so the queue modifier, the formation and the acknowledgement are alike.
pub(crate) fn ground_order(g: &Ground, at: GroundAt, out: &mut OrderOut) {
    let Ground { content, view, me, own, queue_mod: shift, warp } = *g;
    // Positions in the sim's space; the order point is on the plane at the
    // selection's mean height.
    let picked: Vec<(EntityId, Vec3)> =
        own.iter().filter_map(|&id| view.unit(id)
            .filter(|u| !u.flags.contains(sta_sim::view::UnitFlags::BUILDING))
            .map(|u| (id, Vec3::from(u.pos.to_f32())))).collect();
    if picked.is_empty() {
        return;
    }
    let centre = picked.iter().fold(Vec3::ZERO, |a, (_, p)| a + *p) / picked.len() as f32;
    let Some(target) = at.on_plane(centre.y) else { return };
    if super::context::rally_builders(content, view, me, own) {
        clear_targets(out.targets, own);
        out.queue.0.push(CommandKind::Order(Order::Rally { builders: own.to_vec(), at: Target::Point(SimVec3::from_f32(target.to_array())) }));
        return;
    }
    for (id, to) in formation_moves(&picked, centre, target) {
        let to = SimVec3::from_f32(to.to_array());
        if !shift {
            out.targets.0.insert(id, to);
        }
        let order = if warp { Order::MoveWarp { units: vec![id], to, queue: shift } } else { Order::Move { units: vec![id], to, queue: shift } };
        out.queue.0.push(CommandKind::Order(order));
    }
    out.issued.push(OrderIssued { units: picked.iter().map(|p| p.0).collect(), kind: OrderKind::Move });
}

/// The minimap's right-click gives a plain move, not a warp move: Alt is held (read from the
/// executable, static); or the queue modifier is held (INTERIM: the original keeps the queue on a
/// warp move, read from the executable, static, but STA64's warp-move order does not queue yet and
/// would wipe the unit's queued legs, so the plain queued move is given); or the content is
/// Armada I's (INTERIM: its rule was not read, and its craft that fly warp themselves are not given
/// a warp job by this click). The minimap cursor reads the same answer. Alt is the keyboard's Alt,
/// not a rebindable action.
pub(crate) fn minimap_plain(keys: &ButtonInput<KeyCode>, bindings: &KeyBindings, content: Option<&sta_content::ContentDb>) -> bool {
    alt_held(keys) || warp_withheld(keys, bindings, content)
}

/// Alt, the keyboard's Alt (not a rebindable action): the warp modifier of the originals.
fn alt_held(keys: &ButtonInput<KeyCode>) -> bool {
    keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight])
}

/// The two interim reasons the minimap gives no warp move (see [`minimap_plain`]): the queue
/// modifier is held, or the content is Armada I's.
fn warp_withheld(keys: &ButtonInput<KeyCode>, bindings: &KeyBindings, content: Option<&sta_content::ContentDb>) -> bool {
    bindings.pressed(Action::AddModifier, keys) || content.is_some_and(|c| c.profile() == sta_content::Profile::Sta1Classic)
}

/// The main view's right-click on open ground (or on an own craft, which gives no order of its
/// own) is a warp move while Alt is held: the same modifier and the same order as the minimap's
/// (read from the executable, static: a move becomes a warp move when Alt is held; the minimap
/// has it the other way round). INTERIM, as for the minimap: not with the queue modifier held and
/// not on Armada I content. Which craft can warp is the sim's existing warp-move rule. The
/// cursor reads the same answer.
pub(crate) fn world_warp(keys: &ButtonInput<KeyCode>, bindings: &KeyBindings, content: Option<&sta_content::ContentDb>) -> bool {
    alt_held(keys) && !warp_withheld(keys, bindings, content)
}

/// A right-click on the minimap (`RadarClick`, Secondary, inside the map's rectangle): the same
/// order a right-click on that ground point would give (no hit-test of a unit's dot: the click
/// gives the point, never a unit), except that a plain move becomes a warp move unless Alt is
/// held. Nothing without an own selection, while a placement or an armed pick waits (interim:
/// armed modes are not aimed from the minimap), or in a cutscene. Grade: the
/// warp move measured once at normal conditions; the Alt case and a short click read from the
/// executable, static, not measured. What the sim does with the warp move of a craft that cannot
/// warp, or of a short leg, is the sim's existing warp-move rule (interim, unchanged here).
#[allow(clippy::too_many_arguments)]
pub fn radar_order(
    mut clicks: MessageReader<crate::ui::radar::RadarClick>,
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<KeyBindings>,
    selection: Res<Selection>,
    link: Res<SimLink>,
    content: Option<Res<ContentRes>>,
    hud: Option<Res<crate::ui::hud::HudState>>,
    cine: Option<Res<crate::ui::presentation::CineractiveState>>,
    mut queue: ResMut<CommandQueue>,
    mut targets: ResMut<OrderTargets>,
    mut issued: MessageWriter<OrderIssued>,
) {
    // The clicks are read (and so dropped) even in a cutscene: one made during it is not kept
    // for the first frames after it.
    let clicks: Vec<_> = clicks
        .read()
        .filter(|c| c.button == bevy::picking::pointer::PointerButton::Secondary && c.inside_map)
        .map(|c| c.at)
        .collect();
    if clicks.is_empty() || cine.is_some_and(|c| c.on) || hud.is_some_and(|h| h.placing.is_some() || h.targeting.is_some()) {
        return;
    }
    let own = own_selection(&selection, &link);
    if own.is_empty() {
        return;
    }
    let ground = Ground {
        content: content.as_deref().map(|c| &*c.0),
        view: link.curr(),
        me: link.player(),
        own: &own,
        queue_mod: bindings.pressed(Action::AddModifier, &keys),
        warp: !minimap_plain(&keys, &bindings, content.as_deref().map(|c| &*c.0)),
    };
    let mut acks = Vec::new();
    for at in clicks {
        ground_order(&ground, GroundAt::Map(at), &mut OrderOut { queue: &mut queue, targets: &mut targets, issued: &mut acks });
    }
    issued.write_batch(acks);
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

    /// A target the local player does not see (a kept-drawn planet outside every sensor, here the
    /// enemy) takes no context order: the click is a move. Seen, it is attacked.
    #[test]
    fn right_click_on_an_unseen_target_moves_and_on_a_seen_one_gives_its_order() {
        use sta_sim::command::Command;
        use sta_sim::{TickOutput, WorldSource, WorldView};
        struct Frozen(std::sync::Arc<WorldView>);
        impl WorldSource for Frozen {
            fn step(&mut self, _: &[Command]) -> TickOutput {
                TickOutput { view: self.0.clone(), events: vec![] }
            }
            fn view(&self) -> std::sync::Arc<WorldView> {
                self.0.clone()
            }
        }
        for (seen, want_attack) in [(false, false), (true, true)] {
            let (mut app, [a, b, e]) = app();
            app.insert_resource(crate::render::units::LocalPlayer(Some(PlayerId(0))));
            let mut view = (**app.world().resource::<SimLink>().curr()).clone();
            view.visibility = vec![(PlayerId(0), [a, b].into_iter().chain(seen.then_some(e)).collect::<std::collections::BTreeSet<_>>().into_iter().collect())];
            app.insert_resource(SimLink::new(Box::new(Frozen(std::sync::Arc::new(view))), Box::new(LocalRouter::new()), PlayerId(0)));
            aim_at(&mut app, -500.0, 0.0);
            let (cmds, _) = click(&mut app, false);
            match &cmds[..] {
                [CommandKind::Order(Order::Attack { target, .. })] => assert!(want_attack && *target == e, "seen={seen}"),
                [CommandKind::Order(Order::Move { .. })] => assert!(!want_attack, "seen={seen}"),
                other => panic!("seen={seen}: {other:?}"),
            }
        }
    }

    #[test]
    fn right_click_uses_visible_target_behind_unseen_sphere_but_keeps_visible_action_gate() {
        use sta_sim::command::Command;
        use sta_sim::{TickOutput, WorldSource, WorldView};
        struct Frozen(std::sync::Arc<WorldView>);
        impl WorldSource for Frozen {
            fn step(&mut self, _: &[Command]) -> TickOutput { TickOutput { view: self.0.clone(), events: vec![] } }
            fn view(&self) -> std::sync::Arc<WorldView> { self.0.clone() }
        }
        for (front_seen, attack) in [(false, true), (true, false)] {
            let (mut app, [a, b, e]) = app();
            app.insert_resource(crate::render::units::LocalPlayer(Some(PlayerId(0))));
            let mut view = (**app.world().resource::<SimLink>().curr()).clone();
            view.visibility = vec![(PlayerId(0), if front_seen { vec![a, b, e] } else { vec![a, e] })];
            app.insert_resource(SimLink::new(Box::new(Frozen(std::sync::Arc::new(view))), Box::new(LocalRouter::new()), PlayerId(0)));
            let front = app.world_mut().query::<(Entity, &SimEntity)>().iter(app.world()).find(|(_, id)| id.0 == b).unwrap().0;
            app.world_mut().entity_mut(front).insert(GlobalTransform::from(Transform::from_xyz(-500.0, 100.0, 0.0)));
            aim_at(&mut app, -500.0, 0.0);
            let (cmds, _) = click(&mut app, false);
            match &cmds[..] {
                [CommandKind::Order(Order::Attack { target, .. })] => assert!(attack && *target == e),
                [CommandKind::Order(Order::Move { .. })] => assert!(!attack),
                other => panic!("front_seen={front_seen}: {other:?}"),
            }
        }
    }

    #[test]
    fn target_seen_follows_the_local_players_visible_set() {
        let (app, [a, _, e]) = app();
        let mut view = (**app.world().resource::<SimLink>().curr()).clone();
        let me = Some(PlayerId(0));
        view.visibility = Vec::new();
        assert!(target_seen(&view, me, e), "a view with no visibility data shows everything");
        view.visibility = vec![(PlayerId(0), vec![a])];
        assert!(target_seen(&view, me, a) && !target_seen(&view, me, e));
        assert!(target_seen(&view, None, e), "no local player: everything drawn");
        assert!(!target_seen(&view, Some(PlayerId(3)), a), "a seat the view does not know sees nothing");
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

    // ---- The minimap's right click (`radar_order`) and the shared ground-order path.

    use crate::ui::radar::{RadarClick, RadarMap, RadarState, click_at};
    use bevy::picking::pointer::PointerButton;

    fn radar_app() -> (App, [EntityId; 3]) {
        let (mut app, ids) = app();
        app.add_message::<RadarClick>()
            .init_resource::<crate::ui::presentation::CineractiveState>()
            .add_systems(Update, radar_order);
        (app, ids)
    }

    fn radar_click(app: &mut App, at: Vec2, button: PointerButton, inside_map: bool, keys: &[KeyCode]) -> (Vec<CommandKind>, Vec<OrderIssued>) {
        for k in keys {
            app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(*k);
        }
        app.world_mut().write_message(RadarClick { at, button, inside_map });
        let out = run(app);
        for k in keys {
            app.world_mut().resource_mut::<ButtonInput<KeyCode>>().release(*k);
        }
        out
    }

    const SEC: PointerButton = PointerButton::Secondary;

    #[test]
    fn a_minimap_right_click_gives_a_warp_move_to_the_map_point() {
        let (mut app, [a, ..]) = radar_app();
        let (cmds, msgs) = radar_click(&mut app, Vec2::new(2000.0, -3000.0), SEC, true, &[]);
        let to = SimVec3::from_f32([2000.0, 0.0, -3000.0]);
        assert_eq!(cmds, [CommandKind::Order(Order::MoveWarp { units: vec![a], to, queue: false })]);
        assert_eq!((msgs.len(), msgs[0].kind, &msgs[0].units), (1, OrderKind::Move, &vec![a]));
        assert_eq!(app.world().resource::<OrderTargets>().0.get(&a), Some(&to));
    }

    #[test]
    fn alt_or_the_queue_modifier_gives_a_plain_move_and_shift_appends_the_leg() {
        let (mut app, [a, ..]) = radar_app();
        let to = SimVec3::from_f32([2000.0, 0.0, 700.0]);
        let (cmds, _) = radar_click(&mut app, Vec2::new(2000.0, 700.0), SEC, true, &[KeyCode::AltRight]);
        assert_eq!(cmds, [CommandKind::Order(Order::Move { units: vec![a], to, queue: false })]);
        // INTERIM: the queue modifier gives the plain queued move (a warp move would wipe the
        // queue in today's sim).
        let (cmds, _) = radar_click(&mut app, Vec2::new(2000.0, 700.0), SEC, true, &[KeyCode::ShiftLeft]);
        assert_eq!(cmds, [CommandKind::Order(Order::Move { units: vec![a], to, queue: true })]);
        let (cmds, _) = radar_click(&mut app, Vec2::new(2000.0, 700.0), SEC, true, &[KeyCode::ShiftLeft, KeyCode::AltLeft]);
        assert_eq!(cmds, [CommandKind::Order(Order::Move { units: vec![a], to, queue: true })]);
    }

    /// The world's warp modifier is the minimap's Alt, inverted: Alt gives the warp move, not with
    /// the queue modifier, not on Armada I content.
    #[test]
    fn world_warp_is_alt_without_the_queue_modifier_or_armada_one() {
        let db = sta_content::fake::db();
        let a1 = sta_content::ContentDb::builder().profile(sta_content::Profile::Sta1Classic).features(db.features().clone()).build().unwrap();
        let bindings = KeyBindings::default();
        let mut keys = ButtonInput::<KeyCode>::default();
        assert!(!world_warp(&keys, &bindings, Some(&db)));
        keys.press(KeyCode::AltRight);
        assert!(world_warp(&keys, &bindings, Some(&db)));
        assert!(world_warp(&keys, &bindings, None));
        assert!(!world_warp(&keys, &bindings, Some(&a1)));
        keys.press(KeyCode::ShiftLeft);
        assert!(!world_warp(&keys, &bindings, Some(&db)));
    }

    /// Armada I content: no warp job from the minimap (INTERIM: its rule was not read).
    #[test]
    fn armada_one_content_gets_a_plain_move_from_the_minimap() {
        let db = sta_content::fake::db();
        let a1 = sta_content::ContentDb::builder().profile(sta_content::Profile::Sta1Classic).features(db.features().clone()).build().unwrap();
        let (keys, bindings) = (ButtonInput::<KeyCode>::default(), KeyBindings::default());
        assert!(minimap_plain(&keys, &bindings, Some(&a1)));
        assert_eq!(minimap_plain(&keys, &bindings, Some(&db)), db.profile() == sta_content::Profile::Sta1Classic);
        assert!(!minimap_plain(&keys, &bindings, None));
    }

    /// A shift-queued minimap click appends its leg: the sim is run, and the ship still visits the
    /// legs it had before, in order, then the new one.
    #[test]
    fn a_queued_minimap_click_keeps_the_earlier_legs_in_the_sim() {
        use sta_content::fake;
        use sta_core::TeamId;
        use sta_sim::command::Command;
        use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
        use sta_sim::WorldSource;
        // Two identical sims of authored content: one the order systems read, one stepped.
        let make = || {
            let db = std::sync::Arc::new(fake::db());
            let seat = |id, control| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control };
            let setup = GameSetup {
                seed: 1,
                map: std::sync::Arc::new(fake::two_bases()),
                content: db.clone(),
                players: vec![seat(0, Control::Human), seat(1, Control::Idle)],
                options: GameOptions::default(),
            };
            let mut sim = Sim::from_setup(&setup);
            let id = sim.spawn_def(db.key("fx_cruiser").unwrap(), PlayerId(0), SimVec3::from_f32([5000.0, 0.0, 5000.0]), (Fixed::ZERO, Fixed::ONE)).unwrap();
            (sim, db, id)
        };
        let (read, db, a) = make();
        let (mut sim, _, same) = make();
        assert_eq!(a, same);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<OrderIssued>()
            .add_message::<RadarClick>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<CommandQueue>()
            .init_resource::<OrderTargets>()
            .init_resource::<KeyBindings>()
            .init_resource::<crate::ui::presentation::CineractiveState>()
            .insert_resource(ContentRes(db))
            .insert_resource(Selection(vec![a]))
            .insert_resource(SimLink::new(Box::new(read), Box::new(LocalRouter::new()), PlayerId(0)))
            .add_systems(Update, radar_order);
        let at = |x: f32, z: f32| Vec2::new(5000.0 + x, 5000.0 + z);
        // The first leg by a plain click (Alt), then two queued minimap clicks.
        let mut cmds = Vec::new();
        cmds.extend(radar_click(&mut app, at(300.0, 0.0), SEC, true, &[KeyCode::AltLeft]).0);
        cmds.extend(radar_click(&mut app, at(300.0, 400.0), SEC, true, &[KeyCode::ShiftLeft]).0);
        cmds.extend(radar_click(&mut app, at(700.0, 400.0), SEC, true, &[KeyCode::ShiftLeft]).0);
        assert_eq!(cmds.len(), 3);
        assert!(cmds.iter().all(|c| matches!(c, CommandKind::Order(Order::Move { .. }))), "{cmds:?}");
        // One tick each, as the player's clicks come.
        for (i, kind) in cmds.into_iter().enumerate() {
            WorldSource::step(&mut sim, &[Command { player: PlayerId(0), seq: i as u32, kind }]);
        }
        let dist = |sim: &Sim, p: Vec2| {
            let u = sim.view().unit(a).unwrap().pos.to_f32();
            Vec2::new(u[0], u[2]).distance(p)
        };
        // The first tick at which the ship is within reach of each point.
        let legs = [at(300.0, 0.0), at(300.0, 400.0), at(700.0, 400.0)];
        let mut first: [Option<u32>; 3] = [None; 3];
        for tick in 0..60_000 {
            WorldSource::step(&mut sim, &[]);
            for (slot, p) in first.iter_mut().zip(legs) {
                if slot.is_none() && dist(&sim, p) < 80.0 {
                    *slot = Some(tick);
                }
            }
            if first[2].is_some() {
                break;
            }
        }
        let [f, s, l] = first.map(|t| t.expect("every leg is visited"));
        assert!(f < s && s < l, "visited in order: {f} {s} {l}");
    }

    #[test]
    fn a_group_keeps_its_offsets_on_a_minimap_move() {
        let (mut app, [a, b, _]) = radar_app();
        app.insert_resource(Selection(vec![a, b]));
        let (cmds, msgs) = radar_click(&mut app, Vec2::new(1000.0, 1000.0), SEC, true, &[]);
        // a at x 0, b at x 500: the centre is x 250.
        let want = |id, x| CommandKind::Order(Order::MoveWarp { units: vec![id], to: SimVec3::from_f32([x, 0.0, 1000.0]), queue: false });
        assert_eq!(cmds, [want(a, 750.0), want(b, 1250.0)]);
        assert_eq!((msgs.len(), &msgs[0].units), (1, &vec![a, b]));
    }

    #[test]
    fn a_minimap_click_issues_nothing_without_an_own_selection_in_a_strip_or_with_another_button() {
        let (mut app, [_, _, e]) = radar_app();
        let at = Vec2::new(100.0, 100.0);
        // Outside the map's rectangle.
        assert!(radar_click(&mut app, at, SEC, false, &[]).0.is_empty());
        // The left button is the camera's.
        assert!(radar_click(&mut app, at, PointerButton::Primary, true, &[]).0.is_empty());
        // An inspected enemy is not ordered.
        app.insert_resource(Selection(vec![e]));
        let (cmds, msgs) = radar_click(&mut app, at, SEC, true, &[]);
        assert!(cmds.is_empty() && msgs.is_empty());
        app.insert_resource(Selection(vec![]));
        assert!(radar_click(&mut app, at, SEC, true, &[]).0.is_empty());
    }

    #[test]
    fn a_minimap_click_issues_nothing_during_a_cutscene() {
        let (mut app, [a, ..]) = radar_app();
        let at = Vec2::new(2000.0, 700.0);
        app.world_mut().resource_mut::<crate::ui::presentation::CineractiveState>().on = true;
        let (cmds, msgs) = radar_click(&mut app, at, SEC, true, &[]);
        assert!(cmds.is_empty() && msgs.is_empty(), "{cmds:?}");
        // After it the minimap orders again.
        app.update();
        app.update();
        app.world_mut().resource_mut::<crate::ui::presentation::CineractiveState>().on = false;
        let (cmds, _) = radar_click(&mut app, at, SEC, true, &[]);
        assert_eq!(cmds.len(), 1, "{a:?} orders again after it");
    }

    /// A producer's right-click on the ground is a rally; the minimap's is the same rally (never
    /// turned into a warp move).
    #[test]
    fn a_minimap_click_with_a_producer_selected_is_a_rally() {
        use sta_content::fake;
        use sta_core::TeamId;
        use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
        let db = std::sync::Arc::new(fake::db());
        let setup = GameSetup {
            seed: 1,
            map: std::sync::Arc::new(fake::two_bases()),
            content: db.clone(),
            players: vec![PlayerSetup { id: PlayerId(0), team: TeamId(0), race: "fx".into(), control: sta_sim::setup::Control::Human }, PlayerSetup { id: PlayerId(1), team: TeamId(1), race: "fx".into(), control: Control::Idle }],
            options: GameOptions::default(),
        };
        let mut sim = Sim::from_setup(&setup);
        let yard = sim.spawn_def(db.key("fx_yard").unwrap(), PlayerId(0), SimVec3::from_f32([9000.0, 0.0, 0.0]), (Fixed::ZERO, Fixed::ONE)).unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<OrderIssued>()
            .add_message::<RadarClick>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<CommandQueue>()
            .init_resource::<OrderTargets>()
            .init_resource::<KeyBindings>()
            .insert_resource(ContentRes(db))
            .insert_resource(Selection(vec![yard]))
            .insert_resource(SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0)))
            .add_systems(Update, radar_order);
        let (cmds, _) = radar_click(&mut app, Vec2::new(8000.0, 350.0), SEC, true, &[]);
        assert_eq!(cmds, [CommandKind::Order(Order::Rally { builders: vec![yard], at: Target::Point(SimVec3::from_f32([8000.0, 0.0, 350.0])) })]);
    }

    /// The main view's order is the shared function with a ray: a plain move, the point on the
    /// plane at the selection's height, render z flipped into sim z (the tests above of the main
    /// view pin the rest); and the same point as a map point gives the same move.
    #[test]
    fn the_main_view_and_the_minimap_share_the_ground_order() {
        let (app, [a, ..]) = app();
        let view = app.world().resource::<SimLink>().curr().clone();
        let own = [a];
        let ground = Ground { content: None, view: &view, me: PlayerId(0), own: &own, queue_mod: false, warp: false };
        let mut orders = Vec::new();
        for at in [GroundAt::Ray(Ray3d::new(Vec3::new(200.0, 1000.0, -300.0), Dir3::NEG_Y)), GroundAt::Map(Vec2::new(200.0, 300.0))] {
            let (mut queue, mut targets, mut acks) = (CommandQueue::default(), OrderTargets::default(), Vec::new());
            ground_order(&ground, at, &mut OrderOut { queue: &mut queue, targets: &mut targets, issued: &mut acks });
            assert_eq!(acks.len(), 1);
            orders.push(queue.0);
        }
        assert_eq!(orders[0], orders[1]);
        assert_eq!(orders[0], [CommandKind::Order(Order::Move { units: vec![a], to: SimVec3::from_f32([200.0, 0.0, 300.0]), queue: false })]);
    }

    /// The click's map point comes from the pointer in the square's own coordinates, so it does
    /// not depend on the minimap's pixel size or the window's aspect; a map that is not square
    /// is centred in the square and its strips are outside the map.
    #[test]
    fn the_click_point_is_independent_of_window_aspect_and_minimap_size_and_strips_are_outside() {
        // A 6000 x 3000 map: the square is 6000 wide, the map fills its middle half in z.
        let (lo, hi) = (Vec2::new(-1000.0, 500.0), Vec2::new(5000.0, 3500.0));
        let state = RadarState { map: Some(RadarMap::of_bounds(lo, hi)), map_fixed: true, bounds: Some((lo, hi)), ..default() };
        let map = state.map.unwrap();
        for window in [Vec2::new(1600.0, 1200.0), Vec2::new(1920.0, 1080.0), Vec2::new(3440.0, 1440.0)] {
            for side in [160.0_f32, 300.0, 320.0 * window.y / 1200.0] {
                // The square sits at the bottom left of the window; the pointer is a window pixel.
                let top_left = Vec2::new(10.0, window.y - 10.0 - side);
                let centre = top_left + Vec2::splat(side / 2.0);
                for world in [Vec2::new(0.0, 1000.0), Vec2::new(2000.0, 2000.0), Vec2::new(4900.0, 3400.0)] {
                    let pixel = top_left + map.to_square(world) * side;
                    // What Bevy's `RelativeCursorPosition` gives: the pointer from the node's
                    // centre in units of its size.
                    let normalized = (pixel - centre) / side;
                    let c = click_at(&state, map, normalized, SEC);
                    assert!((c.at - world).length() < 0.01, "{window:?} {side}: {:?} vs {world:?}", c.at);
                    assert!(c.inside_map);
                }
            }
        }
        // The strips above and below the rectangle are not the map.
        for world in [Vec2::new(2000.0, 100.0), Vec2::new(2000.0, 3900.0)] {
            let normalized = map.to_square(world) - Vec2::splat(0.5);
            let c = click_at(&state, map, normalized, SEC);
            assert!(!c.inside_map && (c.at - world).length() < 0.01);
        }
        // Without a rectangle the whole square is the map.
        let whole = RadarState { map: Some(map), ..default() };
        assert!(click_at(&whole, map, Vec2::new(0.0, 0.4), SEC).inside_map);
    }
}
