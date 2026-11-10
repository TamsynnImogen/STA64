//! Public-seam regressions for privileged absolute hull edits.

use std::sync::Arc;

use sta_content::{MapDef, MapObject};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::replay::{Recorder, Replay};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

fn at(x: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::ZERO,
        z: Fixed::ZERO,
    }
}

fn setup() -> GameSetup {
    let content = Arc::new(sta_content::fake::db());
    let mut map = MapDef::empty();
    map.objects = [0, 1000, 2000]
        .into_iter()
        .map(|x| MapObject {
            odf: "fx_cruiser".into(),
            owner: Some(0),
            pos: at(x),
            heading: (Fixed::ZERO, Fixed::ONE),
            label: None,
        })
        .collect();
    GameSetup {
        seed: 53,
        map: Arc::new(map),
        content,
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

fn cruiser_ids(sim: &Sim) -> Vec<EntityId> {
    let key = sim.content().unwrap().key("fx_cruiser").unwrap();
    sim.view()
        .units
        .iter()
        .filter(|unit| unit.def == key)
        .map(|unit| unit.id)
        .collect()
}

fn edit(player: PlayerId, seq: u32, value: Edit) -> Command {
    Command {
        player,
        seq,
        kind: CommandKind::Edit(value),
    }
}

#[test]
fn hull_is_an_absolute_value_and_same_batch_edits_apply_in_sequence() {
    let mut sim = Sim::from_setup(&setup());
    let id = cruiser_ids(&sim)[0];
    assert_eq!(sim.unit(id).unwrap().hull, [Fixed::from_int(800); 2]);

    let out = sim.step(&[
        edit(PlayerId::SCRIPT, 1, Edit::Hull(id, Fixed::from_int(250))),
        edit(PlayerId::SCRIPT, 2, Edit::Hull(id, Fixed::from_int(450))),
    ]);
    assert!(
        !out.events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. }))
    );
    assert_eq!(sim.unit(id).unwrap().hull[0], Fixed::from_int(450));

    let out = sim.step(&[edit(
        PlayerId::SCRIPT,
        3,
        Edit::Hull(id, Fixed::from_int(300)),
    )]);
    assert!(
        !out.events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. }))
    );
    assert_eq!(sim.unit(id).unwrap().hull[0], Fixed::from_int(300));

    let out = sim.step(&[edit(
        PlayerId::BENCH,
        1,
        Edit::Hull(id, Fixed::from_int(375)),
    )]);
    assert!(
        !out.events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. }))
    );
    assert_eq!(sim.unit(id).unwrap().hull[0], Fixed::from_int(375));
}

#[test]
fn hull_caps_at_max_and_mortal_zero_or_negative_hull_despawns_normally() {
    let mut sim = Sim::from_setup(&setup());
    let ids = cruiser_ids(&sim);
    let out = sim.step(&[
        edit(
            PlayerId::SCRIPT,
            1,
            Edit::Hull(ids[0], Fixed::from_int(900)),
        ),
        edit(PlayerId::SCRIPT, 2, Edit::Hull(ids[1], Fixed::ZERO)),
        edit(
            PlayerId::SCRIPT,
            3,
            Edit::Hull(ids[2], Fixed::from_int(-20)),
        ),
    ]);
    assert!(
        !out.events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. }))
    );
    assert_eq!(sim.unit(ids[0]).unwrap().hull[0], Fixed::from_int(800));
    assert!(sim.unit(ids[1]).is_none());
    assert!(sim.unit(ids[2]).is_none());
    for id in [ids[1], ids[2]] {
        assert!(
            out.events
                .iter()
                .any(|event| matches!(event, SimEvent::Destroyed { id: gone, .. } if *gone == id))
        );
    }
}

#[test]
fn hull_rejects_normal_players_none_missing_and_already_dead_targets() {
    let mut sim = Sim::from_setup(&setup());
    let id = cruiser_ids(&sim)[0];
    let initial_hull = sim.unit(id).unwrap().hull[0];

    let normal_player = sim.step(&[edit(PlayerId(0), 1, Edit::Hull(id, Fixed::from_int(100)))]);
    assert!(
        normal_player
            .events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. }))
    );
    assert_eq!(sim.unit(id).unwrap().hull[0], initial_hull);

    let invalid = sim.step(&[
        edit(
            PlayerId::SCRIPT,
            1,
            Edit::Hull(EntityId::NONE, Fixed::from_int(100)),
        ),
        edit(
            PlayerId::SCRIPT,
            2,
            Edit::Hull(EntityId(9999), Fixed::from_int(100)),
        ),
    ]);
    assert_eq!(
        invalid
            .events
            .iter()
            .filter(|event| matches!(event, SimEvent::Rejected { .. }))
            .count(),
        2
    );
    assert_eq!(sim.unit(id).unwrap().hull[0], initial_hull);

    let dead = sim.step(&[
        edit(PlayerId::SCRIPT, 3, Edit::Hull(id, Fixed::ZERO)),
        edit(PlayerId::SCRIPT, 4, Edit::Hull(id, Fixed::from_int(100))),
    ]);
    assert_eq!(
        dead.events
            .iter()
            .filter(|event| matches!(event, SimEvent::Destroyed { id: gone, .. } if *gone == id))
            .count(),
        1
    );
    assert_eq!(
        dead.events
            .iter()
            .filter(|event| matches!(event, SimEvent::Rejected { player: PlayerId::SCRIPT, seq: 4 }))
            .count(),
        1
    );
    assert!(sim.unit(id).is_none());
}

#[test]
fn immortal_hull_floor_is_one_and_lethal_edit_works_after_turning_it_off() {
    let mut sim = Sim::from_setup(&setup());
    let id = cruiser_ids(&sim)[0];
    let protected = sim.step(&[
        edit(PlayerId::SCRIPT, 1, Edit::Immortal(id, true)),
        edit(PlayerId::SCRIPT, 2, Edit::Hull(id, Fixed::ZERO)),
    ]);
    assert!(
        !protected
            .events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. }))
    );
    assert_eq!(sim.unit(id).unwrap().hull[0], Fixed::ONE);

    let lethal = sim.step(&[
        edit(PlayerId::SCRIPT, 3, Edit::Immortal(id, false)),
        edit(PlayerId::SCRIPT, 4, Edit::Hull(id, Fixed::from_int(-1))),
    ]);
    assert!(sim.unit(id).is_none());
    assert!(
        lethal
            .events
            .iter()
            .any(|event| matches!(event, SimEvent::Destroyed { id: gone, .. } if *gone == id))
    );
}

#[test]
fn hull_and_existing_edits_replay_to_the_same_digest() {
    let setup = setup();
    let mut sim = Sim::from_setup(&setup);
    let mut recorder = Recorder::with_interval(&setup, 1);
    let id = cruiser_ids(&sim)[0];
    let batches = [
        vec![edit(
            PlayerId::SCRIPT,
            1,
            Edit::Hull(id, Fixed::from_int(300)),
        )],
        vec![edit(
            PlayerId::SCRIPT,
            2,
            Edit::Damage(id, Fixed::from_int(50)),
        )],
        vec![edit(PlayerId::SCRIPT, 3, Edit::Heal(id))],
        vec![
            edit(PlayerId::SCRIPT, 4, Edit::Immortal(id, true)),
            edit(PlayerId::SCRIPT, 5, Edit::Hull(id, Fixed::ZERO)),
        ],
        vec![
            edit(PlayerId::SCRIPT, 6, Edit::Immortal(id, false)),
            edit(PlayerId::SCRIPT, 7, Edit::Hull(id, Fixed::from_int(-1))),
        ],
    ];
    for batch in &batches {
        let expected = sim.step(batch);
        let recorded = recorder.step(batch);
        assert_eq!(expected.events, recorded.events);
    }
    let replay = Replay::decode(&recorder.finish().encode()).unwrap();
    assert_eq!(replay.play(setup.content).unwrap().digest(), sim.digest());
}
