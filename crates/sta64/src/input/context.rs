//! Context orders (UI-14): what a right-click on a unit means for the
//! selection. A freighter on a resource mines it, a ship on an enemy
//! attacks, a stationary builder rallies, anything else moves (in
//! `orders.rs`). The same decision drives the cursor.
//!
//! Resources are known by what the content says they are (the sim's own
//! [`EntityKind`] rule on class), whoever owns them; miners by the same
//! test the sim uses (class `freighter`, or `miner 1`).

use bevy::prelude::*;
use bevy::window::{CursorIcon, PrimaryWindow, SystemCursorIcon};
use sta_content::ContentDb;
use sta_core::Fixed;
use sta_sim::command::{Order, Target};
use sta_sim::view::{Relation, UnitFlags};
use sta_sim::world::EntityKind;
use sta_sim::{EntityId, WorldView};

use super::orders::Aim;
use super::select::{Targetable, own_selection, unit_under};
use crate::sim_link::{ContentRes, Selection, SimLink};

/// A resource node: a moon, asteroid or latinum field, by its definition.
fn is_resource(db: &ContentDb, view: &WorldView, id: EntityId) -> bool {
    view.unit(id).and_then(|u| db.unit(u.def)).is_some_and(EntityKind::mineable_def)
}

fn is_miner(db: &ContentDb, view: &WorldView, id: EntityId) -> bool {
    view.unit(id).and_then(|u| db.unit(u.def)).is_some_and(|d| d.class.0 == "freighter" || d.miner)
}

fn is_enemy(view: &WorldView, me: sta_sim::PlayerId, owner: sta_sim::PlayerId) -> bool {
    if owner == me || !owner.is_player() {
        return false;
    }
    if let Some(relation) = view.player(me).and_then(|p| p.relations.iter().find(|(id, _)| *id == owner).map(|(_, r)| *r)) {
        return relation == Relation::Enemy;
    }
    match (view.player(me), view.player(owner)) {
        (Some(a), Some(b)) => a.team != b.team,
        _ => true, // Legacy/minimal views may omit player metadata.
    }
}

/// The whole selected group must consist of finished, own, stationary
/// builders. Mobile construction rigs keep their right-click Move behavior.
pub(super) fn rally_builders(
    db: Option<&ContentDb>,
    view: &WorldView,
    me: sta_sim::PlayerId,
    own: &[EntityId],
) -> bool {
    let Some(db) = db else { return false };
    !own.is_empty() && own.iter().all(|&id| {
        view.unit(id).is_some_and(|u| {
            u.owner == me
                && !u.flags.contains(UnitFlags::DYING)
                && u.progress.is_none_or(|p| p >= Fixed::ONE)
                && db.unit(u.def).is_some_and(|d| {
                    !d.build.list.is_empty()
                        && d.movement.max_speed == Fixed::ZERO
                        && (matches!(EntityKind::of_def(d), EntityKind::Station | EntityKind::Craft)
                            || EntityKind::producer_def(d))
                })
        })
    })
}

/// The order a right-click on `target` gives `own`, or `None` for a plain
/// move. `me` is the clicking player.
pub fn context_order(db: Option<&ContentDb>, view: &WorldView, me: sta_sim::PlayerId, own: &[EntityId], target: EntityId) -> Option<Order> {
    let t = view.unit(target)?;
    if t.flags.contains(UnitFlags::DYING) {
        return None;
    }
    if rally_builders(db, view, me, own) {
        return Some(Order::Rally { builders: own.to_vec(), at: Target::Entity(target) });
    }
    if let Some(db) = db.filter(|db| is_resource(db, view, target)) {
        let miners: Vec<EntityId> = own.iter().copied().filter(|&id| is_miner(db, view, id)).collect();
        // Mixed groups keep the existing Move fallback.
        return (miners.len() == own.len() && !miners.is_empty()).then_some(Order::Mine { units: miners, node: target });
    }
    is_enemy(view, me, t.owner).then(|| Order::Attack { units: own.to_vec(), target })
}

/// The cursor shows the order a right-click would give: a crosshair for an
/// attack, a pointing hand for mining or rally, the arrow otherwise.
#[allow(clippy::too_many_arguments)] // Bevy injects these system parameters.
pub fn cursor(
    aim: Res<Aim>,
    units: Targetable,
    selection: Res<Selection>,
    link: Res<SimLink>,
    content: Option<Res<ContentRes>>,
    mut windows: Query<&mut CursorIcon, With<PrimaryWindow>>,
    mut commands: Commands,
    window: Query<Entity, With<PrimaryWindow>>,
) {
    let own = own_selection(&selection, &link);
    let icon = aim.0.filter(|_| !own.is_empty()).and_then(|a| unit_under(a.cursor, a.ray, &units)).and_then(|t| {
        context_order(content.as_deref().map(|c| &*c.0), link.curr(), link.player(), &own, t)
    });
    let want = match icon {
        Some(Order::Attack { .. }) => SystemCursorIcon::Crosshair,
        Some(Order::Mine { .. } | Order::Rally { .. }) => SystemCursorIcon::Pointer,
        None if aim.0.is_some() && rally_builders(content.as_deref().map(|c| &*c.0), link.curr(), link.player(), &own) => {
            SystemCursorIcon::Pointer
        }
        _ => SystemCursorIcon::Default,
    };
    let want = CursorIcon::from(want);
    match (windows.single_mut(), window.single()) {
        (Ok(mut cur), _) => {
            if *cur != want {
                *cur = want;
            }
        }
        (_, Ok(w)) => {
            commands.entity(w).insert(want);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sta_content::{DefRef, UnitDef, fake};
    use sta_core::{Fixed, TeamId};
    use sta_sim::command::{CommandKind, LocalRouter};
    use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
    use sta_sim::{PlayerId, Sim, TickOutput, Vec3 as SimVec3, WorldSource};

    use super::super::keys::KeyBindings;
    use super::super::orders::{OrderTargets, order};
    use super::super::select::AimState;
    use super::*;
    use crate::render::units::SelectRadius;
    use crate::sim_link::{CommandQueue, OrderIssued, OrderKind, SimEntity};

    struct ViewSource(Arc<WorldView>);

    impl WorldSource for ViewSource {
        fn step(&mut self, _: &[sta_sim::command::Command]) -> TickOutput {
            TickOutput { view: self.0.clone(), events: Vec::new() }
        }

        fn view(&self) -> Arc<WorldView> {
            self.0.clone()
        }
    }

    /// Seat 0 (us) has a miner and a cruiser; seat 1 a cruiser; a moon owned
    /// by seat 0 (the wrongly-owned case) and one owned by nobody. Returns
    /// ids `[miner, cruiser, enemy, moon_owned, moon_free, yard, rig, second_yard, ally]`.
    fn app(select: usize) -> (App, [EntityId; 9]) {
        let fake = fake::db();
        let mut builder = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for unit in fake.units() {
            builder = builder.unit(unit.clone());
        }
        for weapon in fake.weapons() {
            builder = builder.weapon(weapon.clone());
        }
        let mut rig = UnitDef::new("fx_rig", "constructionrig");
        rig.movement = fake.unit(fake.key("fx_cruiser").unwrap()).unwrap().movement;
        rig.build.list = vec![DefRef::new("fx_station")];
        rig.hull = Fixed::from_int(100);
        let db = Arc::new(builder.unit(rig).build().unwrap());
        let seat = |id, control| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control };
        let setup = GameSetup {
            seed: 1,
            map: Arc::new(fake::two_bases()),
            content: db.clone(),
            players: vec![
                seat(0, Control::Human),
                seat(1, Control::Idle),
                PlayerSetup { id: PlayerId(2), team: TeamId(0), race: "fx".into(), control: Control::Idle },
            ],
            options: GameOptions::default(),
        };
        let mut sim = Sim::from_setup(&setup);
        let h = (Fixed::ZERO, Fixed::ONE);
        let at = |x: f32| SimVec3::from_f32([x, 0.0, 0.0]);
        let mut spawn = |odf: &str, owner: u8, x: f32| sim.spawn_def(db.key(odf).unwrap(), PlayerId(owner), at(x), h).unwrap();
        let ids = [spawn("fx_miner", 0, 0.0), spawn("fx_cruiser", 0, 100.0), spawn("fx_cruiser", 1, -3000.0), spawn("fx_moon", 0, 3000.0), spawn("fx_moon", 255, 6000.0), spawn("fx_yard", 0, 9000.0), spawn("fx_rig", 0, 12000.0), spawn("fx_yard", 0, 15000.0), spawn("fx_cruiser", 2, -6000.0)];
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<OrderIssued>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<CommandQueue>()
            .init_resource::<OrderTargets>()
            .init_resource::<KeyBindings>()
            .init_resource::<Aim>()
            .insert_resource(ContentRes(db))
            .insert_resource(Selection(vec![ids[select]]))
            .insert_resource(SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0)))
            .add_systems(Update, order);
        for id in ids {
            let x = app.world().resource::<SimLink>().curr().unit(id).map_or(0.0, |u| u.pos.to_f32()[0]);
            app.world_mut().spawn((SimEntity(id), SelectRadius(50.0), GlobalTransform::from(Transform::from_xyz(x, 0.0, 0.0))));
        }
        (app, ids)
    }

    fn right_click_at(app: &mut App, x: f32) -> Vec<CommandKind> {
        right_click_at_xz(app, x, 0.0)
    }

    fn right_click_at_xz(app: &mut App, x: f32, z: f32) -> Vec<CommandKind> {
        let ray = Ray3d::new(Vec3::new(x, 1000.0, z), Dir3::NEG_Y);
        app.world_mut().resource_mut::<Aim>().0 = Some(AimState { cursor: Vec2::ZERO, ray, clip_from_world: Mat4::IDENTITY, size: Vec2::new(800.0, 600.0) });
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Right);
        app.update();
        std::mem::take(&mut app.world_mut().resource_mut::<CommandQueue>().0)
    }

    #[test]
    fn freighter_on_a_moon_mines_whoever_owns_it() {
        for (x, moon) in [(3000.0, 3), (6000.0, 4)] {
            let (mut app, ids) = app(0);
            let cmds = right_click_at(&mut app, x);
            assert_eq!(cmds, [CommandKind::Order(Order::Mine { units: vec![ids[0]], node: ids[moon] })]);
            let msgs: Vec<_> = app.world_mut().resource_mut::<Messages<OrderIssued>>().drain().collect();
            assert_eq!(msgs[0].kind, OrderKind::Mine);
        }
    }

    #[test]
    fn warship_on_a_moon_moves_instead() {
        let (mut app, _) = app(1);
        let cmds = right_click_at(&mut app, 6000.0);
        assert!(matches!(cmds[..], [CommandKind::Order(Order::Move { .. })]), "{cmds:?}");
    }

    #[test]
    fn ship_on_an_enemy_attacks() {
        let (mut app, ids) = app(1);
        let cmds = right_click_at(&mut app, -3000.0);
        assert_eq!(cmds, [CommandKind::Order(Order::Attack { units: vec![ids[1]], target: ids[2] })]);
    }

    #[test]
    fn ship_on_an_ally_moves_and_yard_rallies_to_ally() {
        let (mut ship_app, _) = app(1);
        assert!(matches!(right_click_at(&mut ship_app, -6000.0)[..], [CommandKind::Order(Order::Move { .. })]));

        let (mut yard_app, ids) = app(5);
        assert_eq!(right_click_at(&mut yard_app, -6000.0), [CommandKind::Order(Order::Rally {
            builders: vec![ids[5]], at: Target::Entity(ids[8])
        })]);
    }

    #[test]
    fn neutral_relation_does_not_issue_attack() {
        let (mut app, _) = app(1);
        let link = app.world().resource::<SimLink>();
        let mut view = link.curr().as_ref().clone();
        let player = view.players.iter_mut().find(|p| p.id == PlayerId(0)).unwrap();
        player.relations.iter_mut().find(|(id, _)| *id == PlayerId(1)).unwrap().1 = Relation::Neutral;
        app.world_mut().insert_resource(SimLink::new(Box::new(ViewSource(Arc::new(view))), Box::new(LocalRouter::new()), PlayerId(0)));
        assert!(matches!(right_click_at(&mut app, -3000.0)[..], [CommandKind::Order(Order::Move { .. })]));
    }

    #[test]
    fn yard_right_clicks_rally_to_point_or_live_unit_without_move_ack() {
        let (mut point_app, ids) = app(5);
        let cmds = right_click_at(&mut point_app, 10000.0);
        assert_eq!(cmds, [CommandKind::Order(Order::Rally {
            builders: vec![ids[5]], at: Target::Point(SimVec3::from_f32([10000.0, 0.0, 0.0]))
        })]);
        assert!(point_app.world_mut().resource_mut::<Messages<OrderIssued>>().drain().next().is_none());

        let (mut app, ids) = app(5);
        let cmds = right_click_at(&mut app, 100.0);
        assert_eq!(cmds, [CommandKind::Order(Order::Rally { builders: vec![ids[5]], at: Target::Entity(ids[1]) })]);
        assert!(app.world_mut().resource_mut::<Messages<OrderIssued>>().drain().next().is_none());
    }

    #[test]
    fn mobile_builder_and_mixed_group_keep_move() {
        let (mut rig_app, _) = app(6);
        assert!(matches!(right_click_at(&mut rig_app, 13000.0)[..], [CommandKind::Order(Order::Move { .. })]));

        let (mut app, ids) = app(5);
        app.world_mut().insert_resource(Selection(vec![ids[5], ids[1]]));
        let cmds = right_click_at(&mut app, 10000.0);
        assert_eq!(cmds.len(), 2);
        assert!(cmds.iter().all(|c| matches!(c, CommandKind::Order(Order::Move { .. }))));
    }

    #[test]
    fn two_stationary_builders_share_one_rally_order() {
        let (mut app, ids) = app(5);
        app.world_mut().insert_resource(Selection(vec![ids[5], ids[7]]));
        assert_eq!(right_click_at(&mut app, 10000.0), [CommandKind::Order(Order::Rally {
            builders: vec![ids[5], ids[7]], at: Target::Point(SimVec3::from_f32([10000.0, 0.0, 0.0]))
        })]);
    }

    #[test]
    fn ground_rally_flips_render_z_into_sim_z() {
        let (mut app, ids) = app(5);
        assert_eq!(right_click_at_xz(&mut app, 10000.0, -350.0), [CommandKind::Order(Order::Rally {
            builders: vec![ids[5]], at: Target::Point(SimVec3::from_f32([10000.0, 0.0, 350.0]))
        })]);
    }

    #[test]
    fn rally_eligibility_rejects_unfinished_nonowned_and_stale_ids() {
        let (app, ids) = app(5);
        let link = app.world().resource::<SimLink>();
        let db = &app.world().resource::<ContentRes>().0;
        let mut view = link.curr().as_ref().clone();
        assert!(rally_builders(Some(db), &view, PlayerId(0), &[ids[5], ids[7]]));
        view.units.iter_mut().find(|u| u.id == ids[7]).unwrap().progress = Some(Fixed::ZERO);
        assert!(!rally_builders(Some(db), &view, PlayerId(0), &[ids[5], ids[7]]));
        assert!(!rally_builders(Some(db), &view, PlayerId(0), &[ids[2]]));
        assert!(!rally_builders(Some(db), &view, PlayerId(0), &[EntityId(u32::MAX)]));
    }

    #[test]
    fn typed_planet_context_mining_and_stationary_rally_use_resolved_roles() {
        use sta_content::PlanetRole;
        use sta_sim::view::UnitView;
        use sta_sim::world::NEUTRAL;

        let fake = fake::db();
        let mut builder = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for unit in fake.units() { builder = builder.unit(unit.clone()); }
        for weapon in fake.weapons() { builder = builder.weapon(weapon.clone()); }
        let mut background = UnitDef::new("test_background", "planet_obj");
        background.planet_role = Some(PlanetRole::Background);
        background.resource = Some(0); // metadata does not make a background mineable.
        let mut producer = UnitDef::new("test_producer", "planet");
        producer.planet_role = Some(PlanetRole::Producer);
        producer.resource = Some(0);
        producer.build.list = vec![DefRef::new("fx_miner")];
        let mut empty_producer = UnitDef::new("test_empty_producer", "planet");
        empty_producer.planet_role = Some(PlanetRole::Producer);
        let db = builder.unit(background).unit(producer).unit(empty_producer).build().unwrap();
        let make = |id, name, owner| UnitView {
            id: EntityId(id), def: db.key(name).unwrap(), owner,
            pos: SimVec3::ZERO, heading: (Fixed::ZERO, Fixed::ONE), speed: Fixed::ZERO,
            hull: [Fixed::ONE; 2], shields: [[Fixed::ZERO; 2]; 4], crew: None, energy: None,
            flags: UnitFlags::SELECTABLE, progress: None,
        };
        let view = WorldView {
            units: vec![
                make(1, "fx_miner", PlayerId(0)),
                make(2, "test_background", PlayerId(0)),
                make(3, "test_producer", NEUTRAL),
                make(4, "test_empty_producer", NEUTRAL),
                make(5, "test_producer", PlayerId(0)),
            ],
            ..Default::default()
        };
        assert_eq!(context_order(Some(&db), &view, PlayerId(0), &[EntityId(1)], EntityId(2)), None);
        assert_eq!(
            context_order(Some(&db), &view, PlayerId(0), &[EntityId(1)], EntityId(3)),
            Some(Order::Mine { units: vec![EntityId(1)], node: EntityId(3) })
        );
        assert_eq!(context_order(Some(&db), &view, PlayerId(0), &[EntityId(1)], EntityId(4)), None);
        assert!(rally_builders(Some(&db), &view, PlayerId(0), &[EntityId(5)]));
        assert!(!rally_builders(Some(&db), &view, PlayerId(0), &[EntityId(2)]));
        assert_eq!(
            context_order(Some(&db), &view, PlayerId(0), &[EntityId(5)], EntityId(3)),
            Some(Order::Rally { builders: vec![EntityId(5)], at: Target::Entity(EntityId(3)) })
        );
    }
}
