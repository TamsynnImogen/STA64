use super::*;
use crate::command::{Command, CommandKind, Edit, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};
use sta_content::{ContentDb, DefRef, MapDef, MoveStats, UnitDef};
use sta_core::TeamId;
use std::sync::Arc;

const OWNER: PlayerId = PlayerId(0);

fn pos(x: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::ZERO,
        z: Fixed::ZERO,
    }
}

fn content(global: bool) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut features = fake.features().clone();
    features.pay_on_queue = global;
    let with_payment = |name: &str, class: &str, pay: bool| {
        let mut u = UnitDef::new(name, class);
        u.race = "fx".into();
        u.hull = Fixed::from_int(100);
        let mut own = features.clone();
        own.pay_on_queue = pay;
        u.features = Some(own);
        u
    };
    let mut immediate = with_payment("charge_immediate", "shipyard", true);
    let mut deferred = with_payment("charge_deferred", "shipyard", false);
    for yard in [&mut immediate, &mut deferred] {
        yard.build.list = vec![
            DefRef::new("charge_product_off"),
            DefRef::new("charge_product_on"),
        ];
    }
    let mut product_off = with_payment("charge_product_off", "craft", false);
    product_off.build.cost = vec![Fixed::from_int(250)];
    product_off.build.ticks = 450;
    let mut product_on = with_payment("charge_product_on", "craft", true);
    product_on.build.cost = vec![Fixed::from_int(250)];
    product_on.build.ticks = 450;
    let mut rig_immediate = with_payment("charge_rig_immediate", "constructionrig", true);
    let mut rig_deferred = with_payment("charge_rig_deferred", "constructionrig", false);
    for rig in [&mut rig_immediate, &mut rig_deferred] {
        rig.movement = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
        rig.build.list = vec![DefRef::new("fx_station")];
    }
    let mut builder = ContentDb::builder()
        .profile(fake.profile())
        .features(features);
    for weapon in fake.weapons() {
        builder = builder.weapon(weapon.clone());
    }
    for unit in fake.units() {
        builder = builder.unit(unit.clone());
    }
    Arc::new(
        builder
            .unit(immediate)
            .unit(deferred)
            .unit(product_off)
            .unit(product_on)
            .unit(rig_immediate)
            .unit(rig_deferred)
            .build()
            .unwrap(),
    )
}

fn game(global: bool) -> Sim {
    Sim::from_setup(&GameSetup {
        seed: 7,
        map: Arc::new(MapDef::empty()),
        content: content(global),
        players: vec![PlayerSetup {
            id: OWNER,
            team: TeamId(0),
            race: "fx".into(),
            control: Control::Human,
        }],
        options: GameOptions::default(),
    })
}

fn cmd(player: PlayerId, kind: CommandKind) -> Command {
    Command {
        player,
        seq: 0,
        kind,
    }
}

fn spawn(sim: &mut Sim, odf: &str, at: Vec3) -> EntityId {
    let def = sim.content().unwrap().key(odf).unwrap();
    sim.spawn_def(def, OWNER, at, (Fixed::ZERO, Fixed::ONE))
        .unwrap()
}

fn stock(sim: &Sim) -> Fixed {
    sim.economy
        .stock_of(OWNER)
        .first()
        .copied()
        .unwrap_or(Fixed::ZERO)
}

fn fund(sim: &mut Sim, amount: i32) {
    let out = sim.step(&[cmd(
        PlayerId::BENCH,
        CommandKind::Edit(Edit::Cargo(OWNER, 0, Fixed::from_int(amount))),
    )]);
    assert!(
        !out.events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
}

fn build(sim: &mut Sim, builder: EntityId, product: &str, at: Option<Vec3>) -> bool {
    let what = sim.content().unwrap().key(product).unwrap();
    let out = sim.step(&[cmd(
        OWNER,
        CommandKind::Order(Order::Build { builder, what, at }),
    )]);
    !out.events
        .iter()
        .any(|e| matches!(e, SimEvent::Rejected { .. }))
}

#[test]
fn each_yard_uses_its_own_charge_flag_instead_of_global_or_product() {
    for global in [false, true] {
        let mut sim = game(global);
        let immediate = spawn(&mut sim, "charge_immediate", pos(0));
        let deferred = spawn(&mut sim, "charge_deferred", pos(1000));
        assert_eq!(sim.content().unwrap().features().pay_on_queue, global);
        fund(&mut sim, 250);
        // Product's false flag cannot defer the immediate builder's charge.
        assert!(build(&mut sim, immediate, "charge_product_off", None));
        assert_eq!(stock(&sim), Fixed::ZERO);
        assert!(
            sim.production
                .to_rec()
                .queues
                .iter()
                .find(|q| q.0 == immediate.0)
                .unwrap()
                .1[0]
                .4
        );
        // Product's true flag cannot charge the deferred builder at queue time.
        assert!(build(&mut sim, deferred, "charge_product_on", None));
        assert_eq!(stock(&sim), Fixed::ZERO);
        let rec = sim.production.to_rec();
        let item = rec.queues.iter().find(|q| q.0 == deferred.0).unwrap().1[0];
        assert!(!item.3 && !item.4, "unfunded head waits without a charge");
        assert!(!build(&mut sim, immediate, "charge_product_off", None));
        assert_eq!(sim.production.queue(immediate).len(), 1);
        assert_eq!(stock(&sim), Fixed::ZERO);
        fund(&mut sim, 250);
        assert_eq!(stock(&sim), Fixed::ZERO, "head starts and charges once");
        let rec = sim.production.to_rec();
        let item = rec.queues.iter().find(|q| q.0 == deferred.0).unwrap().1[0];
        assert!(item.3 && item.4);
        for _ in 0..10 {
            sim.step(&[]);
        }
        assert_eq!(
            stock(&sim),
            Fixed::ZERO,
            "no repeated charge on later ticks"
        );
    }
}

#[test]
fn pending_rig_deploys_use_the_rigs_payment_flags() {
    for global in [false, true] {
        let mut sim = game(global);
        let immediate = spawn(&mut sim, "charge_rig_immediate", pos(0));
        let deferred = spawn(&mut sim, "charge_rig_deferred", pos(1000));
        fund(&mut sim, 1200);
        assert!(build(&mut sim, immediate, "fx_station", Some(pos(4000))));
        assert_eq!(stock(&sim), Fixed::ZERO);
        assert!(build(&mut sim, deferred, "fx_station", Some(pos(5000))));
        assert_eq!(stock(&sim), Fixed::ZERO);
        let rec = sim.production.to_rec();
        assert_eq!(rec.deploys.len(), 2);
        assert!(rec.deploys.iter().find(|r| r.0 == immediate.0).unwrap().4);
        assert!(!rec.deploys.iter().find(|r| r.0 == deferred.0).unwrap().4);
        assert!(!build(&mut sim, immediate, "fx_station", None));
        assert_eq!(stock(&sim), Fixed::ZERO);
        assert!(sim.production.is_deploying(immediate));
        assert!(sim.production.is_deploying(deferred));

        // A deferred rig reaches its site without funds: no child or charge.
        let before = sim.units.len();
        sim.units.get_mut(deferred).unwrap().pos = pos(5000);
        sim.step(&[]);
        assert!(!sim.production.is_deploying(deferred));
        assert_eq!(sim.units.len(), before);
        assert!(sim.production.to_rec().welds.is_empty());
        assert_eq!(stock(&sim), Fixed::ZERO);

        // A fresh deferred order at the site pays on this deploy tick.
        fund(&mut sim, 1200);
        assert!(build(&mut sim, deferred, "fx_station", Some(pos(5000))));
        assert_eq!(stock(&sim), Fixed::ZERO);
        assert_eq!(sim.units.len(), before + 1);
        assert_eq!(sim.production.to_rec().welds.len(), 1);
        assert!(!sim.production.is_deploying(deferred));

        // The prepaid rig creates its own child without charging a second time.
        sim.units.get_mut(immediate).unwrap().pos = pos(4000);
        sim.step(&[]);
        assert_eq!(stock(&sim), Fixed::ZERO);
        assert_eq!(sim.units.len(), before + 2);
        assert_eq!(sim.production.to_rec().welds.len(), 2);
        assert!(!sim.production.is_deploying(immediate));
    }
}
