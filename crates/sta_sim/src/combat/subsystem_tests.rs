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
    ship.crew = Fixed::from_int(250);
    ship.race = "fx".into();
    ship.system_hit_points = [Fixed::from_int(120), Fixed::from_int(80), Fixed::from_int(60), Fixed::from_int(100), Fixed::ZERO];
    b = b.unit(ship);
    Arc::new(b.build().unwrap())
}

fn setup(d: &Arc<ContentDb>) -> GameSetup {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    GameSetup { seed: 1, map: Arc::new(MapDef::empty()), content: Arc::clone(d), players: vec![seat(0), seat(1)], options: GameOptions::default() }
}

pub(crate) fn world() -> (Sim, EntityId, Arc<ContentDb>) {
    let d = db();
    let mut sim = Sim::from_setup(&setup(&d));
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
fn state_is_in_both_digests_and_survives_a_snapshot() {
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
fn corrupt_subsystem_rows_are_rejected() {
    use crate::combat::damage::{SubsystemRow, Subsystems};
    let row = |off, damage| (EntityId(1), SubsystemRow { off, damage });
    assert!(Subsystems::from_rows(vec![row(0b100, [0; 5])]).is_some());
    assert!(Subsystems::from_rows(vec![row(0b1, [0; 5])]).is_none(), "engines are a flag, not a row bit");
    assert!(Subsystems::from_rows(vec![row(0, [0, -1, 0, 0, 0])]).is_none());
    assert!(Subsystems::from_rows(vec![row(0, [0; 5])]).is_none(), "an empty row is never written");
    assert!(Subsystems::from_rows(vec![row(4, [0; 5]), row(4, [0; 5])]).is_none());
}

#[test]
fn a_huge_maximum_is_capped_and_recharge_cannot_wrap() {
    let (mut sim, id, _) = world();
    assert!(script(&mut sim, Edit::MaxShields(id, Fixed::from_bits(i64::MAX))));
    assert_eq!(sim.units.get(id).unwrap().shields.max, Fixed::from_int(1_000_000_000));
    for _ in 0..5 {
        sim.step(&[]);
    }
    assert!(now(&sim, id) > Fixed::ZERO);
}

#[test]
fn damage_above_the_hit_points_in_a_snapshot_is_corrupt() {
    use crate::combat::damage::SubsystemRow;
    let (mut sim, id, d) = world();
    assert!(script(&mut sim, Edit::SubsystemDamage(id, Subsystem::Weapons, Fixed::from_int(30))));
    let ok = sim.snapshot();
    assert!(Sim::from_snapshot(&ok, Some(Arc::clone(&d))).is_ok());
    let mut bad = sim.clone();
    bad.subsystems = crate::combat::damage::Subsystems::from_rows(vec![(id, SubsystemRow { off: 0, damage: [0, Fixed::from_int(81).to_bits(), 0, 0, 0] })]).unwrap();
    assert!(Sim::from_snapshot(&bad.snapshot(), Some(d)).is_err());
}

// SIM-54: personnel edits.

#[test]
fn unit_crew_clamps_to_the_pool_and_rejects_without_one() {
    let (mut sim, id, _) = world();
    assert!(script(&mut sim, Edit::UnitCrew(id, Fixed::from_int(40))));
    assert_eq!(sim.units.get(id).unwrap().crew.unwrap()[0], Fixed::from_int(40));
    assert!(script(&mut sim, Edit::UnitCrew(id, Fixed::from_int(9999))));
    assert_eq!(sim.units.get(id).unwrap().crew.unwrap(), [Fixed::from_int(250); 2], "clamped to the maximum crew");
    assert!(script(&mut sim, Edit::UnitCrew(id, Fixed::from_int(-3))));
    assert_eq!(sim.units.get(id).unwrap().crew.unwrap()[0], Fixed::ZERO);
    assert!(!edit(&mut sim, PlayerId(1), Edit::UnitCrew(id, Fixed::ONE)), "players cannot");
    sim.units.get_mut(id).unwrap().crew = None;
    assert!(!script(&mut sim, Edit::UnitCrew(id, Fixed::ONE)));
}

#[test]
fn team_crew_and_max_officers_need_an_initialized_seat_and_are_floored_and_capped() {
    let (mut sim, _, _) = world();
    let p = PlayerId(0);
    assert!(!script(&mut sim, Edit::TeamCrew(PlayerId(9), Fixed::ONE)), "no such seat");
    assert!(!script(&mut sim, Edit::MaxOfficers(PlayerId(9), Fixed::ONE)), "no such seat");
    // A seat with no stock or limit yet takes the script's value (a raw write in the original).
    assert!(script(&mut sim, Edit::MaxOfficers(p, Fixed::ONE)));
    assert_eq!(sim.economy.officer_limit(p), Some(Fixed::ONE));
    assert!(script(&mut sim, Edit::TeamCrew(p, Fixed::from_int(2))));
    assert_eq!(sim.economy.crew_of(p), Some(Fixed::from_int(2)));
    sim.economy.set_personnel(p, Some(Fixed::from_int(100)), Some(Fixed::from_int(30)));
    assert!(script(&mut sim, Edit::TeamCrew(p, Fixed::from_int(5000))));
    assert_eq!(sim.economy.crew_of(p), Some(Fixed::from_int(5000)), "above the capacity is allowed");
    assert!(script(&mut sim, Edit::TeamCrew(p, Fixed::from_int(-4))));
    assert_eq!(sim.economy.crew_of(p), Some(Fixed::ZERO));
    assert!(script(&mut sim, Edit::MaxOfficers(p, Fixed::from_int(199_999))));
    assert_eq!(sim.economy.officer_limit(p), Some(Fixed::from_int(199_999)));
    assert!(script(&mut sim, Edit::MaxOfficers(p, Fixed::from_bits(i64::MAX))));
    assert_eq!(sim.economy.officer_limit(p), Some(Fixed::from_int(1_000_000_000)));
    let mut copy = Sim::from_snapshot(&sim.snapshot(), Some(sim.content().unwrap().clone())).unwrap();
    assert_eq!(copy.digest(), sim.digest());
    sim.step(&[]);
    copy.step(&[]);
    assert_eq!(copy.digest(), sim.digest());
}

/// Record the edits through a `Recorder`, encode, decode and play: same digest.
fn replay_equal(edits: impl Fn(EntityId) -> Vec<Edit>) -> (Sim, EntityId) {
    let d = db();
    let setup = setup(&d);
    let mut rec = crate::replay::Recorder::with_interval(&setup, 1);
    let cmd = |seq, e| Command { player: PlayerId::SCRIPT, seq, kind: CommandKind::Edit(e) };
    let at = Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: Fixed::ZERO };
    rec.step(&[cmd(0, Edit::Spawn { odf: d.key("ship").unwrap(), player: PlayerId(1), pos: at, heading: (Fixed::ZERO, Fixed::ONE) })]);
    let id = rec.sim().view().units[0].id;
    for (n, e) in edits(id).into_iter().enumerate() {
        rec.step(&[cmd(n as u32 + 1, e)]);
    }
    for _ in 0..10 {
        rec.step(&[]);
    }
    let digest = rec.sim().digest();
    let replay = crate::replay::Replay::decode(&rec.finish().encode()).unwrap();
    let played = replay.play(d).unwrap();
    assert_eq!(played.digest(), digest);
    (played, id)
}

#[test]
fn shield_and_subsystem_edits_record_and_replay_to_the_same_digest() {
    let (played, id) = replay_equal(|id| vec![
        Edit::ShieldPercent(id, fx(0.5)),
        Edit::MaxShields(id, Fixed::from_int(900)),
        Edit::SubsystemDamage(id, Subsystem::Weapons, Fixed::from_int(30)),
        Edit::Subsystem(id, Subsystem::Sensors, false),
    ]);
    assert_eq!(played.subsystem_state(id, Subsystem::Weapons).unwrap().1, Fixed::from_int(30));
    assert_eq!(played.units.get(id).unwrap().shields.max, Fixed::from_int(900));
}

#[test]
fn personnel_edits_record_and_replay_to_the_same_digest() {
    let (played, id) = replay_equal(|id| vec![
        Edit::UnitCrew(id, Fixed::from_int(40)),
        // No initialized seat in this setup: rejected, deterministically, in both runs.
        Edit::MaxOfficers(PlayerId(1), Fixed::from_int(600)),
        Edit::TeamCrew(PlayerId(1), Fixed::from_int(600)),
    ]);
    assert_eq!(played.units.get(id).unwrap().crew.unwrap()[0], Fixed::from_int(40));
}

#[test]
fn each_edit_changes_its_own_section_digest() {
    let (mut sim, id, _) = world();
    sim.economy.set_personnel(PlayerId(0), Some(Fixed::from_int(100)), Some(Fixed::from_int(30)));
    for (what, e, section) in [
        ("shield percent", Edit::ShieldPercent(id, fx(0.1)), "combat"),
        ("subsystem damage", Edit::SubsystemDamage(id, Subsystem::Engines, Fixed::from_int(5)), "combat"),
        ("unit crew", Edit::UnitCrew(id, Fixed::from_int(7)), "combat"),
        ("team crew", Edit::TeamCrew(PlayerId(0), Fixed::from_int(7)), "economy"),
        ("max officers", Edit::MaxOfficers(PlayerId(0), Fixed::from_int(7)), "economy"),
    ] {
        let (before, full) = (sim.section_digests(), sim.digest());
        // The edit applies on the tick; a tick on its own also moves the roster and the
        // rng, so compare against an identical sim that ticked without the edit.
        let mut control = sim.clone();
        control.step(&[]);
        assert!(script(&mut sim, e), "{what}");
        let differing = control.section_digests().differing(&sim.section_digests());
        assert_eq!(differing, [section], "{what}");
        assert_ne!(sim.digest(), control.digest(), "{what}");
        let _ = (before, full);
    }
}

#[test]
fn section_rows_itemise_the_subsystem_state() {
    let (mut sim, id, _) = world();
    assert!(script(&mut sim, Edit::SubsystemDamage(id, Subsystem::Weapons, Fixed::from_int(30))));
    let rows = sim.section_rows("combat").unwrap();
    assert!(rows.iter().any(|r| r.entity == Some(id) && r.field.starts_with("subsystems")), "{rows:?}");
}

// SIM-54: per-unit attribute edits (max hull, hull value, max crew, max shields, name, race).

fn hull(sim: &Sim, id: EntityId) -> [Fixed; 2] {
    sim.units.get(id).unwrap().hull
}

#[test]
fn max_hull_is_per_unit_keeps_the_current_hull_and_ignores_a_negative_value() {
    let (mut sim, id, d) = world();
    let other = sim.spawn_def(d.key("ship").unwrap(), PlayerId(1), Vec3::ZERO, (Fixed::ZERO, Fixed::ONE)).unwrap();
    assert!(script(&mut sim, Edit::MaxHull(id, Fixed::from_int(1))));
    assert_eq!(hull(&sim, id), [Fixed::from_int(500), Fixed::ONE], "current hull not clamped");
    assert_eq!(hull(&sim, other), [Fixed::from_int(500); 2], "only this unit");
    // The relay clock: read back, add one, write.
    for n in 2..=1800 {
        assert!(script(&mut sim, Edit::MaxHull(id, Fixed::from_int(n))));
    }
    assert_eq!(hull(&sim, id)[1], Fixed::from_int(1800));
    assert!(script(&mut sim, Edit::MaxHull(id, Fixed::from_int(-5))), "accepted");
    assert_eq!(hull(&sim, id)[1], Fixed::from_int(1800), "and ignored");
    assert!(script(&mut sim, Edit::MaxHull(id, Fixed::from_int(0))));
    assert_eq!(hull(&sim, id)[1], Fixed::ZERO);
    assert!(!edit(&mut sim, PlayerId(1), Edit::MaxHull(id, Fixed::ONE)), "players cannot");
    assert!(!script(&mut sim, Edit::MaxHull(EntityId(999), Fixed::ONE)));
    // Heal restores to the unit's own maximum.
    assert!(script(&mut sim, Edit::MaxHull(id, Fixed::from_int(900))));
    assert!(script(&mut sim, Edit::Heal(id)));
    assert_eq!(hull(&sim, id), [Fixed::from_int(900); 2]);
}

#[test]
fn hull_value_is_the_current_hull_without_an_upper_clamp_and_never_kills() {
    let (mut sim, id, _) = world();
    assert!(script(&mut sim, Edit::HullValue(id, Fixed::from_int(20_000))));
    assert_eq!(hull(&sim, id), [Fixed::from_int(20_000), Fixed::from_int(500)]);
    assert!(script(&mut sim, Edit::HullValue(id, Fixed::ZERO)));
    assert!(sim.units.get(id).is_some(), "zero does not destroy");
    assert!(hull(&sim, id)[0] > Fixed::ZERO);
    assert!(script(&mut sim, Edit::Immortal(id, true)));
    assert!(script(&mut sim, Edit::HullValue(id, Fixed::from_int(300))));
    assert!(script(&mut sim, Edit::HullValue(id, Fixed::from_int(100))));
    assert_eq!(hull(&sim, id)[0], Fixed::from_int(300), "an immortal unit ignores a lower value");
}

#[test]
fn max_crew_clamps_the_crew_down_only_and_max_shields_keeps_the_pools() {
    let (mut sim, id, _) = world();
    assert!(script(&mut sim, Edit::MaxCrew(id, Fixed::from_int(20))));
    assert_eq!(sim.units.get(id).unwrap().crew, Some([Fixed::from_int(20); 2]));
    assert!(script(&mut sim, Edit::MaxCrew(id, Fixed::from_int(400))));
    assert_eq!(sim.units.get(id).unwrap().crew, Some([Fixed::from_int(20), Fixed::from_int(400)]), "never raised");
    assert!(script(&mut sim, Edit::UnitCrew(id, Fixed::from_int(999))));
    assert_eq!(sim.units.get(id).unwrap().crew.unwrap()[0], Fixed::from_int(400), "SetCrew clamps to the current maximum");
    assert!(script(&mut sim, Edit::MaxShieldsKeep(id, Fixed::from_int(360))));
    let u = sim.units.get(id).unwrap();
    assert_eq!((u.shields.max, u.shields.now[0]), (Fixed::from_int(360), Fixed::from_int(400)));
    assert!(script(&mut sim, Edit::MaxShields(id, Fixed::from_int(100))));
    assert_eq!(sim.units.get(id).unwrap().shields.now[0], Fixed::from_int(100), "A2 SetMaxShields sets both");
}

#[test]
fn names_and_races_are_per_unit_validated_viewed_and_survive_snapshot_and_death() {
    let (mut sim, id, d) = world();
    let before = sim.digest();
    assert!(script(&mut sim, Edit::Name(id, "Relay Station".into())));
    assert!(!script(&mut sim, Edit::UnitRace(id, "nobody".into())), "unknown race refused");
    assert!(!script(&mut sim, Edit::UnitRace(id, "FX".into())), "case-sensitive");
    assert!(script(&mut sim, Edit::UnitRace(id, "fx".into())));
    assert!(script(&mut sim, Edit::PlayerRace(PlayerId(1), "fx".into())));
    assert!(!script(&mut sim, Edit::PlayerRace(PlayerId(5), "fx".into())), "no such seat");
    let v = sim.view();
    assert_eq!(v.attrs.names, vec![(id, "Relay Station".to_string())]);
    assert_eq!(v.attrs.races, vec![(id, "fx".to_string())]);
    assert_eq!(v.attrs.player_races, vec![(PlayerId(1), "fx".to_string())]);
    assert_ne!(sim.digest(), before);
    let mut copy = Sim::from_snapshot(&sim.snapshot(), Some(Arc::clone(&d))).unwrap();
    assert_eq!(copy.digest(), sim.digest());
    assert_eq!(copy.view().attrs, v.attrs);
    copy.step(&[]);
    sim.step(&[]);
    assert_eq!(copy.digest(), sim.digest());
    assert!(script(&mut sim, Edit::Kill(id)));
    assert!(sim.view().attrs.names.is_empty() && sim.view().attrs.races.is_empty(), "rows go with the unit");
    let long = "é".repeat(200);
    let (mut sim, id, _) = world();
    assert!(script(&mut sim, Edit::Name(id, long)));
    assert_eq!(sim.view().attrs.names[0].1.len(), 126, "cut at a character boundary");
}
