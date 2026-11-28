//! The "engines offline" hint of a move order (#1031, point 2): STA64's own HUD only.

use super::*;
use crate::sim_link::{OrderIssued, OrderKind};
use crate::ui::profile::UiProfile;
use sta_sim::command::{Command, LocalRouter};
use sta_sim::view::UnitFlags;
use sta_sim::{ContentKey, EntityId, Fixed, TickOutput, WorldSource, WorldView};
use std::sync::Arc;

struct Frozen(Arc<WorldView>);

impl WorldSource for Frozen {
    fn step(&mut self, _: &[Command]) -> TickOutput {
        TickOutput { view: self.0.clone(), events: vec![] }
    }
    fn view(&self) -> Arc<WorldView> {
        self.0.clone()
    }
}

fn unit(id: u32, flags: UnitFlags) -> UnitView {
    UnitView {
        id: EntityId(id),
        def: ContentKey(1),
        owner: PlayerId(0),
        pos: sta_sim::Vec3::ZERO,
        heading: (Fixed::ZERO, Fixed::ONE),
        speed: Fixed::ZERO,
        hull: [Fixed::ONE; 2],
        shields: [[Fixed::ONE; 2]; 4],
        crew: None,
        energy: None,
        autonomy: None,
        flags,
        progress: None,
    }
}

fn ids(v: &[u32]) -> Vec<EntityId> {
    v.iter().map(|i| EntityId(*i)).collect()
}

/// Units 1 and 2 have their engines offline, 3 is healthy.
fn app(profile: Option<UiProfile>) -> App {
    let view = Arc::new(WorldView {
        units: vec![unit(1, UnitFlags::ENGINES_OFF), unit(2, UnitFlags::ENGINES_OFF), unit(3, UnitFlags::default())],
        ..Default::default()
    });
    let mut app = App::new();
    app.add_message::<OrderIssued>()
        .add_message::<Notice>()
        .insert_resource(crate::sim_link::SimLink::new(Box::new(Frozen(view)), Box::new(LocalRouter::new()), PlayerId(0)));
    if let Some(p) = profile {
        app.insert_resource(p);
    }
    // One system for the whole test, as in the game: what it has read it does not read again.
    app.add_systems(Update, engines_offline_hint);
    app
}

fn order(app: &mut App, kind: OrderKind, units: &[u32]) {
    app.world_mut().write_message(OrderIssued { units: ids(units), kind });
}

fn run(app: &mut App) -> Vec<Notice> {
    app.update();
    app.world_mut().resource_mut::<Messages<Notice>>().drain().collect()
}

#[test]
fn a_move_order_to_a_craft_with_offline_engines_raises_the_hint_once_in_the_own_hud() {
    for profile in [Some(UiProfile::Lcars), None] {
        let mut app = app(profile);
        order(&mut app, OrderKind::Move, &[1]);
        assert_eq!(run(&mut app), [Notice::new(Severity::Warn, "Unit: engines offline")], "{profile:?}");
        // Nothing is repeated by the next frame.
        assert!(run(&mut app).is_empty());
    }
}

#[test]
fn a_group_order_raises_one_hint_that_counts_the_craft() {
    let mut app = app(Some(UiProfile::Lcars));
    order(&mut app, OrderKind::Move, &[1, 2, 3]);
    assert_eq!(run(&mut app), [Notice::new(Severity::Warn, "2 ships: engines offline")]);
}

#[test]
fn only_a_move_order_to_a_craft_with_offline_engines_raises_it() {
    let mut app = app(Some(UiProfile::Lcars));
    order(&mut app, OrderKind::Move, &[3]);
    order(&mut app, OrderKind::Stop, &[1]);
    order(&mut app, OrderKind::Attack, &[2]);
    assert!(run(&mut app).is_empty());
}

#[test]
fn the_stock_hud_modes_add_nothing() {
    for profile in [UiProfile::Armada1, UiProfile::Armada2, UiProfile::FleetOps] {
        let mut app = app(Some(profile));
        order(&mut app, OrderKind::Move, &[1, 2]);
        assert!(run(&mut app).is_empty(), "{profile:?}");
        // The order is consumed, not kept for a later change of mode.
        app.insert_resource(UiProfile::Lcars);
        assert!(run(&mut app).is_empty(), "{profile:?}: nothing is kept");
    }
}
