//! Completion regressions for the SIM-19 transfer contract.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, PlanetRole, UnitDef};

use crate::command::{Command, CommandKind, Edit, Order};
use crate::production::Production;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::view::UnitFlags;
use crate::world::{EntityKind, NEUTRAL};
use crate::{EntityId, Fixed, MoveStats, PlayerId, Sim, SimEvent, TeamId, Vec3, WorldSource};

fn pos(x: i32) -> Vec3 {
    Vec3::from_f32([x as f32, 0.0, 0.0])
}

fn content(pay_on_queue: bool) -> Arc<ContentDb> {
    let base = sta_content::fake::db();
    let mut features = base.features().clone();
    features.pay_on_queue = pay_on_queue;
    let mut rig = UnitDef::new("completion_rig", "constructionrig");
    rig.race = "fx".into();
    rig.movement = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
    rig.hull = Fixed::from_int(100);
    let mut background = UnitDef::new("completion_background", "planet_obj");
    background.planet_role = Some(PlanetRole::Background);
    background.hull = Fixed::ZERO;
    background.resource = Some(0);
    let mut producer = UnitDef::new("completion_producer", "planet");
    producer.planet_role = Some(PlanetRole::Producer);
    producer.hull = Fixed::from_int(100);
    producer.resource = Some(0);
    producer.planet_crew_gain = 37;
    producer.build.list = vec![sta_content::DefRef::new("fx_frigate")];
    let mut negative = UnitDef::new("completion_negative", "planet");
    negative.planet_role = Some(PlanetRole::Producer);
    negative.hull = Fixed::from_int(100);
    negative.planet_crew_gain = -9;
    let mut unsupported_cost = UnitDef::new("completion_unsupported_cost", "starbase");
    unsupported_cost.hull = Fixed::from_int(100);
    unsupported_cost.build.cost = vec![Fixed::ZERO, Fixed::ZERO, Fixed::ZERO, Fixed::ZERO, Fixed::ONE];
    let mut builder = ContentDb::builder()
        .profile(base.profile())
        .features(features)
        .unit(rig)
        .unit(background)
        .unit(producer)
        .unit(negative)
        .unit(unsupported_cost);
    for unit in base.units() {
        builder = builder.unit(unit.clone());
    }
    for weapon in base.weapons() {
        builder = builder.weapon(weapon.clone());
    }
    Arc::new(builder.build().unwrap())
}

fn game(pay_on_queue: bool) -> Sim {
    Sim::from_setup(&GameSetup {
        seed: 19,
        map: Arc::new(MapDef::empty()),
        content: content(pay_on_queue),
        players: [0, 1, 2]
            .map(|id| PlayerSetup {
                id: PlayerId(id),
                team: TeamId(id),
                race: "fx".into(),
                control: Control::Human,
            })
            .to_vec(),
        options: GameOptions::default(),
    })
}

fn spawn(sim: &mut Sim, odf: &str, owner: PlayerId, x: i32) -> EntityId {
    let key = sim.content().unwrap().key(odf).unwrap();
    sim.spawn_def(key, owner, pos(x), (Fixed::ZERO, Fixed::ONE)).unwrap()
}

fn team(seq: u32, id: EntityId, owner: PlayerId) -> Command {
    Command {
        player: PlayerId::SCRIPT,
        seq,
        kind: CommandKind::Edit(Edit::Team(id, owner)),
    }
}

fn step_accepted(sim: &mut Sim, commands: &[Command]) -> Vec<SimEvent> {
    let events = sim.step(commands).events;
    assert!(
        !events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })),
        "{events:?}"
    );
    events
}

fn fund(sim: &mut Sim, amount: i32) {
    step_accepted(
        sim,
        &[Command {
            player: PlayerId::BENCH,
            seq: 0,
            kind: CommandKind::Edit(Edit::Cargo(PlayerId(0), 0, Fixed::from_int(amount))),
        }],
    );
}

fn build(sim: &mut Sim, builder: EntityId, odf: &str, at: Option<Vec3>) {
    let what = sim.content().unwrap().key(odf).unwrap();
    step_accepted(
        sim,
        &[Command {
            player: PlayerId(0),
            seq: 0,
            kind: CommandKind::Order(Order::Build { builder, what, at }),
        }],
    );
}

#[test]
fn capture_pending_deploy_refunds_original_payer_and_never_deploys_later() {
    for pay_on_queue in [false, true] {
        let mut sim = game(pay_on_queue);
        let rig = spawn(&mut sim, "completion_rig", PlayerId(0), 0);
        fund(&mut sim, 2000);
        build(&mut sim, rig, "fx_station", Some(pos(400)));
        assert!(sim.production.is_deploying(rig));
        step_accepted(&mut sim, &[team(0, rig, PlayerId(1))]);
        assert!(!sim.production.is_deploying(rig));
        assert_eq!(sim.economy.stock_of(PlayerId(0))[0], Fixed::from_int(2000));
        assert!(sim.economy.stock_of(PlayerId(1)).is_empty());
        for _ in 0..20 {
            sim.step(&[]);
        }
        assert!(sim.production.to_rec().welds.is_empty());
    }
}

#[test]
fn capture_active_weld_removes_child_and_accounts_for_payment_mode() {
    for pay_on_queue in [false, true] {
        let mut sim = game(pay_on_queue);
        let rig = spawn(&mut sim, "completion_rig", PlayerId(0), 0);
        fund(&mut sim, 2000);
        build(&mut sim, rig, "fx_station", Some(pos(10)));
        let weld = sim.production.to_rec().welds[0];
        let child = EntityId(weld.0);
        assert!(sim.unit(child).unwrap().progress.is_some());
        let before = sim.snapshot();
        assert!(!sim.apply_edit(PlayerId::SCRIPT, &Edit::Team(child, PlayerId(1))));
        assert_eq!(sim.snapshot(), before);
        step_accepted(&mut sim, &[team(0, rig, PlayerId(1))]);
        assert!(sim.unit(child).is_none());
        assert!(sim.production.to_rec().welds.is_empty());
        assert_eq!(sim.unit(rig).unwrap().owner, PlayerId(1));
        assert_eq!(
            sim.economy.stock_of(PlayerId(0))[0],
            Fixed::from_int(if pay_on_queue { 2000 } else { 800 })
        );
        for _ in 0..20 {
            sim.step(&[]);
        }
        assert!(sim.unit(child).is_none());
    }
}

#[test]
fn malformed_weld_state_rejects_atomically() {
    for fault in 0..5 {
        let mut sim = game(true);
        let rig = spawn(&mut sim, "completion_rig", PlayerId(0), 0);
        fund(&mut sim, 2000);
        build(&mut sim, rig, "fx_station", Some(pos(10)));
        let mut rec = sim.production.to_rec();
        let child = EntityId(rec.welds[0].0);
        match fault {
            0 => sim.units.get_mut(child).unwrap().owner = PlayerId(1),
            1 => rec.welds[0].3 = 0,
            2 => rec.welds[0].2 = rec.welds[0].3,
            3 => sim.units.get_mut(child).unwrap().progress = None,
            _ => sim.units.get_mut(child).unwrap().progress = Some(Fixed::ONE / Fixed::from_int(2)),
        }
        sim.production = Production::from_rec(&rec).unwrap();
        let before = sim.snapshot();
        assert!(
            !sim.apply_edit(PlayerId::SCRIPT, &Edit::Team(rig, PlayerId(1))),
            "fault {fault}"
        );
        assert_eq!(sim.snapshot(), before, "fault {fault}");
    }
}

#[test]
fn child_owned_production_and_weld_refund_overflow_reject_atomically() {
    for fault in 0..3 {
        let mut sim = game(true);
        let rig = spawn(&mut sim, "completion_rig", PlayerId(0), 0);
        fund(&mut sim, 2000);
        build(&mut sim, rig, "fx_station", Some(pos(10)));
        let mut rec = sim.production.to_rec();
        let child = EntityId(rec.welds[0].0);
        match fault {
            0 => rec.queues.push((
                child.0,
                vec![(sim.content().unwrap().key("fx_frigate").unwrap().0, 0, 0, false, false)],
            )),
            1 => sim.economy.set_stock(
                PlayerId(0),
                [Fixed::from_bits(i64::MAX), Fixed::ZERO, Fixed::ZERO, Fixed::ZERO],
            ),
            _ => {
                let key = sim.content().unwrap().key("completion_unsupported_cost").unwrap();
                sim.units.get_mut(child).unwrap().def = key;
                rec.welds[0].3 = 1;
            }
        }
        sim.production = Production::from_rec(&rec).unwrap();
        let before = sim.snapshot();
        assert!(
            !sim.apply_edit(PlayerId::SCRIPT, &Edit::Team(rig, PlayerId(1))),
            "fault {fault}"
        );
        assert_eq!(sim.snapshot(), before, "fault {fault}");
    }
}

#[test]
fn invalid_deploy_receipt_and_refund_overflow_reject_atomically() {
    for fault in 0..3 {
        let mut sim = game(true);
        let rig = spawn(&mut sim, "completion_rig", PlayerId(0), 0);
        fund(&mut sim, 2000);
        build(&mut sim, rig, "fx_station", Some(pos(400)));
        if fault == 0 {
            let mut rec = sim.production.to_rec();
            rec.deploys[0].2 = 1;
            sim.production = Production::from_rec(&rec).unwrap();
        } else if fault == 1 {
            sim.economy.set_stock(
                PlayerId(0),
                [Fixed::from_bits(i64::MAX), Fixed::ZERO, Fixed::ZERO, Fixed::ZERO],
            );
        } else {
            let mut rec = sim.production.to_rec();
            rec.deploys[0].1 = sim.content().unwrap().key("completion_unsupported_cost").unwrap().0;
            sim.production = Production::from_rec(&rec).unwrap();
        }
        let before = sim.snapshot();
        assert!(
            !sim.apply_edit(PlayerId::SCRIPT, &Edit::Team(rig, PlayerId(1))),
            "fault {fault}"
        );
        assert_eq!(sim.snapshot(), before, "fault {fault}");
    }
}

#[test]
fn typed_planets_retain_owner_and_contribute_derived_capacity() {
    let mut sim = game(true);
    let background = spawn(&mut sim, "completion_background", PlayerId(0), 0);
    let producer = spawn(&mut sim, "completion_producer", PlayerId(0), 1000);
    let _negative = spawn(&mut sim, "completion_negative", PlayerId(0), 2000);
    let generic = spawn(&mut sim, "fx_moon", PlayerId(0), 3000);
    assert_eq!(sim.unit(background).unwrap().kind, EntityKind::Resource);
    assert_eq!(sim.unit(background).unwrap().owner, PlayerId(0));
    assert_eq!(sim.unit(background).unwrap().hull[0], Fixed::ZERO);
    assert_eq!(sim.unit(producer).unwrap().kind, EntityKind::Resource);
    assert_eq!(sim.unit(producer).unwrap().owner, PlayerId(0));
    assert_eq!(sim.unit(generic).unwrap().owner, NEUTRAL);
    assert_eq!(sim.planet_crew_capacity(PlayerId(0)), 28);
    assert_eq!(sim.planet_crew_capacity(PlayerId(1)), 0);
    assert!(!sim.apply_edit(PlayerId::SCRIPT, &Edit::Team(generic, PlayerId(1))));
    step_accepted(
        &mut sim,
        &[team(0, background, PlayerId(1)), team(1, producer, PlayerId(1))],
    );
    assert_eq!(sim.planet_crew_capacity(PlayerId(0)), -9);
    assert_eq!(sim.planet_crew_capacity(PlayerId(1)), 37);
    let bytes = sim.snapshot();
    let restored = Sim::from_snapshot(&bytes, sim.content().cloned()).unwrap();
    assert_eq!(restored.planet_crew_capacity(PlayerId(0)), -9);
    assert_eq!(restored.planet_crew_capacity(PlayerId(1)), 37);
    step_accepted(&mut sim, &[team(0, producer, PlayerId(0))]);
    assert_eq!(sim.planet_crew_capacity(PlayerId(0)), 28);
    assert_eq!(sim.planet_crew_capacity(PlayerId(1)), 0);
}

#[test]
fn invalid_planet_hull_or_dying_flag_rejects_without_mutation() {
    for fault in 0..3 {
        let mut sim = game(true);
        let id = match fault {
            0 | 1 => spawn(&mut sim, "completion_producer", PlayerId(0), 0),
            _ => spawn(&mut sim, "completion_background", PlayerId(0), 0),
        };
        let unit = sim.units.get_mut(id).unwrap();
        match fault {
            0 => unit.hull[0] = Fixed::ZERO,
            1 => unit.flags = unit.flags.with(UnitFlags::DYING, true),
            _ => unit.hull[0] = -Fixed::ONE,
        }
        let before = sim.snapshot();
        assert!(
            !sim.apply_edit(PlayerId::SCRIPT, &Edit::Team(id, PlayerId(1))),
            "fault {fault}"
        );
        assert_eq!(sim.snapshot(), before, "fault {fault}");
    }
}

#[test]
fn background_refuses_mining_and_producer_runs_a_capture_cancelled_queue() {
    for pay_on_queue in [false, true] {
        let mut sim = game(pay_on_queue);
        let background = spawn(&mut sim, "completion_background", PlayerId(0), 0);
        let producer = spawn(&mut sim, "completion_producer", PlayerId(0), 1000);
        let miner = spawn(&mut sim, "fx_miner", PlayerId(0), 0);
        let mine = Command {
            player: PlayerId(0),
            seq: 0,
            kind: CommandKind::Order(Order::Mine {
                units: vec![miner],
                node: background,
            }),
        };
        assert!(sim
            .step(&[mine])
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. })));
        assert!(!sim.economy.is_mining(miner));
        step_accepted(
            &mut sim,
            &[Command {
                player: PlayerId(0),
                seq: 0,
                kind: CommandKind::Order(Order::Mine {
                    units: vec![miner],
                    node: producer,
                }),
            }],
        );
        assert!(sim.economy.is_mining(miner));
        fund(&mut sim, 2000);
        build(&mut sim, producer, "fx_frigate", None);
        assert_eq!(sim.production.queue(producer).len(), 1);
        let expected = if pay_on_queue { 2000 } else { 1750 };
        step_accepted(&mut sim, &[team(0, producer, PlayerId(1))]);
        assert!(sim.production.queue(producer).is_empty());
        assert_eq!(sim.economy.stock_of(PlayerId(0))[0], Fixed::from_int(expected));
        for _ in 0..500 {
            sim.step(&[]);
        }
        assert_eq!(
            sim.units
                .iter()
                .filter(|u| u.def == sim.content().unwrap().key("fx_frigate").unwrap())
                .count(),
            0
        );
    }
}

#[test]
fn mid_weld_snapshot_continuation_and_post_capture_snapshot_are_deterministic() {
    let mut sim = game(true);
    let rig = spawn(&mut sim, "completion_rig", PlayerId(0), 0);
    spawn(&mut sim, "completion_producer", PlayerId(0), 2000);
    fund(&mut sim, 2000);
    build(&mut sim, rig, "fx_station", Some(pos(10)));
    let mid = sim.snapshot();
    let mut restored = Sim::from_snapshot(&mid, sim.content().cloned()).unwrap();
    assert_eq!(restored.snapshot(), mid);
    let commands = [team(0, rig, PlayerId(1))];
    let a = sim.step(&commands);
    let b = restored.step(&commands);
    assert_eq!(a.events, b.events);
    assert_eq!(a.view.digest, b.view.digest);
    assert_eq!(
        sim.planet_crew_capacity(PlayerId(0)),
        restored.planet_crew_capacity(PlayerId(0))
    );
    let captured = sim.snapshot();
    let mut reloaded = Sim::from_snapshot(&captured, sim.content().cloned()).unwrap();
    assert_eq!(reloaded.snapshot(), captured);
    for _ in 0..30 {
        let a = sim.step(&[]);
        let b = reloaded.step(&[]);
        assert_eq!(a.events, b.events);
        assert_eq!(a.view.digest, b.view.digest);
        assert_eq!(
            sim.planet_crew_capacity(PlayerId(0)),
            reloaded.planet_crew_capacity(PlayerId(0))
        );
    }
}

#[test]
fn planet_only_seat_does_not_prevent_capture_elimination() {
    let mut sim = game(true);
    let craft = spawn(&mut sim, "fx_frigate", PlayerId(0), 0);
    spawn(&mut sim, "completion_producer", PlayerId(0), 2000);
    spawn(&mut sim, "fx_frigate", PlayerId(1), 5000);
    let events = step_accepted(&mut sim, &[team(0, craft, PlayerId(1))]);
    assert!(events.contains(&SimEvent::PlayerEliminated { player: PlayerId(0) }));
    assert!(sim.is_eliminated(PlayerId(0)));
    assert_eq!(sim.planet_crew_capacity(PlayerId(0)), 37);
}

#[test]
fn capture_loss_is_checked_after_the_sorted_batch() {
    let mut regained = game(true);
    let a = spawn(&mut regained, "fx_frigate", PlayerId(0), 0);
    let b = spawn(&mut regained, "fx_frigate", PlayerId(1), 3000);
    step_accepted(&mut regained, &[team(0, a, PlayerId(1)), team(1, b, PlayerId(0))]);
    assert!(!regained.is_eliminated(PlayerId(0)));
    assert!(!regained.is_eliminated(PlayerId(1)));

    let mut loss = game(true);
    let only = spawn(&mut loss, "fx_frigate", PlayerId(0), 0);
    spawn(&mut loss, "fx_frigate", PlayerId(1), 3000);
    let events = step_accepted(&mut loss, &[team(0, only, PlayerId(1))]);
    assert!(events.contains(&SimEvent::PlayerEliminated { player: PlayerId(0) }));
    assert!(loss.is_eliminated(PlayerId(0)));

    let mut no_loss = game(true);
    let owned = spawn(&mut no_loss, "fx_frigate", PlayerId(0), 0);
    let planet = spawn(&mut no_loss, "completion_producer", PlayerId(0), 1000);
    step_accepted(
        &mut no_loss,
        &[team(0, owned, PlayerId(0)), team(1, planet, PlayerId(1))],
    );
    assert!(!no_loss.is_eliminated(PlayerId(0)));
    assert!(!no_loss.apply_edit(PlayerId::SCRIPT, &Edit::Team(EntityId(u32::MAX), PlayerId(1))));
    assert!(!no_loss.is_eliminated(PlayerId(0)));
}

#[test]
fn seatless_synthetic_repeated_transfers_remain_legal() {
    let mut sim = Sim::new(19);
    let id = sim.spawn(0, pos(0), (Fixed::ZERO, Fixed::ONE), MoveStats::STATIC);
    sim.units.get_mut(id).unwrap().hull = [Fixed::ONE; 2];
    for (seq, owner) in [PlayerId(7), NEUTRAL, PlayerId(0), PlayerId(7)].into_iter().enumerate() {
        step_accepted(&mut sim, &[team(seq as u32, id, owner)]);
        assert_eq!(sim.unit(id).unwrap().owner, owner);
    }
}

#[test]
fn rig_and_planet_capture_replay_same_build_with_identical_events_and_digests() {
    let content = content(true);
    let setup = GameSetup {
        seed: 19,
        map: Arc::new(MapDef::empty()),
        content: content.clone(),
        players: [0, 1]
            .map(|id| PlayerSetup {
                id: PlayerId(id),
                team: TeamId(id),
                race: "fx".into(),
                control: Control::Human,
            })
            .to_vec(),
        options: GameOptions::default(),
    };
    let spawn_edit = |seq, odf| Command {
        player: PlayerId::SCRIPT,
        seq,
        kind: CommandKind::Edit(Edit::Spawn {
            odf: content.key(odf).unwrap(),
            player: PlayerId(0),
            pos: pos(if seq == 0 { 0 } else { 2000 }),
            heading: (Fixed::ZERO, Fixed::ONE),
        }),
    };
    let mut recorder = crate::replay::Recorder::with_interval(&setup, 1);
    let mut trace = Vec::new();
    for tick in 0..20 {
        let commands = match tick {
            0 => vec![spawn_edit(0, "completion_rig")],
            1 => vec![spawn_edit(1, "completion_producer")],
            2 => vec![Command {
                player: PlayerId::BENCH,
                seq: 0,
                kind: CommandKind::Edit(Edit::Cargo(PlayerId(0), 0, Fixed::from_int(2000))),
            }],
            3 => vec![Command {
                player: PlayerId(0),
                seq: 0,
                kind: CommandKind::Order(Order::Build {
                    builder: EntityId(1),
                    what: content.key("fx_station").unwrap(),
                    at: Some(pos(10)),
                }),
            }],
            4 => vec![team(0, EntityId(2), PlayerId(1))],
            5 => vec![team(0, EntityId(1), PlayerId(1))],
            _ => Vec::new(),
        };
        let result = recorder.step(&commands);
        assert!(
            !result.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })),
            "tick {tick}: {:?}",
            result.events
        );
        trace.push((
            result.events,
            result.view.digest,
            recorder.sim().planet_crew_capacity(PlayerId(1)),
        ));
    }
    assert_eq!(recorder.sim().planet_crew_capacity(PlayerId(1)), 37);
    let digest = recorder.sim().digest();
    let replay = recorder.finish();
    let decoded = crate::replay::Replay::decode(&replay.encode()).unwrap();
    assert_eq!(decoded.play(content.clone()).unwrap().digest(), digest);
    let mut rerun = Sim::from_setup(&decoded.setup(content).unwrap());
    for (tick, expected) in trace.iter().enumerate() {
        let commands: Vec<_> = decoded
            .commands
            .iter()
            .filter(|logged| logged.tick == tick as u64)
            .map(|logged| logged.command.clone())
            .collect();
        let result = rerun.step(&commands);
        assert_eq!(&result.events, &expected.0);
        assert_eq!(result.view.digest, expected.1);
        assert_eq!(rerun.planet_crew_capacity(PlayerId(1)), expected.2);
    }
    assert_eq!(rerun.digest(), digest);
}
