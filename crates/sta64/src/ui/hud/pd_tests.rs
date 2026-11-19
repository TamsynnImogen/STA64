//! Headless dispatch coverage for the point-defence HUD action and binding.

use std::sync::Arc;

use bevy::prelude::*;
use sta_content::ContentDb;
use sta_sim::command::{Command, CommandKind, Order};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::{UnitFlags, UnitView};
use sta_sim::{EntityId, Fixed, PlayerId, Sim, TeamId, Vec3, WorldSource, WorldView};

use super::model::{Act, Page, grid};
use super::panels::GridCell;
use super::*;
use crate::input::keys::{Action, KeyBindings};

fn feed() -> HudFeed {
    let fake = sta_content::fake::db();
    let mut features = fake.features().clone();
    features.pd_energy = true;
    features.point_defense_cycles = true;
    let mut builder = ContentDb::builder().profile(fake.profile()).features(features);
    for weapon in fake.weapons() {
        let mut weapon = weapon.clone();
        if weapon.odf == "fx_pd" { weapon.energy_cost = Fixed::from_int(40); }
        builder = builder.weapon(weapon);
    }
    for unit in fake.units() {
        let mut unit = unit.clone();
        if unit.odf == "fx_station" {
            unit.special_energy_max = Fixed::from_int(50);
            unit.special_energy_rate = Fixed::ZERO;
        }
        builder = builder.unit(unit);
    }
    let db = Arc::new(builder.build().unwrap());
    let station = UnitView {
        id: EntityId(1), def: db.key("fx_station").unwrap(), owner: PlayerId(1),
        pos: Vec3::ZERO, heading: (Fixed::ZERO, Fixed::ONE), speed: Fixed::ZERO,
        hull: [Fixed::ONE; 2], shields: [[Fixed::ZERO; 2]; 4], flags: UnitFlags::SELECTABLE,
        progress: None, crew: None, energy: None, autonomy: None,
    };
    HudFeed {
        view: Arc::new(WorldView { units: vec![station], ..Default::default() }),
        player: PlayerId(1), selected: vec![EntityId(1)], content: Some(db), ..Default::default()
    }
}

#[test]
fn click_and_rebound_hotkey_send_the_same_slot_once_per_frame() {
    let feed = feed();
    let act = grid(&feed, Page::Top).into_iter()
        .find(|button| matches!(button.act, Act::Special(_)))
        .expect("PD action").act;
    let Act::Special(slot) = act else { unreachable!() };
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_message::<ButtonClicked>()
        .add_message::<SelectionChanged>()
        .add_message::<OrderIssued>()
        .add_message::<Notice>()
        .insert_resource(feed)
        .insert_resource(KeyBindings::default())
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<HudState>()
        .init_resource::<CommandQueue>()
        .init_resource::<Selection>()
        .init_resource::<OrderTargets>()
        .add_systems(Update, press);
    let cell = app.world_mut().spawn(GridCell(act)).id();
    app.world_mut().resource_mut::<Messages<ButtonClicked>>().write(ButtonClicked(cell));
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyP);
    app.update();
    assert_eq!(app.world().resource::<CommandQueue>().0.len(), 1);
    assert!(matches!(&app.world().resource::<CommandQueue>().0[0], CommandKind::Order(Order::Special { slot: sent, .. }) if *sent == slot));

    app.world_mut().resource_mut::<KeyBindings>().rebind(Action::PointDefence, KeyCode::KeyO);
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyO);
    app.update();
    assert_eq!(app.world().resource::<CommandQueue>().0.len(), 2);
    assert!(matches!(&app.world().resource::<CommandQueue>().0[1], CommandKind::Order(Order::Special { slot: sent, .. }) if *sent == slot));
}

#[test]
fn live_sim_auto_off_refreshes_the_hud_button() {
    let mut feed = feed();
    let db = feed.content.as_ref().unwrap().clone();
    let station = db.key("fx_station").unwrap();
    let slot = sta_sim::specials::point_defence_slot(&db, db.unit(station).unwrap()).unwrap();
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 7,
        map: Arc::new(sta_content::MapDef::empty()),
        content: db,
        players: vec![PlayerSetup {
            id: PlayerId(1), team: TeamId(1), race: "fx".into(), control: Control::Idle,
        }],
        options: GameOptions::default(),
    });
    let id = sim.spawn_def(station, PlayerId(1), Vec3::ZERO, (Fixed::ZERO, Fixed::ONE)).unwrap();
    feed.selected = vec![id];
    sim.step(&[]);
    feed.view = sim.view();
    let button = |feed: &HudFeed| grid(feed, Page::Top).into_iter().find(|b| b.act == Act::Special(slot)).unwrap();
    assert_eq!(button(&feed).label, "PD: OFF");
    sim.step(&[Command {
        player: PlayerId(1), seq: 0,
        kind: model::command(&feed, &button(&feed).act).unwrap().0,
    }]);
    feed.view = sim.view();
    assert!(feed.view.point_defence_on(id, slot));
    assert_eq!(button(&feed).label, "PD: ON");
    for _ in 0..120 { sim.step(&[]); }
    feed.view = sim.view();
    assert!(!feed.view.point_defence_on(id, slot));
    assert_eq!(button(&feed).label, "PD: OFF");
    assert_eq!(button(&feed).look, super::model::Look::Locked);
}
