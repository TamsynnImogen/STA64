use crate::command::{Command, CommandKind, Edit, Order, Target};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::view::UnitFlags;
use crate::world::NEUTRAL;
use crate::{EntityId, Fixed, MoveStats, PlayerId, Sim, SimEvent, TeamId, Vec3, WorldSource};
use sta_content::TechState;
use sta_content::{ContentDb, MapDef, UnitDef};
use std::sync::Arc;

fn pos(x: i32) -> Vec3 {
    Vec3::from_f32([x as f32, 0.0, 0.0])
}

fn game(seats: &[u8], pay_on_queue: bool) -> Sim {
    let base = sta_content::fake::db();
    let mut features = base.features().clone();
    features.pay_on_queue = pay_on_queue;
    let mut rig = UnitDef::new("fx_rig", "constructionrig");
    rig.race = "fx".into();
    rig.movement = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
    rig.hull = Fixed::from_int(100);
    let mut builder = ContentDb::builder()
        .profile(base.profile())
        .features(features)
        .unit(rig);
    for unit in base.units() {
        builder = builder.unit(unit.clone());
    }
    for weapon in base.weapons() {
        builder = builder.weapon(weapon.clone());
    }
    Sim::from_setup(&GameSetup {
        seed: 19,
        map: Arc::new(MapDef::empty()),
        content: Arc::new(builder.build().unwrap()),
        players: seats
            .iter()
            .map(|&id| PlayerSetup {
                id: PlayerId(id),
                team: TeamId(id),
                race: "fx".into(),
                control: Control::Human,
            })
            .collect(),
        options: GameOptions::default(),
    })
}

fn command(player: PlayerId, seq: u32, kind: CommandKind) -> Command {
    Command { player, seq, kind }
}

fn team(player: PlayerId, seq: u32, id: EntityId, to: PlayerId) -> Command {
    command(player, seq, CommandKind::Edit(Edit::Team(id, to)))
}

fn order(player: u8, kind: Order) -> Command {
    command(PlayerId(player), 0, CommandKind::Order(kind))
}

fn accepted(sim: &mut Sim, commands: &[Command]) -> Vec<SimEvent> {
    let events = sim.step(commands).events;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. })),
        "{events:?}"
    );
    events
}

fn rejected(sim: &mut Sim, commands: &[Command]) {
    let events = sim.step(commands).events;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. })),
        "{events:?}"
    );
}

fn spawn(sim: &mut Sim, odf: &str, owner: PlayerId, at: Vec3) -> EntityId {
    let key = sim.content().unwrap().key(odf).unwrap();
    sim.spawn_def(key, owner, at, (Fixed::ZERO, Fixed::ONE))
        .unwrap()
}

#[test]
fn team_edit_validates_authority_target_and_destinations_before_mutation() {
    let mut sim = game(&[0, 3], true);
    let craft = spawn(&mut sim, "fx_frigate", PlayerId(0), pos(0));
    let station = spawn(&mut sim, "fx_station", PlayerId(0), pos(1000));
    let resource = spawn(&mut sim, "fx_moon", NEUTRAL, pos(2000));
    let baseline = sim.unit(craft).unwrap().clone();
    for (from, id, to) in [
        (PlayerId(0), craft, PlayerId(3)),
        (PlayerId::SCRIPT, EntityId(9999), PlayerId(3)),
        (PlayerId::SCRIPT, resource, PlayerId(3)),
        (PlayerId::SCRIPT, craft, PlayerId(1)), // unconfigured sparse seat
        (PlayerId::SCRIPT, craft, PlayerId(8)),
        (PlayerId::SCRIPT, craft, PlayerId::SCRIPT),
        (PlayerId::SCRIPT, craft, PlayerId::BENCH),
    ] {
        rejected(&mut sim, &[team(from, 0, id, to)]);
        assert_eq!(sim.unit(craft).unwrap(), &baseline);
    }
    sim.units.get_mut(craft).unwrap().flags = UnitFlags::DYING;
    rejected(&mut sim, &[team(PlayerId::SCRIPT, 0, craft, PlayerId(3))]);
    sim.units.get_mut(craft).unwrap().flags = baseline.flags;
    sim.units.get_mut(craft).unwrap().hull[0] = Fixed::ZERO;
    rejected(&mut sim, &[team(PlayerId::SCRIPT, 0, craft, PlayerId(3))]);
    sim.units.get_mut(craft).unwrap().hull[0] = baseline.hull[0];
    assert_eq!(sim.unit(craft).unwrap(), &baseline);
    accepted(&mut sim, &[team(PlayerId::BENCH, 0, station, PlayerId(3))]);
    assert_eq!(sim.unit(station).unwrap().owner, PlayerId(3));
    sim.units.get_mut(station).unwrap().progress = Some(Fixed::ONE);
    rejected(&mut sim, &[team(PlayerId::SCRIPT, 0, station, PlayerId(0))]);
    assert_eq!(sim.unit(station).unwrap().owner, PlayerId(3));
    sim.units.get_mut(station).unwrap().progress = None;
    accepted(&mut sim, &[team(PlayerId::SCRIPT, 0, craft, NEUTRAL)]);
    assert_eq!(sim.unit(craft).unwrap().owner, NEUTRAL);
    sim.victory = crate::victory::Victory::from_rec(&crate::victory::VictoryRec {
        eliminated: vec![3],
    });
    rejected(&mut sim, &[team(PlayerId::SCRIPT, 0, craft, PlayerId(3))]);
    assert_eq!(sim.unit(craft).unwrap().owner, NEUTRAL);

    let mut bare = Sim::new(19);
    let id = bare.spawn(0, pos(0), (Fixed::ZERO, Fixed::ONE), MoveStats::STATIC);
    bare.units.get_mut(id).unwrap().hull = [Fixed::ONE; 2];
    accepted(&mut bare, &[team(PlayerId::SCRIPT, 0, id, PlayerId(7))]);
    assert_eq!(bare.unit(id).unwrap().owner, PlayerId(7));
    rejected(&mut bare, &[team(PlayerId::SCRIPT, 0, id, PlayerId(8))]);
    assert_eq!(bare.unit(id).unwrap().owner, PlayerId(7));
}

#[test]
fn team_edit_stops_actor_activity_and_new_owner_controls_later_ticks() {
    let mut sim = game(&[0, 1], true);
    let craft = spawn(&mut sim, "fx_cruiser", PlayerId(0), pos(0));
    let enemy = spawn(&mut sim, "fx_frigate", PlayerId(1), pos(10000));
    let own_yard = spawn(&mut sim, "fx_yard", PlayerId(0), pos(2000));
    let other_yard = spawn(&mut sim, "fx_yard", PlayerId(0), pos(3000));
    accepted(
        &mut sim,
        &[order(
            0,
            Order::Move {
                units: vec![craft],
                to: pos(500),
                queue: false,
            },
        )],
    );
    accepted(
        &mut sim,
        &[order(
            0,
            Order::Move {
                units: vec![craft],
                to: pos(800),
                queue: true,
            },
        )],
    );
    accepted(
        &mut sim,
        &[order(
            0,
            Order::Attack {
                units: vec![craft],
                target: enemy,
            },
        )],
    );
    accepted(
        &mut sim,
        &[order(
            0,
            Order::Rally {
                builders: vec![own_yard],
                at: Target::Point(pos(900)),
            },
        )],
    );
    accepted(
        &mut sim,
        &[order(
            0,
            Order::Rally {
                builders: vec![other_yard],
                at: Target::Entity(craft),
            },
        )],
    );
    assert!(sim.movement.course(craft).is_some());
    assert_eq!(sim.weapons.attack_of(craft), Some(enemy));
    assert_eq!(sim.production.to_rec().rallies.len(), 2);

    accepted(
        &mut sim,
        &[
            team(PlayerId::SCRIPT, 0, craft, PlayerId(0)),
            team(PlayerId::SCRIPT, 1, craft, PlayerId(1)),
        ],
    );
    assert_eq!(sim.unit(craft).unwrap().owner, PlayerId(1));
    assert_eq!(sim.unit(craft).unwrap().target, None);
    assert!(sim.movement.course(craft).is_none());
    assert_eq!(sim.weapons.attack_of(craft), None);
    assert_eq!(
        sim.production.to_rec().rallies.len(),
        2,
        "another builder's target stays live"
    );
    accepted(
        &mut sim,
        &[team(PlayerId::SCRIPT, 0, own_yard, PlayerId(0))],
    );
    let rallies = sim.production.to_rec().rallies;
    assert_eq!(rallies.len(), 1);
    assert_eq!(rallies[0].0, other_yard);
    assert_eq!(rallies[0].2, Target::Entity(craft));

    rejected(
        &mut sim,
        &[order(
            0,
            Order::Move {
                units: vec![craft],
                to: pos(1000),
                queue: false,
            },
        )],
    );
    accepted(
        &mut sim,
        &[order(
            1,
            Order::Move {
                units: vec![craft],
                to: pos(1200),
                queue: false,
            },
        )],
    );
    assert_eq!(sim.unit(craft).unwrap().target, Some(pos(1200)));
}

fn fund(sim: &mut Sim, amount: i32) {
    accepted(
        sim,
        &[command(
            PlayerId::BENCH,
            0,
            CommandKind::Edit(Edit::Cargo(PlayerId(0), 0, Fixed::from_int(amount))),
        )],
    );
}

fn build(sim: &mut Sim, builder: EntityId, odf: &str, at: Option<Vec3>) {
    let what = sim.content().unwrap().key(odf).unwrap();
    accepted(sim, &[order(0, Order::Build { builder, what, at })]);
}

fn rejected_without_state_change(sim: &mut Sim, id: EntityId) {
    let edit = Edit::Team(id, PlayerId(1));
    let digest = sim.digest();
    let unit = sim.unit(id).unwrap().clone();
    let production = sim.production.to_rec();
    let stock = sim.economy.stock_of(PlayerId(0));
    assert!(!sim.apply_edit(PlayerId::SCRIPT, &edit));
    assert_eq!(sim.digest(), digest);
    assert_eq!(sim.unit(id).unwrap(), &unit);
    assert_eq!(sim.production.to_rec(), production);
    assert_eq!(sim.economy.stock_of(PlayerId(0)), stock);
    let mut empty_step = sim.clone();
    rejected(sim, &[team(PlayerId::SCRIPT, 0, id, PlayerId(1))]);
    empty_step.step(&[]);
    assert_eq!(sim.digest(), empty_step.digest());
    assert_eq!(sim.production.to_rec(), empty_step.production.to_rec());
}

#[test]
fn active_queue_deploy_and_weld_cleanup_obey_both_pay_modes() {
    for pay_on_queue in [false, true] {
        let mut queued = game(&[0, 1], pay_on_queue);
        let yard = spawn(&mut queued, "fx_yard", PlayerId(0), pos(0));
        fund(&mut queued, 2000);
        build(&mut queued, yard, "fx_frigate", None);
        build(&mut queued, yard, "fx_frigate", None);
        assert_eq!(queued.production.queue(yard).len(), 2);
        let before = queued.economy.stock_of(PlayerId(0));
        accepted(&mut queued, &[team(PlayerId::SCRIPT, 0, yard, PlayerId(1))]);
        assert!(queued.production.queue(yard).is_empty());
        assert_eq!(queued.unit(yard).unwrap().owner, PlayerId(1));
        let expected = if pay_on_queue {
            [2000, 0, 0, 0].map(Fixed::from_int).to_vec()
        } else {
            before
        };
        assert_eq!(queued.economy.stock_of(PlayerId(0)), expected);
        assert!(queued.economy.stock_of(PlayerId(1)).is_empty());

        let mut deploying = game(&[0, 1], pay_on_queue);
        let rig = spawn(&mut deploying, "fx_rig", PlayerId(0), pos(0));
        fund(&mut deploying, 2000);
        build(&mut deploying, rig, "fx_station", Some(pos(400)));
        assert!(deploying.production.is_deploying(rig));
        accepted(&mut deploying, &[team(PlayerId::SCRIPT, 0, rig, PlayerId(1))]);
        assert!(!deploying.production.is_deploying(rig));
        assert_eq!(deploying.unit(rig).unwrap().owner, PlayerId(1));
        assert_eq!(deploying.economy.stock_of(PlayerId(0))[0], Fixed::from_int(2000));

        let mut welding = game(&[0, 1], pay_on_queue);
        let rig = spawn(&mut welding, "fx_rig", PlayerId(0), pos(0));
        fund(&mut welding, 2000);
        build(&mut welding, rig, "fx_station", Some(pos(10)));
        let welds = welding.production.to_rec().welds;
        assert_eq!(welds.len(), 1);
        let child = EntityId(welds[0].0);
        assert!(welding.unit(child).unwrap().progress.is_some());
        rejected_without_state_change(&mut welding, child);
        accepted(&mut welding, &[team(PlayerId::SCRIPT, 0, rig, PlayerId(1))]);
        assert!(welding.unit(child).is_none());
        assert!(welding.production.to_rec().welds.is_empty());
        assert_eq!(welding.unit(rig).unwrap().owner, PlayerId(1));
        let expected = if pay_on_queue { 2000 } else { 800 };
        assert_eq!(welding.economy.stock_of(PlayerId(0))[0], Fixed::from_int(expected));
    }
}

#[test]
fn loaded_cargo_survives_transfer_and_delivers_to_the_new_owners_station() {
    let mut sim = game(&[0, 1], true);
    let node = spawn(&mut sim, "fx_moon", NEUTRAL, pos(0));
    let miner = spawn(&mut sim, "fx_miner", PlayerId(0), pos(0));
    spawn(&mut sim, "fx_station", PlayerId(1), pos(0));
    accepted(
        &mut sim,
        &[order(
            0,
            Order::Mine {
                units: vec![miner],
                node,
            },
        )],
    );
    for _ in 0..10 {
        sim.step(&[]);
    }
    let cargo = sim.economy.cargo(miner).unwrap();
    assert!(cargo > Fixed::ZERO);
    accepted(&mut sim, &[team(PlayerId::SCRIPT, 0, miner, PlayerId(1))]);
    assert_eq!(sim.economy.cargo(miner), Some(cargo));
    assert!(!sim.economy.is_mining(miner));
    for _ in 0..20 {
        sim.step(&[]);
    }
    assert_eq!(sim.economy.stock_of(PlayerId(1)), Vec::<Fixed>::new());
    accepted(
        &mut sim,
        &[order(
            1,
            Order::Mine {
                units: vec![miner],
                node,
            },
        )],
    );
    let mut deliveries = Vec::new();
    for _ in 0..30 {
        deliveries.extend(sim.step(&[]).events);
    }
    assert_eq!(sim.economy.stock_of(PlayerId(0)), Vec::<Fixed>::new());
    assert_eq!(sim.economy.stock_of(PlayerId(1))[0], cargo);
    assert!(deliveries.contains(&SimEvent::Delivered {
        player: PlayerId(1),
        resource: 0,
        amount: cargo
    }));
}

#[test]
fn owner_change_preserves_physical_state() {
    let content = Arc::new(crate::specials::fixture::db());
    let sim_setup = GameSetup {
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
    let mut sim = Sim::from_setup(&sim_setup);
    let id = spawn(&mut sim, "fx_cloaker", PlayerId(0), pos(0));
    let enemy = spawn(&mut sim, "fx_frigate", PlayerId(1), pos(10000));
    sim.labels.insert(id, "retained".into());
    accepted(
        &mut sim,
        &[command(
            PlayerId::SCRIPT,
            0,
            CommandKind::Edit(Edit::SetEnergy(id, Fixed::from_int(375))),
        )],
    );
    accepted(
        &mut sim,
        &[order(
            0,
            Order::Attack {
                units: vec![id],
                target: enemy,
            },
        )],
    );
    accepted(
        &mut sim,
        &[order(
            0,
            Order::Cloak {
                units: vec![id],
                on: true,
            },
        )],
    );
    assert_eq!(sim.weapons.attack_of(id), Some(enemy));
    let mut expected = sim.unit(id).unwrap().clone();
    let energy = sim.specials.energy_of(id);
    let cloak = sim.specials.cloak_state(id);
    let specials = sim.specials.to_rec();
    let label = sim.labels.get(id).cloned();
    assert!(energy.is_some());
    assert!(cloak.is_some());
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::Team(id, PlayerId(1))));
    expected.owner = PlayerId(1);
    assert_eq!(sim.unit(id).unwrap(), &expected);
    assert_eq!(sim.labels.get(id), label.as_ref());
    assert_eq!(sim.specials.energy_of(id), energy);
    assert_eq!(sim.specials.cloak_state(id), cloak);
    assert_eq!(sim.specials.to_rec(), specials);
    // The actor's explicit attack is intentionally cleared.
    assert_eq!(sim.weapons.attack_of(id), None);
    assert!(!sim.victory.is_eliminated(PlayerId(0)));
    accepted(&mut sim, &[team(PlayerId::BENCH, 0, id, PlayerId(0))]);
    assert!(!sim.victory.is_eliminated(PlayerId(0)));
}

#[test]
fn ownership_change_recomputes_visibility_and_tech_holdings() {
    let mut sim = game(&[0, 1], true);
    let craft = spawn(&mut sim, "fx_frigate", PlayerId(0), pos(0));
    spawn(&mut sim, "fx_frigate", PlayerId(1), pos(10000));
    let count = |sim: &Sim, owner: PlayerId| {
        crate::tech::Owned::new(
            sim.content().unwrap(),
            &sim.units,
            &sim.players,
            owner,
            "fx",
        )
        .count("fx_frigate")
    };
    assert_eq!(count(&sim, PlayerId(0)), 1);
    assert!(
        sim.view()
            .visible_to(PlayerId(0))
            .iter()
            .any(|u| u.id == craft)
    );
    assert!(
        !sim.view()
            .visible_to(PlayerId(1))
            .iter()
            .any(|u| u.id == craft)
    );
    accepted(&mut sim, &[team(PlayerId::SCRIPT, 0, craft, PlayerId(1))]);
    assert_eq!(count(&sim, PlayerId(0)), 0);
    assert_eq!(count(&sim, PlayerId(1)), 2);
    assert!(
        !sim.view()
            .visible_to(PlayerId(0))
            .iter()
            .any(|u| u.id == craft)
    );
    assert!(
        sim.view()
            .visible_to(PlayerId(1))
            .iter()
            .any(|u| u.id == craft)
    );
}

#[test]
fn transfer_keeps_mount_cooldowns_and_existing_shots() {
    let mut sim = game(&[0, 1], true);
    let actor = spawn(&mut sim, "fx_cruiser", PlayerId(0), pos(0));
    let target = spawn(&mut sim, "fx_miner", PlayerId(1), pos(700));
    accepted(
        &mut sim,
        &[order(
            0,
            Order::Attack {
                units: vec![actor],
                target,
            },
        )],
    );
    assert!(sim.weapons.shots_in_flight() > 0);
    let mut expected = sim.weapons.clone();
    expected.stop_attack(&sim.units, PlayerId::SCRIPT, &[actor]);
    let bytes = postcard::to_allocvec(&expected.to_rec()).unwrap();
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::Team(actor, PlayerId(1))));
    assert_eq!(postcard::to_allocvec(&sim.weapons.to_rec()).unwrap(), bytes);
}

#[test]
fn transferred_loaded_miner_snapshot_continues_with_new_owner_delivery() {
    let mut sim = game(&[0, 1], true);
    let node = spawn(&mut sim, "fx_moon", NEUTRAL, pos(0));
    let station = spawn(&mut sim, "fx_station", PlayerId(1), pos(1000));
    let miner = spawn(&mut sim, "fx_miner", PlayerId(0), pos(0));
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::Weapons(station, false)));
    accepted(
        &mut sim,
        &[order(
            0,
            Order::Mine {
                units: vec![miner],
                node,
            },
        )],
    );
    for _ in 0..10 {
        sim.step(&[]);
    }
    let cargo = sim.economy.cargo(miner).unwrap();
    assert!(cargo > Fixed::ZERO);
    accepted(&mut sim, &[team(PlayerId::SCRIPT, 0, miner, PlayerId(1))]);
    let bytes = sim.snapshot();
    let mut loaded = Sim::from_snapshot(&bytes, sim.content().cloned()).unwrap();
    assert_eq!(loaded.snapshot(), bytes);
    for tick in 0..800 {
        let commands = if tick == 0 {
            vec![order(
                1,
                Order::Mine {
                    units: vec![miner],
                    node,
                },
            )]
        } else if tick == 1 {
            vec![order(0, Order::Stop { units: vec![miner] })]
        } else {
            vec![]
        };
        let original = sim.step(&commands);
        let restored = loaded.step(&commands);
        assert_eq!(original.events, restored.events);
        assert_eq!(original.view.digest, restored.view.digest);
        if tick == 1 {
            assert!(
                original
                    .events
                    .iter()
                    .any(|e| matches!(e, SimEvent::Rejected { .. }))
            );
        }
    }
    assert_eq!(sim.snapshot(), loaded.snapshot());
    assert!(sim.economy.stock_of(PlayerId(1))[0] >= cargo);
    assert!(sim.economy.stock_of(PlayerId(0)).is_empty());
}

#[test]
fn existing_team_command_replays_same_build_with_identical_events() {
    let content = game(&[0, 1], true).content().unwrap().clone();
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
    let mut recorded = crate::replay::Recorder::with_interval(&setup, 1);
    let mut events = Vec::new();
    for tick in 0..120 {
        let commands = match tick {
            0 => vec![command(
                PlayerId::SCRIPT,
                0,
                CommandKind::Edit(Edit::Spawn {
                    odf: content.key("fx_miner").unwrap(),
                    player: PlayerId(0),
                    pos: pos(0),
                    heading: (Fixed::ZERO, Fixed::ONE),
                }),
            )],
            1 => vec![order(
                0,
                Order::Move {
                    units: vec![EntityId(1)],
                    to: pos(1000),
                    queue: true,
                },
            )],
            5 => vec![team(PlayerId::SCRIPT, 0, EntityId(1), PlayerId(1))],
            6 => vec![order(
                0,
                Order::Move {
                    units: vec![EntityId(1)],
                    to: pos(-1000),
                    queue: false,
                },
            )],
            7 => vec![order(
                1,
                Order::Move {
                    units: vec![EntityId(1)],
                    to: pos(500),
                    queue: false,
                },
            )],
            25 => vec![
                team(PlayerId::BENCH, 0, EntityId(1), NEUTRAL),
                team(PlayerId::BENCH, 1, EntityId(1), PlayerId(0)),
            ],
            _ => vec![],
        };
        events.push(recorded.step(&commands).events);
    }
    let digest = recorded.sim().digest();
    let replay = recorded.finish();
    let decoded = crate::replay::Replay::decode(&replay.encode()).unwrap();
    assert_eq!(decoded.play(content.clone()).unwrap().digest(), digest);
    let mut rerun = Sim::from_setup(&decoded.setup(content).unwrap());
    for (tick, expected) in events.iter().enumerate() {
        let commands: Vec<_> = decoded
            .commands
            .iter()
            .filter(|logged| logged.tick == tick as u64)
            .map(|logged| logged.command.clone())
            .collect();
        assert_eq!(&rerun.step(&commands).events, expected);
    }
    assert_eq!(rerun.digest(), digest);
}

#[test]
fn metadata_resources_stay_neutral_even_with_craft_or_station_class() {
    for class in ["craft", "starbase"] {
        let mut def = UnitDef::new("metadata_resource", class);
        def.hull = Fixed::from_int(100);
        def.resource = Some(0);
        let content = Arc::new(ContentDb::builder().unit(def).build().unwrap());
        let setup = GameSetup {
            seed: 19,
            map: Arc::new(MapDef::empty()),
            content,
            players: vec![PlayerSetup {
                id: PlayerId(0),
                team: TeamId(0),
                race: String::new(),
                control: Control::Human,
            }],
            options: GameOptions::default(),
        };
        let mut sim = Sim::from_setup(&setup);
        let id = spawn(&mut sim, "metadata_resource", PlayerId(0), pos(0));
        assert_ne!(
            sim.unit(id).unwrap().kind,
            crate::world::EntityKind::Resource
        );
        assert_eq!(sim.unit(id).unwrap().owner, NEUTRAL);
        sim.units.get_mut(id).unwrap().target = Some(pos(100));
        let before = sim.snapshot();
        for to in [PlayerId(0), NEUTRAL] {
            assert!(!sim.apply_edit(PlayerId::SCRIPT, &Edit::Team(id, to)));
            assert_eq!(sim.snapshot(), before);
        }
    }
}
