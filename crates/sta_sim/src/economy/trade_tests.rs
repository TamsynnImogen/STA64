use std::sync::Arc;

use sta_content::{DefRef, MapDef, MoveStats, TradeGood, UnitDef};
use sta_core::TeamId;

use super::*;
use crate::WorldSource;
use crate::command::{Command, CommandKind, Edit, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};

const HOME: PlayerId = PlayerId(0);
const AWAY: PlayerId = PlayerId(1);

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}

fn game() -> (Sim, Arc<ContentDb>, EntityId, EntityId) {
    let mut station = UnitDef::new("market", "trading_station");
    station.hull = n(1000);
    station.build.list = vec![DefRef::new("merchant")];
    station.trade_station = Some(TradeStationDef {
        transfer_rate: n(50),
        profit_share: n(1) / n(4),
        goods: vec![TradeGood {
            key: "goods".into(),
            value: n(2),
        }],
    });
    let mut cargo = UnitDef::new("merchant", "cargo_ship");
    cargo.hull = n(100);
    cargo.trade_cargo_capacity = Some(n(40));
    cargo.movement = MoveStats::from_f32(400.0, 1.0, 1.0, 1.0, 0.2);
    cargo.build.ticks = 4;
    let content = Arc::new(
        ContentDb::builder()
            .unit(station)
            .unit(cargo)
            .build()
            .unwrap(),
    );
    let seat = |player: PlayerId| PlayerSetup {
        id: player,
        team: TeamId(player.0),
        race: String::new(),
        control: Control::Human,
    };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(MapDef::empty()),
        content: content.clone(),
        players: vec![seat(HOME), seat(AWAY)],
        options: GameOptions::default(),
    });
    let a = spawn(&mut sim, "market", HOME, 0);
    let b = spawn(&mut sim, "market", AWAY, 4_000);
    sim.economy.set_stock(HOME, [n(100), n(143), n(100), n(0)]);
    sim.economy.set_stock(AWAY, [n(0); 4]);
    (sim, content, a, b)
}

fn spawn(sim: &mut Sim, name: &str, owner: PlayerId, x: i32) -> EntityId {
    let key = sim.content().unwrap().key(name).unwrap();
    sim.spawn_def(
        key,
        owner,
        Vec3 {
            x: n(x),
            ..Vec3::ZERO
        },
        (Fixed::ZERO, Fixed::ONE),
    )
    .unwrap()
}

fn issuing(sim: &mut Sim, player: PlayerId, kind: CommandKind) -> bool {
    let out = sim.step(&[Command {
        player,
        seq: 0,
        kind,
    }]);
    !out.events
        .iter()
        .any(|event| matches!(event, crate::SimEvent::Rejected { .. }))
}

fn command(sim: &mut Sim, station_or_ship: EntityId, action: TradeAction) -> bool {
    issuing(
        sim,
        HOME,
        CommandKind::Order(Order::Trade {
            units: vec![station_or_ship],
            action,
        }),
    )
}

fn until(sim: &mut Sim, pred: impl Fn(&Sim) -> bool) {
    for _ in 0..1800 {
        if pred(sim) {
            return;
        }
        sim.step(&[]);
    }
    panic!("trade did not advance: {:?}", sim.economy.trade_rows());
}

fn hold(sim: &Sim, id: EntityId) -> Option<Hold> {
    sim.economy.traders.get(id).and_then(|t| t.hold)
}

fn stock(sim: &Sim, player: PlayerId, resource: usize) -> Fixed {
    sim.economy.stock_of(player)[resource]
}

#[test]
fn exchanges_use_fixed_lots_and_require_owner_and_funds() {
    let (mut sim, _, home, away) = game();
    assert!(!command(
        &mut sim,
        away,
        TradeAction::Buy(TradeResource::Dilithium)
    ));
    assert!(command(
        &mut sim,
        home,
        TradeAction::Buy(TradeResource::Dilithium)
    ));
    assert_eq!(stock(&sim, HOME, 0), n(200));
    assert_eq!(stock(&sim, HOME, 1), n(143) - BUY_PRICE);
    assert!(!command(
        &mut sim,
        home,
        TradeAction::Buy(TradeResource::Metal)
    ));
    assert!(!command(
        &mut sim,
        home,
        TradeAction::Sell(TradeResource::Crew)
    ));
    assert!(command(
        &mut sim,
        home,
        TradeAction::Sell(TradeResource::Metal)
    ));
    assert_eq!(stock(&sim, HOME, 2), n(0));
    assert_eq!(stock(&sim, HOME, 1), n(213) - BUY_PRICE);
    assert!(!command(
        &mut sim,
        home,
        TradeAction::Sell(TradeResource::Metal)
    ));
}

#[test]
fn built_cargo_ship_shuttles_and_pays_both_owners_after_reload() {
    let (mut sim, db, home, away) = game();
    let what = db.key("merchant").unwrap();
    assert!(issuing(
        &mut sim,
        HOME,
        CommandKind::Order(Order::Build {
            builder: home,
            what,
            at: None,
        })
    ));
    until(&mut sim, |sim| {
        sim.units
            .iter()
            .any(|u| u.def == what && u.progress.is_none())
    });
    let ship = sim.units.iter().find(|u| u.def == what).unwrap().id;
    assert!(command(&mut sim, ship, TradeAction::Auto(home)));
    assert!(!command(&mut sim, ship, TradeAction::Auto(home)));
    assert!(command(&mut sim, ship, TradeAction::Auto(away)));
    until(&mut sim, |sim| {
        hold(sim, ship).is_some_and(|h| h.amount < n(40))
    });
    let mut loaded = Sim::from_snapshot(&sim.snapshot(), Some(db)).unwrap();
    assert_eq!(sim.digest(), loaded.digest());
    for _ in 0..1800 {
        if stock(&sim, HOME, 1) == n(1543) {
            break;
        }
        sim.step(&[]);
        loaded.step(&[]);
        assert_eq!(sim.digest(), loaded.digest());
    }
    // 4000 distance, cargo value 2, 25% commission at the other player's port.
    assert_eq!(stock(&sim, AWAY, 1), n(200));
    assert_eq!(stock(&sim, HOME, 1), n(1543));
    assert!(hold(&sim, ship).is_none());
}

#[test]
fn shipment_loads_before_delivery_and_keeps_cargo_when_stopped_or_port_is_lost() {
    let (mut sim, _, home, away) = game();
    let ship = spawn(&mut sim, "merchant", HOME, 10);
    assert!(command(
        &mut sim,
        ship,
        TradeAction::Load(TradeResource::Dilithium)
    ));
    until(&mut sim, |sim| {
        hold(sim, ship).is_some_and(|h| h.amount == n(40))
    });
    assert_eq!(stock(&sim, HOME, 0), n(60));
    assert_eq!(stock(&sim, AWAY, 0), n(0));
    assert_eq!(sim.cargo_view()[0].resource, Some(0));
    assert!(!command(
        &mut sim,
        ship,
        TradeAction::Load(TradeResource::Metal)
    ));
    assert!(command(&mut sim, ship, TradeAction::Destination(away)));
    assert!(issuing(
        &mut sim,
        HOME,
        CommandKind::Order(Order::Stop { units: vec![ship] })
    ));
    assert_eq!(hold(&sim, ship).unwrap().amount, n(40));
    assert!(command(&mut sim, ship, TradeAction::Destination(away)));
    assert!(issuing(
        &mut sim,
        PlayerId::BENCH,
        CommandKind::Edit(Edit::Kill(away))
    ));
    assert_eq!(hold(&sim, ship).unwrap().amount, n(40));
    assert!(sim.economy.traders.get(ship).unwrap().route.is_none());

    let replacement = spawn(&mut sim, "market", AWAY, 4_000);
    assert!(command(
        &mut sim,
        ship,
        TradeAction::Destination(replacement)
    ));
    until(&mut sim, |sim| hold(sim, ship).is_none());
    assert_eq!(stock(&sim, HOME, 0), n(60));
    assert_eq!(stock(&sim, AWAY, 0), n(40));
    assert_eq!(stock(&sim, HOME, 1), n(143));
    assert!(sim.economy.traders.get(ship).unwrap().route.is_none());
    assert_eq!(sim.economy.traders.get(ship).unwrap().source, Some(home));
}

#[test]
fn crew_shipment_does_not_change_officer_limit() {
    let (mut sim, _, _, away) = game();
    sim.economy.set_personnel(HOME, Some(n(20)), Some(n(5)));
    sim.economy.set_personnel(AWAY, Some(n(10)), Some(n(2)));
    let ship = spawn(&mut sim, "merchant", HOME, 10);
    assert!(command(
        &mut sim,
        ship,
        TradeAction::Load(TradeResource::Crew)
    ));
    assert!(command(&mut sim, ship, TradeAction::Destination(away)));
    until(&mut sim, |sim| sim.economy.crew_of(AWAY) == Some(n(30)));
    assert_eq!(sim.economy.crew_of(HOME), Some(n(0)));
    assert_eq!(sim.economy.officer_base(HOME), Some(n(5)));
    assert_eq!(sim.economy.officer_base(AWAY), Some(n(2)));
}

#[test]
fn cargo_ships_queue_for_a_single_berth() {
    let (mut sim, db, home, _) = game();
    let first = spawn(&mut sim, "merchant", HOME, 0);
    let second = spawn(&mut sim, "merchant", HOME, 18);
    assert!(issuing(
        &mut sim,
        HOME,
        CommandKind::Order(Order::Trade {
            units: vec![second, first],
            action: TradeAction::Load(TradeResource::Dilithium),
        })
    ));
    assert_eq!(sim.economy.trade_dock_of(first), Some(home));
    assert!(hold(&sim, first).is_some());
    assert!(hold(&sim, second).is_none());
    let mut loaded = Sim::from_snapshot(&sim.snapshot(), Some(db)).unwrap();
    until(&mut loaded, |sim| {
        hold(sim, first).is_some_and(|h| h.amount == n(40))
    });
    assert!(hold(&loaded, second).is_none());
    until(&mut loaded, |sim| {
        hold(sim, second).is_some_and(|h| h.amount == n(40))
    });
    assert_eq!(stock(&loaded, HOME, 0), n(20));
}
