//! Public-seam regressions for Stop on immobile craft/stations.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, MapObject};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::replay::{Recorder, Replay};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

fn at(x: i32, z: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::ZERO,
        z: Fixed::from_int(z),
    }
}

fn setup() -> GameSetup {
    let fake = sta_content::fake::db();
    let mut features = fake.features().clone();
    features.pay_on_queue = true;
    let mut builder = ContentDb::builder().profile(fake.profile()).features(features);
    for weapon in fake.weapons() {
        builder = builder.weapon(weapon.clone());
    }
    for unit in fake.units() {
        builder = builder.unit(unit.clone());
    }
    let db = Arc::new(builder.build().unwrap());
    let mut map = MapDef::empty();
    map.objects = [
        ("fx_station", Some(0), at(0, 0)),
        ("fx_yard", Some(0), at(2000, 0)),
        // Outside autonomous range; a Fired event would reflect an uncleared order.
        ("fx_cruiser", Some(1), at(0, 1000)),
        ("fx_moon", Some(0), at(4000, 0)),
    ]
    .into_iter()
    .map(|(odf, owner, pos)| MapObject {
        odf: odf.into(),
        owner,
        pos,
        heading: (Fixed::ZERO, Fixed::ONE),
        label: None,
    })
    .collect();
    GameSetup {
        seed: 41,
        map: Arc::new(map),
        content: db,
        players: (0..2)
            .map(|n| PlayerSetup {
                id: PlayerId(n),
                team: TeamId(n),
                race: "fx".into(),
                control: Control::Idle,
            })
            .collect(),
        options: GameOptions::default(),
    }
}

fn id(sim: &Sim, odf: &str) -> EntityId {
    let key = sim.content().unwrap().key(odf).unwrap();
    sim.view().units.iter().find(|unit| unit.def == key).unwrap().id
}

fn command(player: PlayerId, seq: u32, order: Order) -> Command {
    Command {
        player,
        seq,
        kind: CommandKind::Order(order),
    }
}

fn stopped(events: &[SimEvent]) -> bool {
    !events.iter().any(|event| matches!(event, SimEvent::Rejected { .. }))
}

#[test]
fn stationary_station_stop_cancels_attack_and_replays_identically() {
    let setup = setup();
    let mut sim = Sim::from_setup(&setup);
    let mut recorder = Recorder::with_interval(&setup, 1);
    let station = id(&sim, "fx_station");
    let enemy = id(&sim, "fx_cruiser");
    let attack = command(
        PlayerId(0),
        1,
        Order::Attack {
            units: vec![station],
            target: enemy,
        },
    );
    let mut unstopped = Sim::from_setup(&setup);
    assert!(stopped(&sim.step(std::slice::from_ref(&attack)).events));
    unstopped.step(std::slice::from_ref(&attack));
    recorder.step(&[attack]);
    let stop = command(PlayerId(0), 2, Order::Stop { units: vec![station] });
    let result = sim.step(std::slice::from_ref(&stop));
    assert!(
        stopped(&result.events),
        "station Stop was rejected: {:?}",
        result.events
    );
    unstopped.step(&[]);
    assert_ne!(
        sim.digest(),
        unstopped.digest(),
        "Stop did not clear the station's attack state"
    );
    recorder.step(&[stop]);
    for _ in 0..100 {
        let out = sim.step(&[]);
        assert!(
            !out.events
                .iter()
                .any(|event| matches!(event, SimEvent::Fired { shooter, .. } if *shooter == station))
        );
        recorder.step(&[]);
    }
    assert_eq!(sim.digest(), recorder.sim().digest());
    let replay = Replay::decode(&recorder.finish().encode()).unwrap();
    assert_eq!(replay.play(setup.content).unwrap().digest(), sim.digest());
}

#[test]
fn stationary_yard_stop_refunds_paid_queue_and_never_builds() {
    let setup = setup();
    let mut sim = Sim::from_setup(&setup);
    let yard = id(&sim, "fx_yard");
    let cruiser = setup.content.key("fx_cruiser").unwrap();
    let fund = Command {
        player: PlayerId::SCRIPT,
        seq: 1,
        kind: CommandKind::Edit(Edit::Cargo(PlayerId(0), 0, Fixed::from_int(1000))),
    };
    assert!(stopped(&sim.step(&[fund]).events));
    let build = command(
        PlayerId(0),
        1,
        Order::Build {
            builder: yard,
            what: cruiser,
            at: None,
        },
    );
    assert!(stopped(&sim.step(&[build]).events));
    assert_eq!(
        sim.view().player(PlayerId(0)).unwrap().resources[0],
        Fixed::from_int(400)
    );
    let stop = command(PlayerId(0), 2, Order::Stop { units: vec![yard] });
    let out = sim.step(&[stop]);
    assert!(stopped(&out.events), "yard Stop was rejected: {:?}", out.events);
    assert_eq!(
        sim.view().player(PlayerId(0)).unwrap().resources[0],
        Fixed::from_int(1000)
    );
    for _ in 0..1000 {
        let out = sim.step(&[]);
        assert!(
            !out.events
                .iter()
                .any(|event| matches!(event, SimEvent::Built { builder, .. } if *builder == yard))
        );
    }
    let again = sim.step(&[command(PlayerId(0), 3, Order::Stop { units: vec![yard] })]);
    assert!(stopped(&again.events), "idempotent yard Stop was rejected");
}

#[test]
fn invalid_stop_rejects_without_mutating_active_attack_or_queue() {
    let setup = setup();
    let station = id(&Sim::from_setup(&setup), "fx_station");
    let yard = id(&Sim::from_setup(&setup), "fx_yard");
    let enemy = id(&Sim::from_setup(&setup), "fx_cruiser");
    let moon = id(&Sim::from_setup(&setup), "fx_moon");
    for (player, target) in [
        (PlayerId(1), station),
        (PlayerId(0), EntityId(999)),
        (PlayerId(0), moon),
        (PlayerId::SCRIPT, moon), // Kind rejection also bypasses owner filtering.
    ] {
        let mut sim = Sim::from_setup(&setup);
        let mut control = Sim::from_setup(&setup);
        let prime = [
            Command {
                player: PlayerId::SCRIPT,
                seq: 1,
                kind: CommandKind::Edit(Edit::Cargo(PlayerId(0), 0, Fixed::from_int(1000))),
            },
            command(
                PlayerId(0),
                1,
                Order::Build {
                    builder: yard,
                    what: setup.content.key("fx_cruiser").unwrap(),
                    at: None,
                },
            ),
            command(
                PlayerId(0),
                2,
                Order::Attack {
                    units: vec![station],
                    target: enemy,
                },
            ),
        ];
        // Fund in an earlier tick: SCRIPT sorts after player orders in one batch.
        for batch in [&prime[..1], &prime[1..]] {
            assert!(stopped(&sim.step(batch).events));
            control.step(batch);
        }
        let result = sim.step(&[command(player, 9, Order::Stop { units: vec![target] })]);
        control.step(&[]);
        assert!(
            result
                .events
                .iter()
                .any(|event| matches!(event, SimEvent::Rejected { .. }))
        );
        assert_eq!(
            sim.digest(),
            control.digest(),
            "invalid Stop changed state for {player:?} {target:?}"
        );
    }
}

#[test]
fn mixed_stop_filters_foreign_actor_script_can_stop_and_move_stays_invalid() {
    let setup = setup();
    let station = id(&Sim::from_setup(&setup), "fx_station");
    let enemy = id(&Sim::from_setup(&setup), "fx_cruiser");
    let mut mixed = Sim::from_setup(&setup);
    let mut own_only = Sim::from_setup(&setup);
    let attacks = [
        command(
            PlayerId(0),
            1,
            Order::Attack {
                units: vec![station],
                target: enemy,
            },
        ),
        command(
            PlayerId(1),
            1,
            Order::Attack {
                units: vec![enemy],
                target: station,
            },
        ),
    ];
    assert!(stopped(&mixed.step(&attacks).events));
    own_only.step(&attacks);
    let out = mixed.step(&[command(
        PlayerId(0),
        2,
        Order::Stop {
            units: vec![station, enemy],
        },
    )]);
    own_only.step(&[command(PlayerId(0), 2, Order::Stop { units: vec![station] })]);
    assert!(stopped(&out.events));
    assert_eq!(mixed.digest(), own_only.digest(), "mixed Stop touched foreign attack");

    let mut script = Sim::from_setup(&setup);
    assert!(stopped(
        &script
            .step(&[command(PlayerId::SCRIPT, 1, Order::Stop { units: vec![station] })])
            .events
    ));
    let mut control = Sim::from_setup(&setup);
    for order in [
        Order::Move {
            units: vec![station],
            to: at(500, 0),
            queue: false,
        },
        Order::Formation {
            units: vec![station],
            kind: 0,
        },
    ] {
        let out = control.step(&[command(PlayerId(0), 7, order)]);
        assert!(
            out.events
                .iter()
                .any(|event| matches!(event, SimEvent::Rejected { .. }))
        );
    }
}
