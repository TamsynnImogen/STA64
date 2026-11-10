//! Context orders (UI-14): what a right-click on a unit means for the
//! selection. A freighter on a resource mines it, a ship on an enemy
//! attacks, anything else moves (in `orders.rs`). The same decision drives
//! the cursor, so what it shows is what the click does.
//!
//! Rally (builder on the map) waits for SIM-17.
//!
//! Resources are known by what the content says they are (the sim's own
//! [`EntityKind`] rule on class), whoever owns them; miners by the same
//! test the sim uses (class `freighter`, or `miner 1`).

use bevy::prelude::*;
use bevy::window::{CursorIcon, PrimaryWindow, SystemCursorIcon};
use sta_content::ContentDb;
use sta_sim::command::Order;
use sta_sim::world::EntityKind;
use sta_sim::{EntityId, WorldView};

use super::orders::Aim;
use super::select::{Targetable, own_selection, unit_under};
use crate::sim_link::{ContentRes, OrderKind, Selection, SimLink};

/// A resource node: a moon, asteroid or latinum field, by its definition.
fn is_resource(db: &ContentDb, view: &WorldView, id: EntityId) -> bool {
    view.unit(id).and_then(|u| db.unit(u.def)).is_some_and(|d| EntityKind::of(&d.class, &d.movement) == EntityKind::Resource)
}

fn is_miner(db: &ContentDb, view: &WorldView, id: EntityId) -> bool {
    view.unit(id).and_then(|u| db.unit(u.def)).is_some_and(|d| d.class.0 == "freighter" || d.miner)
}

/// The order a right-click on `target` gives `own`, or `None` for a plain
/// move. `me` is the clicking player.
pub fn context_order(db: Option<&ContentDb>, view: &WorldView, me: sta_sim::PlayerId, own: &[EntityId], target: EntityId) -> Option<(Order, OrderKind)> {
    let t = view.unit(target)?;
    if let Some(db) = db.filter(|db| is_resource(db, view, target)) {
        let miners: Vec<EntityId> = own.iter().copied().filter(|&id| is_miner(db, view, id)).collect();
        // Mixed groups: only the freighters mine, the rest keep to a move.
        return (miners.len() == own.len() && !miners.is_empty()).then_some((Order::Mine { units: miners, node: target }, OrderKind::Mine));
    }
    (t.owner != me && !t.flags.contains(sta_sim::view::UnitFlags::DYING))
        .then(|| (Order::Attack { units: own.to_vec(), target }, OrderKind::Attack))
}

/// The cursor shows the order a right-click would give: a crosshair for an
/// attack, a pointing hand for mining, the arrow otherwise.
#[allow(clippy::too_many_arguments)]
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
    let want = match icon.map(|i| i.1) {
        Some(OrderKind::Attack) => SystemCursorIcon::Crosshair,
        Some(OrderKind::Mine) => SystemCursorIcon::Pointer,
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

    use sta_content::fake;
    use sta_core::{Fixed, TeamId};
    use sta_sim::command::{CommandKind, LocalRouter};
    use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
    use sta_sim::{PlayerId, Sim, Vec3 as SimVec3};

    use super::super::keys::KeyBindings;
    use super::super::orders::{OrderTargets, order};
    use super::super::select::AimState;
    use super::*;
    use crate::render::units::SelectRadius;
    use crate::sim_link::{CommandQueue, OrderIssued, SimEntity};

    /// Seat 0 (us) has a miner and a cruiser; seat 1 a cruiser; a moon owned
    /// by seat 0 (the wrongly-owned case) and one owned by nobody. Returns
    /// ids `[miner, cruiser, enemy, moon_owned, moon_free]`.
    fn app(select: usize) -> (App, [EntityId; 5]) {
        let db = Arc::new(fake::db());
        let seat = |id, control| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control };
        let setup = GameSetup {
            seed: 1,
            map: Arc::new(fake::two_bases()),
            content: db.clone(),
            players: vec![seat(0, Control::Human), seat(1, Control::Idle)],
            options: GameOptions::default(),
        };
        let mut sim = Sim::from_setup(&setup);
        let h = (Fixed::ZERO, Fixed::ONE);
        let at = |x: f32| SimVec3::from_f32([x, 0.0, 0.0]);
        let mut spawn = |odf: &str, owner: u8, x: f32| sim.spawn_def(db.key(odf).unwrap(), PlayerId(owner), at(x), h).unwrap();
        let ids = [spawn("fx_miner", 0, 0.0), spawn("fx_cruiser", 0, 100.0), spawn("fx_cruiser", 1, -3000.0), spawn("fx_moon", 0, 3000.0), spawn("fx_moon", 255, 6000.0)];
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
        let ray = Ray3d::new(Vec3::new(x, 1000.0, 0.0), Dir3::NEG_Y);
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
}
