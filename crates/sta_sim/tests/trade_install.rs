//! Installed Armada II commerce regression. Set STA64_TEST_TRADE_INSTALL to
//! a Fleet Operations Roots install with the STA2 Classic mod mounted.

use std::path::Path;
use std::sync::Arc;

use sta_content::{ContentDb, MapDef, OdfIndex, Profile};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order, TradeAction, TradeResource};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};
use sta_vfs::Vfs;

fn n(value: i32) -> Fixed {
    Fixed::from_int(value)
}

fn step(sim: &mut Sim, player: PlayerId, kind: CommandKind) {
    let result = sim.step(&[Command {
        player,
        seq: 0,
        kind,
    }]);
    assert!(
        !result
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. })),
        "{:?}",
        result.events
    );
}

fn trade(sim: &mut Sim, player: PlayerId, unit: EntityId, action: TradeAction) {
    step(
        sim,
        player,
        CommandKind::Order(Order::Trade {
            units: vec![unit],
            action,
        }),
    );
}

#[test]
fn classic_starbase_builds_cargo_and_trading_stations_transfer_a_full_shipment() {
    let Ok(root) = std::env::var("STA64_TEST_TRADE_INSTALL") else {
        return;
    };
    let vfs =
        Vfs::mount_install(Path::new(&root), Some("STA2 Classic")).expect("mount STA2 Classic");
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load_as(&vfs, Profile::Sta2Classic, &index).expect("installed ODFs");
    let db = Arc::new(db);
    let market = db.key("ftrading").expect("Federation trading station");
    let cargo = db.key("fcargo").expect("Federation cargo ship");
    let base = db.key("fbase").expect("Federation starbase");
    assert_eq!(db.unit(cargo).unwrap().trade_cargo_capacity, Some(n(500)));
    let def = db.unit(market).unwrap().trade_station.as_ref().unwrap();
    assert_eq!(def.transfer_rate, n(50));
    assert_eq!(def.goods.len(), 12);

    let owner = PlayerId(0);
    let receiver = PlayerId(1);
    let seat = |id| PlayerSetup {
        id,
        team: TeamId(0),
        race: "federation".into(),
        control: Control::Human,
    };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 5,
        content: db,
        map: Arc::new(MapDef::empty()),
        players: vec![seat(owner), seat(receiver)],
        options: GameOptions::default(),
    });
    let heading = (Fixed::ZERO, Fixed::ONE);
    let builder = sim
        .spawn_def(
            base,
            owner,
            Vec3 {
                x: n(-900),
                ..Vec3::ZERO
            },
            heading,
        )
        .unwrap();
    let source = sim.spawn_def(market, owner, Vec3::ZERO, heading).unwrap();
    let dest = sim
        .spawn_def(
            market,
            receiver,
            Vec3 {
                x: n(4200),
                ..Vec3::ZERO
            },
            heading,
        )
        .unwrap();
    for (player, r, amount) in [
        (owner, 0, 4000),
        (owner, 1, 500),
        (owner, 2, 4000),
        (receiver, 0, 0),
    ] {
        step(
            &mut sim,
            PlayerId::BENCH,
            CommandKind::Edit(Edit::Cargo(player, r, n(amount))),
        );
    }
    step(
        &mut sim,
        PlayerId::BENCH,
        CommandKind::Edit(Edit::TeamCrew(owner, n(2000))),
    );
    step(
        &mut sim,
        PlayerId::BENCH,
        CommandKind::Edit(Edit::MaxOfficers(owner, n(100))),
    );
    trade(
        &mut sim,
        owner,
        source,
        TradeAction::Buy(TradeResource::Dilithium),
    );
    assert_eq!(sim.view().player(owner).unwrap().resources[0], n(4100));

    step(
        &mut sim,
        owner,
        CommandKind::Order(Order::Build {
            builder,
            what: cargo,
            at: None,
        }),
    );
    let mut ship = None;
    for _ in 0..2200 {
        if let Some(unit) = sim
            .view()
            .units
            .iter()
            .find(|u| u.def == cargo && u.progress.is_none())
        {
            ship = Some(unit.id);
            break;
        }
        sim.step(&[]);
    }
    let ship = ship.expect("classic fbase launches fcargo");
    trade(
        &mut sim,
        owner,
        ship,
        TradeAction::Load(TradeResource::Dilithium),
    );
    let before = sim.view().player(owner).unwrap().resources[0];
    for _ in 0..2400 {
        if sim
            .cargo_view()
            .iter()
            .any(|r| r.entity == ship && r.amount == n(500))
        {
            break;
        }
        sim.step(&[]);
    }
    assert!(
        sim.cargo_view()
            .iter()
            .any(|r| r.entity == ship && r.amount == n(500))
    );
    assert_eq!(
        sim.view().player(owner).unwrap().resources[0],
        before - n(500)
    );
    trade(&mut sim, owner, ship, TradeAction::Destination(dest));
    for _ in 0..3200 {
        if sim.view().player(receiver).unwrap().resources[0] == n(500) {
            break;
        }
        sim.step(&[]);
    }
    let vessel = sim.unit(ship).unwrap();
    assert_eq!(
        sim.view().player(receiver).unwrap().resources[0],
        n(500),
        "ship at {:?} headed to {:?} at speed {}, dock {:?}, hold {:?}",
        vessel.pos.to_f32(),
        vessel.target.map(Vec3::to_f32),
        vessel.speed.to_f32(),
        sim.unit(dest).unwrap().pos.to_f32(),
        sim.cargo_view()
    );
    assert!(
        sim.cargo_view()
            .iter()
            .any(|r| r.entity == ship && r.amount == n(0))
    );
}
