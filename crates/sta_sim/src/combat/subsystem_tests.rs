//! SIM-55: mission-set shields and subsystem state.

use crate::command::{Command, CommandKind, Edit, Subsystem};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::view::UnitFlags;
use crate::{Sim, SimEvent, WorldSource};
use sta_content::{ContentDb, MapDef, Profile, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use std::sync::Arc;

fn fx(v: f64) -> Fixed {
    Fixed::from_f64(v)
}

fn db() -> Arc<ContentDb> {
    let mut b = ContentDb::builder().profile(Profile::Sta2Classic).features(Profile::Sta2Classic.features());
    let mut ship = UnitDef::new("ship", "craft");
    ship.hull = Fixed::from_int(500);
    ship.shields.max = Fixed::from_int(400);
    ship.shields.recharge = fx(0.5);
    ship.system_hit_points = [Fixed::from_int(120), Fixed::from_int(80), Fixed::from_int(60), Fixed::from_int(100), Fixed::ZERO];
    b = b.unit(ship);
    Arc::new(b.build().unwrap())
}

fn world() -> (Sim, EntityId, Arc<ContentDb>) {
    let d = db();
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup { seed: 1, map: Arc::new(MapDef::empty()), content: Arc::clone(&d), players: vec![seat(0), seat(1)], options: GameOptions::default() });
    let at = Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: Fixed::ZERO };
    let id = sim.spawn_def(d.key("ship").unwrap(), PlayerId(1), at, (Fixed::ZERO, Fixed::ONE)).unwrap();
    (sim, id, d)
}

fn edit(sim: &mut Sim, from: PlayerId, e: Edit) -> bool {
    let out = sim.step(&[Command { player: from, seq: 0, kind: CommandKind::Edit(e) }]);
    !out.events.iter().any(|ev| matches!(ev, SimEvent::Rejected { .. }))
}

fn script(sim: &mut Sim, e: Edit) -> bool {
    edit(sim, PlayerId::SCRIPT, e)
}

fn now(sim: &Sim, id: EntityId) -> Fixed {
    sim.units.get(id).unwrap().shields.now[0]
}

#[test]
fn shield_percent_sets_the_pool_to_that_fraction_of_the_maximum_and_clamps() {
    let (mut sim, id, _) = world();
    assert!(script(&mut sim, Edit::ShieldPercent(id, fx(0.25))));
    assert!((now(&sim, id).to_bits() - Fixed::from_int(100).to_bits()).abs() < Fixed::from_f64(0.6).to_bits());
    assert!(script(&mut sim, Edit::ShieldPercent(id, fx(7.0))));
    assert_eq!(now(&sim, id), Fixed::from_int(400));
    assert!(script(&mut sim, Edit::ShieldPercent(id, fx(-1.0))));
    assert!(now(&sim, id) <= fx(0.5), "recharge resumes from zero by the data's rate");
    let before = now(&sim, id);
    sim.step(&[]);
    assert!(now(&sim, id) > before);
}

#[test]
fn directional_units_set_every_facing() {
    let (mut sim, id, _) = world();
    {
        let u = sim.units.get_mut(id).unwrap();
        u.shields = crate::world::Shields::full(Fixed::from_int(400), true);
        u.shields.now = [Fixed::ZERO; 4];
    }
    assert!(script(&mut sim, Edit::ShieldPercent(id, fx(0.5))));
    let s = sim.units.get(id).unwrap().shields;
    assert!(s.now.iter().all(|p| p.to_bits() >= Fixed::from_int(200).to_bits()));
}

#[test]
fn max_shields_sets_the_maximum_and_the_pool_and_recharge_caps_at_it() {
    let (mut sim, id, _) = world();
    assert!(script(&mut sim, Edit::MaxShields(id, Fixed::from_int(900))));
    let s = sim.units.get(id).unwrap().shields;
    assert_eq!((s.max, s.now[0]), (Fixed::from_int(900), Fixed::from_int(900)));
    assert!(script(&mut sim, Edit::MaxShields(id, Fixed::from_int(-5))));
    assert_eq!(sim.units.get(id).unwrap().shields.max, Fixed::ZERO);
    sim.step(&[]);
    assert_eq!(now(&sim, id), Fixed::ZERO, "a zero maximum recharges nothing");
}

#[test]
fn switches_are_recorded_and_engines_and_weapons_stay_flags() {
    let (mut sim, id, _) = world();
    for sub in [Subsystem::Shields, Subsystem::LifeSupport, Subsystem::Sensors] {
        assert_eq!(sim.subsystem_state(id, sub), Some((true, Fixed::ZERO)));
        assert!(script(&mut sim, Edit::Subsystem(id, sub, false)));
        assert!(!sim.subsystem_state(id, sub).unwrap().0);
    }
    assert!(sim.subsystem_state(id, Subsystem::Engines).unwrap().0, "others untouched");
    assert!(script(&mut sim, Edit::Subsystem(id, Subsystem::Engines, false)));
    assert!(sim.units.get(id).unwrap().flags.contains(UnitFlags::ENGINES_OFF));
    assert!(script(&mut sim, Edit::Subsystem(id, Subsystem::Shields, true)));
    assert!(sim.subsystem_state(id, Subsystem::Shields).unwrap().0);
    assert!(!sim.subsystem_state(id, Subsystem::Sensors).unwrap().0);
}

#[test]
fn damage_adds_clamps_to_the_hit_points_and_repairs_when_negative() {
    let (mut sim, id, _) = world();
    assert!(script(&mut sim, Edit::SubsystemDamage(id, Subsystem::Engines, Fixed::from_int(50))));
    assert!(script(&mut sim, Edit::SubsystemDamage(id, Subsystem::Engines, Fixed::from_int(50))));
    assert_eq!(sim.subsystem_state(id, Subsystem::Engines).unwrap().1, Fixed::from_int(100));
    assert!(script(&mut sim, Edit::SubsystemDamage(id, Subsystem::Engines, Fixed::from_int(500))));
    assert_eq!(sim.subsystem_state(id, Subsystem::Engines).unwrap().1, Fixed::from_int(120), "clamped to enginesHitPoints");
    assert!(script(&mut sim, Edit::SubsystemDamage(id, Subsystem::Engines, Fixed::from_int(-1000))));
    assert_eq!(sim.subsystem_state(id, Subsystem::Engines).unwrap().1, Fixed::ZERO);
    assert!(sim.subsystems.is_empty(), "a unit with nothing set has no row");
    assert!(!script(&mut sim, Edit::SubsystemDamage(id, Subsystem::Sensors, Fixed::ONE)), "no sensorsHitPoints");
}

#[test]
fn rejections_players_dead_and_unknown_units() {
    let (mut sim, id, _) = world();
    for e in [Edit::ShieldPercent(id, Fixed::ONE), Edit::MaxShields(id, Fixed::ONE), Edit::SubsystemDamage(id, Subsystem::Weapons, Fixed::ONE), Edit::Subsystem(id, Subsystem::Shields, false)] {
        assert!(!edit(&mut sim, PlayerId(1), e.clone()), "{e:?}");
        let mut gone = e;
        match &mut gone {
            Edit::ShieldPercent(i, _) | Edit::MaxShields(i, _) | Edit::SubsystemDamage(i, ..) | Edit::Subsystem(i, ..) => *i = EntityId(999),
            _ => {}
        }
        assert!(!script(&mut sim, gone));
    }
    assert!(script(&mut sim, Edit::Kill(id)));
    assert!(!script(&mut sim, Edit::ShieldPercent(id, Fixed::ONE)));
}

#[test]
fn heal_mends_damage_and_a_destroyed_unit_loses_its_row() {
    let (mut sim, id, _) = world();
    assert!(script(&mut sim, Edit::SubsystemDamage(id, Subsystem::Weapons, Fixed::from_int(30))));
    assert!(script(&mut sim, Edit::Subsystem(id, Subsystem::LifeSupport, false)));
    assert!(script(&mut sim, Edit::Heal(id)));
    assert_eq!(sim.subsystem_state(id, Subsystem::Weapons).unwrap().1, Fixed::ZERO);
    assert!(!sim.subsystem_state(id, Subsystem::LifeSupport).unwrap().0);
    assert!(script(&mut sim, Edit::Kill(id)));
    assert!(sim.subsystems.is_empty());
}

#[test]
fn state_is_in_both_digests_and_survives_a_snapshot_and_replay() {
    let (mut sim, id, d) = world();
    let (full, sections) = (sim.digest(), sim.section_digests());
    assert!(script(&mut sim, Edit::SubsystemDamage(id, Subsystem::Weapons, Fixed::from_int(30))));
    assert!(script(&mut sim, Edit::Subsystem(id, Subsystem::Sensors, false)));
    assert_ne!(sim.digest(), full);
    assert!(sim.section_digests().differing(&sections).contains(&"combat"));
    let mut copy = Sim::from_snapshot(&sim.snapshot(), Some(Arc::clone(&d))).unwrap();
    assert_eq!(copy.digest(), sim.digest());
    assert_eq!(copy.subsystem_state(id, Subsystem::Weapons), sim.subsystem_state(id, Subsystem::Weapons));
    for _ in 0..30 {
        sim.step(&[]);
        copy.step(&[]);
        assert_eq!(copy.digest(), sim.digest());
    }
}

#[test]
fn a_format_14_snapshot_still_loads_with_no_subsystem_state() {
    let (sim, _, d) = world();
    let mut bytes = sim.snapshot();
    // Without rows the format-17 suffix is `version 1, no rows`: two bytes.
    assert_eq!(bytes[bytes.len() - 2..], [1, 0]);
    bytes.truncate(bytes.len() - 2);
    bytes[4..6].copy_from_slice(&14u16.to_le_bytes());
    let copy = Sim::from_snapshot(&bytes, Some(d)).unwrap();
    assert_eq!(copy.digest(), sim.digest());
}

#[test]
fn corrupt_subsystem_rows_are_rejected() {
    use crate::combat::damage::{SubsystemRow, Subsystems};
    let row = |off, damage| (EntityId(1), SubsystemRow { off, damage });
    assert!(Subsystems::from_rows(vec![row(0b100, [0; 5])]).is_some());
    assert!(Subsystems::from_rows(vec![row(0b1, [0; 5])]).is_none(), "engines are a flag, not a row bit");
    assert!(Subsystems::from_rows(vec![row(0, [0, -1, 0, 0, 0])]).is_none());
    assert!(Subsystems::from_rows(vec![row(0, [0; 5])]).is_none(), "an empty row is never written");
    assert!(Subsystems::from_rows(vec![row(4, [0; 5]), row(4, [0; 5])]).is_none());
}
