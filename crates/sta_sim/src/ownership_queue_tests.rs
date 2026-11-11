use crate::command::{Command, CommandKind, Edit, Order};
use crate::production::{Production, ProductionRec};
use crate::replay::{Recorder, Replay};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::world::NEUTRAL;
use crate::{ContentKey, EntityId, Fixed, PlayerId, Sim, SimEvent, TeamId, Vec3, WorldSource};
use sta_content::{ContentDb, DefRef, MapDef, MapObject, UnitDef};
use std::sync::Arc;

const OLD: PlayerId = PlayerId(0);
const NEW: PlayerId = PlayerId(1);

fn content(prepaid: bool) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut features = fake.features().clone();
    features.pay_on_queue = prepaid;
    let mut yard = UnitDef::new("transfer_yard", "shipyard");
    yard.race = "fx".into();
    yard.hull = Fixed::from_int(1000);
    yard.build.list = [
        "transfer_four",
        "transfer_fifth",
        "transfer_huge",
        "transfer_nonpositive",
    ]
    .map(DefRef::new)
    .to_vec();
    let product = |name: &str, cost: Vec<Fixed>| {
        let mut u = UnitDef::new(name, "craft");
        u.race = "fx".into();
        u.hull = Fixed::from_int(100);
        u.build.cost = cost;
        u.build.ticks = 50;
        u
    };
    let four = product(
        "transfer_four",
        [10, 20, 30, 40].map(Fixed::from_int).to_vec(),
    );
    let fifth = product(
        "transfer_fifth",
        [0, 0, 0, 0, 1].map(Fixed::from_int).to_vec(),
    );
    let huge = product("transfer_huge", vec![Fixed::from_bits(i64::MAX - 10)]);
    let nonpositive = product(
        "transfer_nonpositive",
        vec![
            Fixed::from_int(-5),
            Fixed::ZERO,
            Fixed::ZERO,
            Fixed::ZERO,
            Fixed::ZERO,
            Fixed::from_int(-7),
        ],
    );
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
            .unit(yard)
            .unit(four)
            .unit(fifth)
            .unit(huge)
            .unit(nonpositive)
            .build()
            .unwrap(),
    )
}

fn setup(prepaid: bool, map: MapDef) -> GameSetup {
    GameSetup {
        seed: 19,
        map: Arc::new(map),
        content: content(prepaid),
        players: [OLD, NEW]
            .map(|id| PlayerSetup {
                id,
                team: TeamId(id.0),
                race: "fx".into(),
                control: Control::Human,
            })
            .to_vec(),
        options: GameOptions::default(),
    }
}

fn game(prepaid: bool) -> Sim {
    Sim::from_setup(&setup(prepaid, MapDef::empty()))
}

fn command(from: PlayerId, seq: u32, kind: CommandKind) -> Command {
    Command {
        player: from,
        seq,
        kind,
    }
}

fn build(from: PlayerId, seq: u32, yard: EntityId, what: ContentKey) -> Command {
    command(
        from,
        seq,
        CommandKind::Order(Order::Build {
            builder: yard,
            what,
            at: None,
        }),
    )
}

fn transfer(seq: u32, yard: EntityId, to: PlayerId) -> Command {
    command(
        PlayerId::SCRIPT,
        seq,
        CommandKind::Edit(Edit::Team(yard, to)),
    )
}

fn fund(sim: &mut Sim, player: PlayerId, amounts: [Fixed; 4]) {
    for (res, amount) in amounts.into_iter().enumerate() {
        let events = sim
            .step(&[command(
                PlayerId::BENCH,
                res as u32,
                CommandKind::Edit(Edit::Cargo(player, res as u8, amount)),
            )])
            .events;
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, SimEvent::Rejected { .. })),
            "{events:?}"
        );
    }
}

fn stock(sim: &Sim, player: PlayerId) -> Vec<Fixed> {
    sim.economy.stock_of(player)
}

fn spawn_yard(sim: &mut Sim, owner: PlayerId) -> EntityId {
    let key = sim.content().unwrap().key("transfer_yard").unwrap();
    sim.spawn_def(key, owner, Vec3::ZERO, (Fixed::ZERO, Fixed::ONE))
        .unwrap()
}

fn key(sim: &Sim, name: &str) -> ContentKey {
    sim.content().unwrap().key(name).unwrap()
}

#[test]
fn prepaid_three_jobs_refund_four_costs_before_production_runs_and_new_owner_controls() {
    let mut sim = game(true);
    let yard = spawn_yard(&mut sim, OLD);
    let original = [100, 200, 300, 400].map(Fixed::from_int);
    fund(&mut sim, OLD, original);
    let product = key(&sim, "transfer_four");
    let batch = [
        build(OLD, 0, yard, product),
        build(OLD, 1, yard, product),
        build(OLD, 2, yard, product),
        transfer(0, yard, NEW),
    ];
    let events = sim.step(&batch).events;
    assert!(
        !events.iter().any(|e| matches!(
            e,
            SimEvent::Rejected { .. } | SimEvent::BuildStarted { .. } | SimEvent::Built { .. }
        )),
        "{events:?}"
    );
    assert!(sim.production.queue(yard).is_empty());
    assert_eq!(stock(&sim, OLD), original);
    assert!(stock(&sim, NEW).is_empty());
    assert_eq!(sim.unit(yard).unwrap().owner, NEW);
    for _ in 0..100 {
        let events = sim.step(&[]).events;
        assert!(!events.iter().any(|e| matches!(e, SimEvent::Built { .. })));
    }
    assert_eq!(sim.units.len(), 1, "old queue never creates a child");
    let out = sim.step(&[build(OLD, 0, yard, product)]);
    assert!(
        out.events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
    fund(&mut sim, NEW, original);
    let out = sim.step(&[build(NEW, 0, yard, product)]);
    assert!(
        !out.events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
    assert_eq!(sim.production.queue(yard).len(), 1);
}

#[test]
fn deferred_unfunded_jobs_clear_without_credit_after_waiting() {
    let mut sim = game(false);
    let yard = spawn_yard(&mut sim, OLD);
    let product = key(&sim, "transfer_four");
    for _ in 0..3 {
        let out = sim.step(&[build(OLD, 0, yard, product)]);
        assert!(
            !out.events
                .iter()
                .any(|e| matches!(e, SimEvent::Rejected { .. }))
        );
    }
    for _ in 0..10 {
        sim.step(&[]);
    }
    let rec = sim.production.to_rec();
    assert_eq!(rec.queues[0].1.len(), 3);
    assert!(
        rec.queues[0]
            .1
            .iter()
            .all(|item| item.2 == 0 && !item.3 && !item.4)
    );
    let events = sim.step(&[transfer(0, yard, NEW)]).events;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
    assert!(sim.production.queue(yard).is_empty());
    assert!(stock(&sim, OLD).is_empty());
    assert!(stock(&sim, NEW).is_empty());
}

#[test]
fn same_owner_and_repeated_transfers_refund_once() {
    let mut sim = game(true);
    let yard = spawn_yard(&mut sim, OLD);
    let original = [100, 200, 300, 400].map(Fixed::from_int);
    fund(&mut sim, OLD, original);
    let product = key(&sim, "transfer_four");
    let events = sim
        .step(&[
            build(OLD, 0, yard, product),
            build(OLD, 1, yard, product),
            transfer(0, yard, OLD),
            transfer(1, yard, NEW),
            transfer(2, yard, OLD),
        ])
        .events;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. })),
        "{events:?}"
    );
    assert_eq!(stock(&sim, OLD), original);
    assert!(stock(&sim, NEW).is_empty());
    assert!(sim.production.queue(yard).is_empty());
    assert_eq!(sim.unit(yard).unwrap().owner, OLD);
}

fn inject(sim: &mut Sim, yard: EntityId, items: Vec<(u32, u8, u64, bool, bool)>) {
    let mut rec: ProductionRec = sim.production.to_rec();
    rec.queues.push((yard.0, items));
    sim.production = Production::from_rec(&rec).unwrap();
}

fn atomically_rejected(mut sim: Sim, yard: EntityId, to: PlayerId) {
    let before = sim.digest();
    let owner = sim.unit(yard).unwrap().owner;
    let queue = sim.production.to_rec();
    let old_stock = stock(&sim, owner);
    assert!(!sim.apply_edit(PlayerId::SCRIPT, &Edit::Team(yard, to)));
    assert_eq!(sim.digest(), before);
    assert_eq!(sim.unit(yard).unwrap().owner, owner);
    assert_eq!(sim.production.to_rec(), queue);
    assert_eq!(stock(&sim, owner), old_stock);
    let mut baseline = sim.clone();
    let events = sim.step(&[transfer(0, yard, to)]).events;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
    baseline.step(&[]);
    assert_eq!(sim.digest(), baseline.digest());
    assert_eq!(sim.production.to_rec(), baseline.production.to_rec());
}

#[test]
fn malformed_queues_unpaid_active_heads_and_overflow_reject_atomically() {
    let base = || {
        let mut sim = game(true);
        let yard = spawn_yard(&mut sim, OLD);
        (sim, yard)
    };
    let (mut sim, yard) = base();
    let four = key(&sim, "transfer_four").0;
    inject(&mut sim, yard, vec![(four, NEW.0, 0, false, true)]);
    atomically_rejected(sim, yard, NEW);

    let (mut sim, yard) = base();
    inject(&mut sim, yard, vec![(four, OLD.0, 1, false, true)]);
    atomically_rejected(sim, yard, NEW);

    let (mut sim, yard) = base();
    inject(&mut sim, yard, vec![(u32::MAX, OLD.0, 0, false, true)]);
    atomically_rejected(sim, yard, NEW);

    let (mut sim, yard) = base();
    inject(&mut sim, yard, vec![(four, OLD.0, 0, true, false)]);
    atomically_rejected(sim, yard, NEW);

    let (mut sim, yard) = base();
    let fifth = key(&sim, "transfer_fifth").0;
    inject(&mut sim, yard, vec![(fifth, OLD.0, 0, false, true)]);
    atomically_rejected(sim, yard, NEW);

    let (mut sim, yard) = base();
    let huge = key(&sim, "transfer_huge").0;
    sim.step(&[command(
        PlayerId::BENCH,
        0,
        CommandKind::Edit(Edit::Cargo(OLD, 0, Fixed::from_bits(100))),
    )]);
    inject(&mut sim, yard, vec![(huge, OLD.0, 0, false, true)]);
    atomically_rejected(sim, yard, NEW);
}

#[test]
fn neutral_paid_credit_rejects_but_unpaid_queue_can_clear() {
    let mut paid = game(true);
    let yard = spawn_yard(&mut paid, NEUTRAL);
    let product = key(&paid, "transfer_four").0;
    inject(&mut paid, yard, vec![(product, NEUTRAL.0, 0, false, true)]);
    atomically_rejected(paid, yard, NEW);

    let mut unpaid = game(false);
    let yard = spawn_yard(&mut unpaid, NEUTRAL);
    let product = key(&unpaid, "transfer_four").0;
    inject(
        &mut unpaid,
        yard,
        vec![(product, NEUTRAL.0, 0, false, false)],
    );
    let events = unpaid.step(&[transfer(0, yard, NEW)]).events;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. })),
        "{events:?}"
    );
    assert_eq!(unpaid.unit(yard).unwrap().owner, NEW);
    assert!(unpaid.production.queue(yard).is_empty());
    assert!(stock(&unpaid, NEW).is_empty());
}

#[test]
fn paid_record_refunds_even_under_deferred_builder_mode_and_negative_balance() {
    let mut sim = game(false);
    let yard = spawn_yard(&mut sim, OLD);
    sim.step(&[command(
        PlayerId::BENCH,
        0,
        CommandKind::Edit(Edit::Cargo(OLD, 0, Fixed::from_int(-5))),
    )]);
    let product = key(&sim, "transfer_four").0;
    inject(&mut sim, yard, vec![(product, OLD.0, 0, false, true)]);
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::Team(yard, NEW)));
    assert_eq!(
        stock(&sim, OLD),
        [5, 20, 30, 40].map(Fixed::from_int).to_vec()
    );
    assert!(stock(&sim, NEW).is_empty());
    assert!(sim.production.queue(yard).is_empty());

    let mut unpaid = game(true);
    let yard = spawn_yard(&mut unpaid, OLD);
    let product = key(&unpaid, "transfer_four").0;
    inject(&mut unpaid, yard, vec![(product, OLD.0, 0, false, false)]);
    assert!(unpaid.apply_edit(PlayerId::SCRIPT, &Edit::Team(yard, NEW)));
    assert!(
        stock(&unpaid, OLD).is_empty(),
        "unpaid item earns no refund under prepaid builder mode"
    );
    assert!(unpaid.production.queue(yard).is_empty());
}

#[test]
fn neutral_paid_nonpositive_costs_clear_without_creating_a_bank_row() {
    let mut sim = game(true);
    let yard = spawn_yard(&mut sim, NEUTRAL);
    let product = key(&sim, "transfer_nonpositive").0;
    inject(&mut sim, yard, vec![(product, NEUTRAL.0, 0, false, true)]);
    let events = sim.step(&[transfer(0, yard, NEW)]).events;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. })),
        "{events:?}"
    );
    assert_eq!(sim.unit(yard).unwrap().owner, NEW);
    assert!(sim.production.queue(yard).is_empty());
    assert!(
        sim.economy.to_rec().stock.is_empty(),
        "zero and negative costs, including beyond slot four, do not create stock"
    );
}

#[test]
fn unstarted_queue_snapshot_and_same_build_replay_continue_deterministically() {
    let mut live = game(false);
    let yard = spawn_yard(&mut live, OLD);
    let product = key(&live, "transfer_four");
    live.step(&[build(OLD, 0, yard, product), build(OLD, 1, yard, product)]);
    let bytes = live.snapshot();
    let mut loaded = Sim::from_snapshot(&bytes, Some(Arc::clone(live.content().unwrap()))).unwrap();
    assert_eq!(loaded.digest(), live.digest());
    for commands in [vec![transfer(0, yard, NEW)], vec![]] {
        let a = live.step(&commands);
        let b = loaded.step(&commands);
        assert_eq!(a.events, b.events);
        assert_eq!(a.view.digest, b.view.digest);
    }

    let mut map = MapDef::empty();
    map.objects.push(MapObject {
        odf: "transfer_yard".into(),
        owner: Some(OLD.0),
        pos: Vec3::ZERO,
        heading: (Fixed::ZERO, Fixed::ONE),
        label: None,
    });
    let setup = setup(true, map);
    let content = Arc::clone(&setup.content);
    let mut recorder = Recorder::with_interval(&setup, 1);
    let yard = recorder.sim().units.iter().next().unwrap().id;
    let product = key(recorder.sim(), "transfer_four");
    for (res, amount) in [100, 200, 300, 400].into_iter().enumerate() {
        recorder.step(&[command(
            PlayerId::BENCH,
            res as u32,
            CommandKind::Edit(Edit::Cargo(OLD, res as u8, Fixed::from_int(amount))),
        )]);
    }
    recorder.step(&[
        build(OLD, 0, yard, product),
        build(OLD, 1, yard, product),
        build(OLD, 2, yard, product),
        transfer(0, yard, NEW),
    ]);
    for _ in 0..55 {
        recorder.step(&[]);
    }
    let digest = recorder.sim().digest();
    let replay = Replay::decode(&recorder.finish().encode()).unwrap();
    let played = replay.play(content).unwrap();
    assert_eq!(played.digest(), digest);
    assert!(played.production.queue(yard).is_empty());
    assert_eq!(played.unit(yard).unwrap().owner, NEW);
}

#[path = "ownership_active_tests.rs"]
mod active_tests;
