use super::*;
use crate::command::{Command, CommandKind, Edit, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};
use sta_content::{ContentDb, DefRef, MapDef, Profile, UnitDef};
use sta_core::TeamId;
use std::sync::Arc;

const OWNER: PlayerId = PlayerId(0);

fn game() -> Sim {
    let mut product = UnitDef::new("push_product", "craft");
    product.hull = Fixed::from_int(100);
    product.build.cost = vec![Fixed::from_int(10)];
    product.build.ticks = 20;

    let mut yard = UnitDef::new("push_yard", "shipyard");
    yard.hull = Fixed::from_int(100);
    yard.build.cost = vec![Fixed::from_int(20)];
    yard.build.ticks = 2;
    yard.build.queue_limit = Some(10);
    yard.build.list = vec![DefRef::new("push_yard")];
    // Slots 0, 3 and 4 of an authored ODF (2 is `null`): the loader types them in order.
    yard.stats.push_build_items = vec!["push_product".into(), "push_product".into(), "missing_class".into()];

    let mut starter = UnitDef::new("push_starter", "shipyard");
    starter.hull = Fixed::from_int(100);
    starter.build.list = vec![DefRef::new("push_yard")];

    let mut features = Profile::Native.features();
    features.pay_on_queue = true;
    let content = Arc::new(ContentDb::builder().profile(Profile::Native).features(features)
        .unit(product).unit(yard).unit(starter).build().unwrap());
    Sim::from_setup(&GameSetup {
        seed: 20,
        map: Arc::new(MapDef::empty()),
        content,
        players: vec![PlayerSetup { id: OWNER, team: TeamId(0), race: "fx".into(), control: Control::Human }],
        options: GameOptions::default(),
    })
}

fn command(player: PlayerId, kind: CommandKind) -> Command {
    Command { player, seq: 0, kind }
}

fn spawn(sim: &mut Sim, name: &str) -> EntityId {
    let key = sim.content().unwrap().key(name).unwrap();
    sim.spawn_def(key, OWNER, Vec3::ZERO, (Fixed::ZERO, Fixed::ONE)).unwrap()
}

fn fund(sim: &mut Sim) {
    sim.step(&[command(PlayerId::BENCH, CommandKind::Edit(Edit::Cargo(OWNER, 0, Fixed::from_int(100))))]);
}

#[test]
fn finished_immediate_producer_pushes_indexed_duplicates_with_gaps() {
    let mut sim = game();
    fund(&mut sim);
    let yard = spawn(&mut sim, "push_yard");
    let product = sim.content().unwrap().key("push_product").unwrap();
    assert_eq!(sim.production.queue(yard), vec![(product, 0), (product, 0)]);
    assert_eq!(sim.economy.stock_of(OWNER)[0], Fixed::from_int(80));
}

#[test]
fn normally_completed_producer_receives_push_queue_once() {
    let mut sim = game();
    fund(&mut sim);
    let starter = spawn(&mut sim, "push_starter");
    let yard_def = sim.content().unwrap().key("push_yard").unwrap();
    sim.step(&[command(OWNER, CommandKind::Order(Order::Build { builder: starter, what: yard_def, at: None }))]);
    let mut built_yard = None;
    for _ in 0..4 {
        let out = sim.step(&[]);
        for event in out.events {
            if let SimEvent::Built { builder, id } = event
                && builder == starter
            {
                built_yard = Some(id);
            }
        }
        if built_yard.is_some() { break; }
    }
    let yard = built_yard.expect("yard completed");
    let product = sim.content().unwrap().key("push_product").unwrap();
    assert_eq!(sim.production.queue(yard), vec![(product, 0), (product, 0)]);
    assert_eq!(sim.economy.stock_of(OWNER)[0], Fixed::from_int(60));
}
