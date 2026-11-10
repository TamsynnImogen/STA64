use super::*;
use crate::command::{Command, CommandKind, Edit, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};
use sta_content::{ContentDb, DefRef, MapDef, Profile, UnitDef};
use sta_core::TeamId;
use std::sync::Arc;

const OWNER: PlayerId = PlayerId(0);

fn content(profile: Profile) -> Arc<ContentDb> {
    let mut product = UnitDef::new("limit_product", "craft");
    product.race = "fx".into();
    product.hull = Fixed::from_int(100);
    product.build.cost = vec![Fixed::ONE];
    product.build.ticks = 50;
    product.build.queue_limit = Some(0); // The product's cap must never select the builder's limit.
    let yard = |name: &str, limit| {
        let mut u = UnitDef::new(name, "shipyard");
        u.race = "fx".into();
        u.hull = Fixed::from_int(100);
        u.build.list = vec![DefRef::new("limit_product")];
        u.build.queue_limit = limit;
        u
    };
    Arc::new(
        ContentDb::builder()
            .profile(profile)
            .features(profile.features())
            .unit(product)
            .unit(yard("limit_default", None))
            .unit(yard("limit_zero", Some(0)))
            .unit(yard("limit_one", Some(1)))
            .unit(yard("limit_twelve", Some(12)))
            .build()
            .unwrap(),
    )
}

fn game(profile: Profile) -> Sim {
    Sim::from_setup(&GameSetup {
        seed: 20,
        map: Arc::new(MapDef::empty()),
        content: content(profile),
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

fn spawn(sim: &mut Sim, name: &str) -> EntityId {
    let key = sim.content().unwrap().key(name).unwrap();
    sim.spawn_def(key, OWNER, Vec3::ZERO, (Fixed::ZERO, Fixed::ONE))
        .unwrap()
}

fn fund(sim: &mut Sim, n: i32) {
    let out = sim.step(&[cmd(
        PlayerId::BENCH,
        CommandKind::Edit(Edit::Cargo(OWNER, 0, Fixed::from_int(n))),
    )]);
    assert!(
        !out.events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
}

fn stock(sim: &Sim) -> Fixed {
    sim.economy
        .stock_of(OWNER)
        .first()
        .copied()
        .unwrap_or(Fixed::ZERO)
}

fn build(sim: &mut Sim, yard: EntityId) -> bool {
    let what = sim.content().unwrap().key("limit_product").unwrap();
    let out = sim.step(&[cmd(
        OWNER,
        CommandKind::Order(Order::Build {
            builder: yard,
            what,
            at: None,
        }),
    )]);
    !out.events
        .iter()
        .any(|e| matches!(e, SimEvent::Rejected { .. }))
}

fn rejected_without_charge_or_queue_change(sim: &mut Sim, yard: EntityId) {
    let before = sim.clone();
    let before_stock = stock(sim);
    let before_len = sim.production.queue(yard).len();
    assert!(!build(sim, yard));
    assert_eq!(stock(sim), before_stock);
    assert_eq!(sim.production.queue(yard).len(), before_len);
    let mut no_command = before;
    no_command.step(&[]);
    assert_eq!(sim.production.to_rec(), no_command.production.to_rec());
    assert_eq!(sim.digest(), no_command.digest());
}

#[test]
fn profile_defaults_count_the_active_head_and_completion_reopens_one_slot() {
    for (profile, cap) in [
        (Profile::Sta1Classic, 5),
        (Profile::Native, 5),
        (Profile::Sta2Classic, 10),
        (Profile::Fo4, 10),
    ] {
        let mut sim = game(profile);
        let yard = spawn(&mut sim, "limit_default");
        fund(&mut sim, 20);
        for n in 0..cap {
            assert!(build(&mut sim, yard), "{profile:?} admission {n}");
        }
        assert_eq!(sim.production.queue(yard).len(), cap as usize);
        let rec = sim.production.to_rec();
        let head = &rec.queues.iter().find(|q| q.0 == yard.0).unwrap().1[0];
        assert!(head.3, "the active head still occupies a slot");
        rejected_without_charge_or_queue_change(&mut sim, yard);
        for _ in 0..50 {
            sim.step(&[]);
        }
        assert_eq!(sim.production.queue(yard).len(), cap as usize - 1);
        assert!(build(&mut sim, yard), "completion opens exactly one slot");
        assert_eq!(sim.production.queue(yard).len(), cap as usize);
    }
}

#[test]
fn builder_overrides_zero_one_and_twelve_ignore_product_cap_and_reject_before_payment() {
    let mut sim = game(Profile::Native);
    let zero = spawn(&mut sim, "limit_zero");
    let one = spawn(&mut sim, "limit_one");
    let twelve = spawn(&mut sim, "limit_twelve");
    fund(&mut sim, 20);
    rejected_without_charge_or_queue_change(&mut sim, zero);
    assert_eq!(stock(&sim), Fixed::from_int(20));
    assert!(
        build(&mut sim, one),
        "product's zero cap cannot block its builder"
    );
    assert_eq!(stock(&sim), Fixed::from_int(19));
    rejected_without_charge_or_queue_change(&mut sim, one);
    for n in 0..12 {
        assert!(build(&mut sim, twelve), "admission {n} above vanilla ten");
    }
    assert_eq!(sim.production.queue(twelve).len(), 12);
    assert_eq!(stock(&sim), Fixed::from_int(7));
    rejected_without_charge_or_queue_change(&mut sim, twelve);
    assert_eq!(stock(&sim), Fixed::from_int(7));
}

#[test]
fn long_queue_snapshot_continues_and_rejects_changed_capacity_content() {
    let mut live = game(Profile::Fo4);
    let yard = spawn(&mut live, "limit_default");
    fund(&mut live, 20);
    for _ in 0..7 {
        assert!(build(&mut live, yard));
    }
    assert_eq!(live.production.queue(yard).len(), 7);
    let bytes = live.snapshot();
    let original_content = Arc::clone(live.content().unwrap());
    let mut loaded = Sim::from_snapshot(&bytes, Some(original_content.clone())).unwrap();
    assert_eq!(loaded.digest(), live.digest());
    assert_eq!(loaded.production.to_rec(), live.production.to_rec());

    let mut altered = ContentDb::builder()
        .profile(original_content.profile())
        .features(original_content.features().clone());
    for unit in original_content.units() {
        let mut unit = unit.clone();
        if unit.odf == "limit_default" {
            unit.build.queue_limit = Some(9);
        }
        altered = altered.unit(unit);
    }
    let altered = Arc::new(altered.build().unwrap());
    assert_ne!(altered.hash(), original_content.hash());
    assert!(matches!(
        Sim::from_snapshot(&bytes, Some(altered)),
        Err(crate::snapshot::SnapshotError::ContentMismatch { .. })
    ));

    for _ in 0..3 {
        assert!(build(&mut live, yard));
        assert!(build(&mut loaded, yard));
        assert_eq!(loaded.digest(), live.digest());
    }
    rejected_without_charge_or_queue_change(&mut live, yard);
    rejected_without_charge_or_queue_change(&mut loaded, yard);
    assert_eq!(loaded.digest(), live.digest());
    for tick in 0..50 {
        let a = live.step(&[]);
        let b = loaded.step(&[]);
        assert_eq!(a.events, b.events, "tick {tick}");
        assert_eq!(a.view.digest, b.view.digest, "tick {tick}");
    }
    assert_eq!(live.production.queue(yard).len(), 9);
    assert!(build(&mut live, yard));
    assert!(build(&mut loaded, yard));
    assert_eq!(loaded.production.to_rec(), live.production.to_rec());
    assert_eq!(loaded.digest(), live.digest());
}
