use std::sync::Arc;

use super::*;
use crate::command::{Command, CommandKind, Order};
use crate::replay::{Recorder, Replay};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Fixed, Sim, SimEvent, TeamId, WorldSource};
use sta_content::{DefRef, MapDef, MapObject, UnitDef};

const YARD: EntityId = EntityId(1);
const SECOND: EntityId = EntityId(2);
const LEADER: EntityId = EntityId(3);
const ENEMY: EntityId = EntityId(4);

fn p(x: i32, z: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::ZERO,
        z: Fixed::from_int(z),
    }
}

fn setup() -> GameSetup {
    let fake = sta_content::fake::db();
    let mut b = ContentDb::builder()
        .profile(fake.profile())
        .features(fake.features().clone());
    for u in fake.units() {
        let mut u = u.clone();
        u.hardpoints.clear();
        u.build.ticks = 2;
        u.build.cost.clear();
        // Even if a station has inherited movement numbers, normal Move
        // orders require the craft kind. Rally must use the same rule.
        if u.class.0 == "starbase" {
            u.movement = fake.unit(fake.key("fx_frigate").unwrap()).unwrap().movement;
        }
        b = b.unit(u);
    }
    let mut rig = UnitDef::new("fx_rig", "constructionrig");
    rig.movement = fake.unit(fake.key("fx_frigate").unwrap()).unwrap().movement;
    rig.build.list = vec![DefRef::new("fx_station")];
    rig.hull = Fixed::from_int(100);
    b = b.unit(rig);
    let mut map = MapDef::empty();
    let object = |odf: &str, owner, pos| MapObject {
        odf: odf.into(),
        owner: Some(owner),
        pos,
        heading: (Fixed::ZERO, Fixed::ONE),
        label: None,
    };
    map.objects = vec![
        object("fx_yard", 0, p(0, 0)),
        object("fx_yard", 0, p(2000, 0)),
        object("fx_frigate", 0, p(0, 1200)),
        object("fx_yard", 1, p(4000, 0)),
        object("fx_rig", 0, p(-2000, 0)),
    ];
    GameSetup {
        seed: 3,
        map: Arc::new(map),
        content: Arc::new(b.build().unwrap()),
        players: (0..2)
            .map(|id| PlayerSetup {
                id: PlayerId(id),
                team: TeamId(id),
                race: "fx".into(),
                control: Control::Human,
            })
            .collect(),
        options: GameOptions::default(),
    }
}

fn order(from: u8, seq: u32, o: Order) -> Command {
    Command {
        player: PlayerId(from),
        seq,
        kind: CommandKind::Order(o),
    }
}

fn rally(ids: &[EntityId], at: Target) -> Command {
    order(
        0,
        1,
        Order::Rally {
            builders: ids.to_vec(),
            at,
        },
    )
}

fn build(s: &Sim, builder: EntityId) -> Command {
    order(
        0,
        2,
        Order::Build {
            builder,
            what: s.content().unwrap().key("fx_frigate").unwrap(),
            at: None,
        },
    )
}

fn accepted(s: &mut Sim, cmds: &[Command]) -> Vec<SimEvent> {
    let out = s.step(cmds);
    assert!(
        !out.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })),
        "{:?}",
        out.events
    );
    out.events
}

fn completed(s: &mut Sim, ticks: usize) -> Vec<(EntityId, EntityId)> {
    let mut built = Vec::new();
    for _ in 0..ticks {
        for e in accepted(s, &[]) {
            if let SimEvent::Built { builder, id } = e {
                built.push((builder, id));
            }
        }
    }
    built
}

#[test]
fn each_builder_sends_every_queued_ship_to_its_own_rally() {
    let mut s = Sim::from_setup(&setup());
    let a = p(0, 1000);
    let b = p(2000, 800);
    accepted(&mut s, &[rally(&[YARD], Target::Point(a))]);
    accepted(&mut s, &[rally(&[SECOND], Target::Point(b))]);
    let mut builds = vec![build(&s, YARD), build(&s, YARD), build(&s, SECOND)];
    for (i, c) in builds.iter_mut().enumerate() {
        c.seq = i as u32 + 10;
    }
    accepted(&mut s, &builds);
    let made = completed(&mut s, 350);
    assert_eq!(made.len(), 3);
    assert_eq!(made.iter().filter(|(yard, _)| *yard == YARD).count(), 2);
    for (yard, id) in made {
        let u = s.unit(id).unwrap();
        assert_eq!(u.pos, if yard == YARD { a } else { b });
        assert_eq!(u.target, None);
    }
    assert_eq!(s.unit(YARD).unwrap().pos, p(0, 0));
    assert_eq!(s.unit(YARD).unwrap().target, None);
}

#[test]
fn entity_rally_samples_the_current_position_at_each_completion() {
    let mut s = Sim::from_setup(&setup());
    let cmd = build(&s, YARD);
    accepted(&mut s, &[rally(&[YARD], Target::Entity(LEADER)), cmd]);
    let made = completed(&mut s, 2);
    assert_eq!(made.len(), 1);
    assert_eq!(s.unit(made[0].1).unwrap().target, Some(p(0, 1200)));
    accepted(
        &mut s,
        &[order(
            0,
            3,
            Order::Move {
                units: vec![LEADER],
                to: p(0, 2400),
                queue: false,
            },
        )],
    );
    completed(&mut s, 300);
    assert_eq!(s.unit(LEADER).unwrap().pos, p(0, 2400));
    assert_eq!(s.unit(made[0].1).unwrap().pos, p(0, 1200));
    let cmd = build(&s, YARD);
    accepted(&mut s, &[cmd]);
    let second = completed(&mut s, 2);
    assert_eq!(second.len(), 1);
    assert_eq!(s.unit(second[0].1).unwrap().target, Some(p(0, 2400)));
}

#[test]
fn clear_does_not_cancel_a_queue_and_the_next_ship_is_idle() {
    let mut s = Sim::from_setup(&setup());
    let cmd = build(&s, YARD);
    accepted(&mut s, &[rally(&[YARD], Target::Point(p(0, 1000))), cmd]);
    accepted(&mut s, &[rally(&[YARD], Target::None)]);
    assert!(s.production.rallies.is_empty());
    let made = completed(&mut s, 2);
    assert_eq!(made.len(), 1);
    assert_eq!(s.unit(made[0].1).unwrap().target, None);
    assert_eq!(s.unit(made[0].1).unwrap().pos, p(0, 0));
}

#[test]
fn invalid_targets_and_unauthorized_or_nonbuilder_units_are_rejected_without_changes() {
    let mut s = Sim::from_setup(&setup());
    for cmd in [
        rally(&[YARD], Target::Entity(EntityId(999))),
        rally(&[LEADER], Target::Point(p(0, 100))),
        rally(&[ENEMY], Target::Point(p(0, 100))),
        rally(&[], Target::Point(p(0, 100))),
        order(
            99,
            1,
            Order::Rally {
                builders: vec![YARD],
                at: Target::Point(p(0, 100)),
            },
        ),
    ] {
        let mut control = s.clone();
        let result = s.step(&[cmd]);
        assert!(result.events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
        control.step(&[]);
        assert_eq!(s.digest(), control.digest());
        assert!(s.production.rallies.is_empty());
    }
    // Mixed groups follow ordinary order policy: eligible owned builders take it.
    accepted(&mut s, &[rally(&[ENEMY, YARD, YARD, LEADER], Target::Point(p(0, 100)))]);
    assert_eq!(
        s.production.rallies.to_rec(),
        vec![(YARD, PlayerId(0), Target::Point(p(0, 100)))]
    );
    let mut b = s.clone();
    accepted(&mut b, &[rally(&[YARD], Target::Point(p(0, 200)))]);
    accepted(&mut s, &[]);
    assert_ne!(
        s.digest(),
        b.digest(),
        "the rally must affect the digest before anything is built"
    );
}

#[test]
fn dead_targets_destroyed_builders_and_captured_builders_drop_rallies() {
    let mut s = Sim::from_setup(&setup());
    accepted(&mut s, &[rally(&[YARD], Target::Entity(LEADER))]);
    assert!(s.despawn(LEADER, None));
    assert!(s.production.rallies.is_empty());
    accepted(&mut s, &[rally(&[YARD], Target::Point(p(0, 100)))]);
    // Ownership-changing commands are not implemented yet; exercise the
    // subsystem invariant directly, without adding a capture mechanic here.
    s.units.get_mut(YARD).unwrap().owner = PlayerId(1);
    accepted(&mut s, &[]);
    assert!(s.production.rallies.is_empty());
    accepted(&mut s, &[rally(&[SECOND], Target::Point(p(0, 100)))]);
    assert!(s.despawn(SECOND, None));
    assert!(s.production.rallies.is_empty());
}

#[test]
fn snapshot_with_queued_production_keeps_both_target_kinds_and_future_digests() {
    for at in [Target::Point(p(0, 1000)), Target::Entity(LEADER)] {
        let cfg = setup();
        let mut s = Sim::from_setup(&cfg);
        let cmd = build(&s, YARD);
        accepted(&mut s, &[rally(&[YARD], at), cmd]);
        let bytes = s.snapshot();
        let mut reloaded = Sim::from_snapshot(&bytes, Some(cfg.content.clone())).unwrap();
        assert_eq!(s.digest(), reloaded.digest());
        assert_eq!(s.production.rallies.to_rec(), reloaded.production.rallies.to_rec());
        for _ in 0..350 {
            let a = s.step(&[]);
            let b = reloaded.step(&[]);
            assert_eq!(a.events, b.events);
            assert_eq!(a.view.digest, b.view.digest);
        }
        let mut legacy = bytes;
        legacy[4..6].copy_from_slice(&6_u16.to_le_bytes());
        assert!(matches!(
            Sim::from_snapshot(&legacy, Some(cfg.content)),
            Err(crate::snapshot::SnapshotError::FormatVersion { found: 6 })
        ));
    }
}

#[test]
fn rally_commands_survive_a_real_replay_round_trip() {
    let cfg = setup();
    let mut rec = Recorder::with_interval(&cfg, 1);
    let batch = [rally(&[YARD], Target::Entity(LEADER)), build(rec.sim(), YARD)];
    rec.step(&batch);
    for _ in 0..350 {
        rec.step(&[]);
    }
    let digest = rec.sim().digest();
    let replay = Replay::decode(&rec.finish().encode()).unwrap();
    assert_eq!(replay.play(cfg.content).unwrap().digest(), digest);
}

#[test]
fn rally_records_reject_noncanonical_or_impossible_rows() {
    let row = (YARD, PlayerId(0), Target::Point(p(0, 100)));
    assert!(Rallies::from_rec(&vec![row, row]).is_none());
    assert!(Rallies::from_rec(&vec![(YARD, PlayerId(0), Target::None)]).is_none());
    assert!(Rallies::from_rec(&vec![(EntityId(0), PlayerId(0), row.2)]).is_none());
    assert!(Rallies::from_rec(&vec![(YARD, PlayerId::SCRIPT, row.2)]).is_none());
    assert!(Rallies::from_rec(&vec![(YARD, PlayerId(0), Target::Entity(EntityId(0)))]).is_none());
}

#[test]
fn a_rig_rally_does_not_make_a_finished_station_move() {
    let mut s = Sim::from_setup(&setup());
    let rig = EntityId(5);
    let cmd = order(
        0,
        2,
        Order::Build {
            builder: rig,
            what: s.content().unwrap().key("fx_station").unwrap(),
            at: Some(p(-2000, 0)),
        },
    );
    accepted(&mut s, &[rally(&[rig], Target::Point(p(-2000, 1000))), cmd]);
    let made = completed(&mut s, 5);
    assert_eq!(made.len(), 1);
    let station = s.unit(made[0].1).unwrap();
    assert_eq!(station.kind, EntityKind::Station);
    assert_eq!(station.target, None);
    assert_eq!(station.pos, p(-2000, 0));
}
