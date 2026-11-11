//! Focused regressions for privileged per-instance shield writes.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, MapObject, Profile};

use crate::command::{Command, CommandKind, Edit};
use crate::replay::{Recorder, Replay};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::view::UnitFlags;
use crate::{EntityId, Fixed, PlayerId, Sim, SimEvent, TeamId, Vec3, WorldSource};

fn at(z: i32) -> Vec3 {
    Vec3::from_f32([2000.0, 0.0, z as f32])
}

fn setup(profile: Profile, target_z: i32) -> GameSetup {
    setup_with_recharge(profile, target_z, None)
}

fn setup_with_recharge(profile: Profile, target_z: i32, recharge: Option<Fixed>) -> GameSetup {
    let fake = sta_content::fake::db();
    let mut features = fake.features().clone();
    features.directional_shields = profile == Profile::Native;
    let mut builder = ContentDb::builder().profile(profile).features(features);
    for unit in fake.units() {
        let mut unit = unit.clone();
        if let Some(recharge) = recharge {
            unit.shields.recharge = recharge;
        }
        builder = builder.unit(unit);
    }
    for weapon in fake.weapons() {
        builder = builder.weapon(weapon.clone());
    }
    let mut map = MapDef::empty();
    map.objects = [("fx_cruiser", 0, 2000), ("fx_frigate", 1, target_z)]
        .into_iter()
        .map(|(odf, owner, z)| MapObject {
            odf: odf.into(),
            owner: Some(owner),
            pos: at(z),
            heading: (Fixed::ZERO, Fixed::ONE),
            label: None,
        })
        .collect();
    GameSetup {
        seed: 89,
        map: Arc::new(map),
        content: Arc::new(builder.build().unwrap()),
        players: [0, 1]
            .map(|id| PlayerSetup {
                id: PlayerId(id),
                team: TeamId(id),
                race: "fx".into(),
                control: Control::Idle,
            })
            .to_vec(),
        options: GameOptions::default(),
    }
}

#[test]
fn recharge_near_fixed_maximum_clamps_without_wrapping() {
    let mut sim = Sim::from_setup(&setup_with_recharge(
        Profile::Native,
        10_000,
        Some(Fixed::from_f64(0.3)),
    ));
    let (_, id) = ids(&sim);
    let maximum = Fixed::from_bits(i64::MAX);
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::MaxShields(id, maximum)));
    assert!(sim.apply_edit(
        PlayerId::SCRIPT,
        &Edit::ShieldFraction(id, Fixed::from_bits(Fixed::ONE.to_bits() - 1))
    ));
    sim.step(&[]);
    sim.step(&[]);
    assert_eq!(sim.unit(id).unwrap().shields.now, [maximum; 4]);
}

fn ids(sim: &Sim) -> (EntityId, EntityId) {
    let shooter = sim.view().units.iter().find(|u| u.owner == PlayerId(0)).unwrap().id;
    let target = sim.view().units.iter().find(|u| u.owner == PlayerId(1)).unwrap().id;
    (shooter, target)
}

fn edit(player: PlayerId, seq: u32, value: Edit) -> Command {
    Command {
        player,
        seq,
        kind: CommandKind::Edit(value),
    }
}

fn no_rejection(events: &[SimEvent]) {
    assert!(
        !events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })),
        "{events:?}"
    );
}

#[test]
fn max_and_fraction_are_absolute_ordered_per_instance_writes() {
    for profile in [Profile::Native, Profile::Sta1Classic] {
        let mut sim = Sim::from_setup(&setup(profile, 10_000));
        let (other, target) = ids(&sim);
        let other_before = sim.unit(other).unwrap().clone();
        let target_before = sim.unit(target).unwrap().clone();
        let directional = target_before.shields.directional;
        assert_eq!(directional, profile == Profile::Native);
        let expected_pools = if directional { 4 } else { 1 };
        if !directional {
            sim.units.get_mut(target).unwrap().shields.now[1..].copy_from_slice(&[
                Fixed::from_int(11),
                Fixed::from_int(12),
                Fixed::from_int(13),
            ]);
        }
        let inactive_before = sim.unit(target).unwrap().shields.now[1..].to_vec();
        let before = sim.digest();
        let out = sim.step(&[
            edit(PlayerId::SCRIPT, 2, Edit::ShieldFraction(target, Fixed::from_f64(0.25))),
            edit(PlayerId::SCRIPT, 0, Edit::MaxShields(target, Fixed::from_int(200))),
            edit(PlayerId::SCRIPT, 1, Edit::ShieldFraction(target, Fixed::from_f64(0.5))),
        ]);
        no_rejection(&out.events);
        assert!(
            !out.events
                .iter()
                .any(|e| matches!(e, SimEvent::Hit { .. } | SimEvent::ShieldsDown { .. }))
        );
        let unit = sim.unit(target).unwrap();
        assert_eq!(unit.shields.max, Fixed::from_int(200));
        // Last sorted SCRIPT edit uses the new maximum, not previous current.
        assert_eq!(
            unit.shields.now[..expected_pools].to_vec(),
            vec![Fixed::from_f64(50.25); expected_pools]
        );
        if !directional {
            assert_eq!(unit.shields.now[1..].to_vec(), inactive_before);
        }
        assert_eq!(unit.hull, target_before.hull);
        assert_eq!(unit.flags, target_before.flags);
        assert_eq!(unit.shields.directional, directional);
        assert_eq!(sim.unit(other).unwrap(), &other_before);
        assert_eq!(
            out.view.unit(target).unwrap().shields[0],
            [Fixed::from_f64(50.25), Fixed::from_int(200)]
        );
        assert_ne!(sim.digest(), before);
        assert_eq!(
            sim.content().unwrap().unit(unit.def).unwrap().shields.max,
            target_before.shields.max
        );
    }
}

#[test]
fn zero_negative_overcharge_and_max_fill_retain_raw_values() {
    let mut sim = Sim::from_setup(&setup(Profile::Native, 10_000));
    let (_, id) = ids(&sim);
    for (value, expected) in [
        (Fixed::ZERO, Fixed::ZERO),
        (Fixed::from_f64(-0.5), Fixed::from_int(-75)),
        (Fixed::from_int(2), Fixed::from_int(300)),
    ] {
        let before = sim.digest();
        assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::ShieldFraction(id, value)));
        assert_eq!(sim.unit(id).unwrap().shields.now, [expected; 4]);
        assert_ne!(sim.digest(), before, "shield state is hashed without a tick advance");
    }
    for maximum in [Fixed::ZERO, Fixed::from_int(-10), Fixed::from_int(125)] {
        assert!(sim.apply_edit(PlayerId::BENCH, &Edit::MaxShields(id, maximum)));
        assert_eq!(sim.unit(id).unwrap().shields.max, maximum);
        assert_eq!(sim.unit(id).unwrap().shields.now, [maximum; 4]);
    }
    let filled = sim.digest();
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::ShieldFraction(id, Fixed::ONE)));
    assert_eq!(sim.unit(id).unwrap().shields.now, [Fixed::from_int(125); 4]);
    assert_eq!(sim.digest(), filled, "an equal-value write adds no shadow state");
}

#[test]
fn privilege_missing_dying_and_checked_overflow_reject_atomically() {
    let mut sim = Sim::from_setup(&setup(Profile::Native, 10_000));
    let (_, id) = ids(&sim);
    for value in [Edit::ShieldFraction(id, Fixed::ONE), Edit::MaxShields(id, Fixed::ONE)] {
        let before = sim.snapshot();
        assert!(!sim.apply_edit(PlayerId(0), &value));
        assert_eq!(sim.snapshot(), before);
    }
    for bad_id in [EntityId::NONE, EntityId(u32::MAX)] {
        for value in [
            Edit::ShieldFraction(bad_id, Fixed::ONE),
            Edit::MaxShields(bad_id, Fixed::ONE),
        ] {
            let before = sim.snapshot();
            assert!(!sim.apply_edit(PlayerId::SCRIPT, &value));
            assert_eq!(sim.snapshot(), before);
        }
    }
    let flags = sim.unit(id).unwrap().flags;
    sim.units.get_mut(id).unwrap().flags = flags.with(UnitFlags::DYING, true);
    for value in [Edit::ShieldFraction(id, Fixed::ONE), Edit::MaxShields(id, Fixed::ONE)] {
        let before = sim.snapshot();
        assert!(!sim.apply_edit(PlayerId::BENCH, &value));
        assert_eq!(sim.snapshot(), before);
    }
    sim.units.get_mut(id).unwrap().flags = flags;

    // The endpoint is representable; multiplying it by two is not.
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::MaxShields(id, Fixed::from_bits(i64::MAX))));
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::ShieldFraction(id, Fixed::ONE)));
    assert_eq!(sim.unit(id).unwrap().shields.now, [Fixed::from_bits(i64::MAX); 4]);
    let before = sim.snapshot();
    assert!(!sim.apply_edit(PlayerId::SCRIPT, &Edit::ShieldFraction(id, Fixed::from_int(2))));
    assert_eq!(sim.snapshot(), before);
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::MaxShields(id, Fixed::from_bits(i64::MIN))));
    let before = sim.snapshot();
    assert!(!sim.apply_edit(PlayerId::SCRIPT, &Edit::ShieldFraction(id, -Fixed::ONE)));
    assert_eq!(sim.snapshot(), before);
}

#[test]
fn generic_core_accepts_live_zero_hull_and_construction_progress() {
    let mut sim = Sim::from_setup(&setup(Profile::Native, 10_000));
    let (_, id) = ids(&sim);
    sim.units.get_mut(id).unwrap().hull[0] = Fixed::ZERO;
    sim.units.get_mut(id).unwrap().progress = Some(Fixed::from_f64(0.5));
    let hull = sim.unit(id).unwrap().hull;
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::MaxShields(id, Fixed::from_int(77))));
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::ShieldFraction(id, Fixed::from_f64(0.5))));
    let unit = sim.unit(id).unwrap();
    assert_eq!(unit.hull, hull);
    assert_eq!(unit.progress, Some(Fixed::from_f64(0.5)));
    assert_eq!(unit.shields.now, [Fixed::from_f64(38.5); 4]);
}

#[test]
fn normal_recharge_damage_and_heal_consume_the_mutated_instance() {
    let mut sim = Sim::from_setup(&setup(Profile::Native, 2350));
    let (shooter, target) = ids(&sim);
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::MaxShields(target, Fixed::from_int(100))));
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::ShieldFraction(target, Fixed::ZERO)));
    let out = sim.step(&[]);
    let beam = out
        .events
        .iter()
        .find(|e| matches!(e, SimEvent::Hit { target: id, by, .. } if *id == target && *by == shooter))
        .unwrap();
    let SimEvent::Hit { shield, hull, .. } = beam else {
        unreachable!()
    };
    assert_eq!(*shield, Fixed::from_f64(0.25));
    assert_eq!(*hull, Fixed::from_f64(19.75));
    assert_eq!(sim.unit(target).unwrap().shields.max, Fixed::from_int(100));
    assert!(sim.unit(target).unwrap().hull[0] < Fixed::from_int(300));
    assert!(sim.apply_edit(PlayerId::SCRIPT, &Edit::Heal(target)));
    assert_eq!(sim.unit(target).unwrap().hull[0], Fixed::from_int(300));
    assert_eq!(sim.unit(target).unwrap().shields.now, [Fixed::from_int(100); 4]);
}

#[test]
fn new_tags_snapshot_and_replay_preserve_shield_edits() {
    let id = EntityId(1);
    for (tag, value) in [
        (20, Edit::ForceOntoMap { id, enabled: true }),
        (21, Edit::PerceivedTeam(id, PlayerId(1))),
        (22, Edit::ShieldFraction(id, Fixed::ONE)),
        (23, Edit::MaxShields(id, Fixed::ONE)),
    ] {
        let bytes = postcard::to_allocvec(&value).unwrap();
        assert_eq!(bytes[0], tag);
        assert_eq!(postcard::from_bytes::<Edit>(&bytes).unwrap(), value);
    }
    let setup = setup(Profile::Native, 10_000);
    let mut sim = Sim::from_setup(&setup);
    let mut recorder = Recorder::with_interval(&setup, 1);
    let (_, target) = ids(&sim);
    for batch in [
        vec![edit(
            PlayerId::SCRIPT,
            0,
            Edit::MaxShields(target, Fixed::from_int(120)),
        )],
        vec![edit(
            PlayerId::SCRIPT,
            1,
            Edit::ShieldFraction(target, Fixed::from_f64(0.5)),
        )],
        vec![edit(
            PlayerId::BENCH,
            2,
            Edit::ShieldFraction(target, Fixed::from_int(-1)),
        )],
    ] {
        let expected = sim.step(&batch);
        let recorded = recorder.step(&batch);
        no_rejection(&expected.events);
        assert_eq!(expected.events, recorded.events);
        assert_eq!(expected.view.digest, recorded.view.digest);
    }
    let expected_shields = recorder.sim().unit(target).unwrap().shields.clone();
    let saved = sim.snapshot();
    let mut loaded = Sim::from_snapshot(&saved, sim.content().cloned()).unwrap();
    assert_eq!(loaded.snapshot(), saved);
    for _ in 0..5 {
        let original = sim.step(&[]);
        let resumed = loaded.step(&[]);
        assert_eq!(original.events, resumed.events);
        assert_eq!(original.view.digest, resumed.view.digest);
    }
    let replay = Replay::decode(&recorder.finish().encode()).unwrap();
    // Replay ends before the five continuation ticks.
    let replayed = replay.play(setup.content).unwrap();
    assert_eq!(replayed.unit(target).unwrap().shields, expected_shields);
}
