//! SIM-74 `crew_weapon_delay`: reloads slow down at yellow and red crew status.

use std::sync::Arc;

use sta_content::{ContentDb, RaceRepair};
use sta_core::{Fixed, PlayerId, TeamId, Vec3};

use super::weapons::crew_delay_factor;
use crate::command::{Command, CommandKind, Order};
use crate::event::SimEvent;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};

fn f(v: f64) -> Fixed {
    Fixed::from_f64(v)
}

#[test]
fn the_factor_follows_the_crew_status_and_the_engine_defaults() {
    let none = RaceRepair::default();
    let crew = |now: f64| [f(now), f(100.0)];
    assert_eq!(crew_delay_factor(none, crew(100.0)), Fixed::ONE);
    assert_eq!(crew_delay_factor(none, crew(51.0)), Fixed::ONE);
    assert_eq!(crew_delay_factor(none, crew(50.0)), f(3.0), "yellow is inclusive (0.5)");
    assert_eq!(crew_delay_factor(none, crew(25.0)), f(9.0), "red replaces yellow (0.25)");
    let race = RaceRepair { weapon_yellow: Some(f(2.0)), weapon_red: Some(f(4.0)), crew_yellow_status: Some(f(0.75)), ..RaceRepair::default() };
    assert_eq!(crew_delay_factor(race, crew(70.0)), f(2.0));
    assert_eq!(crew_delay_factor(race, crew(20.0)), f(4.0));
    assert_eq!(crew_delay_factor(none, [f(0.0), Fixed::ZERO]), Fixed::ONE, "no crew maximum: never slowed");
}

/// Shots of a cruiser with `crew` of its maximum at a frigate over 60 s.
fn shots(switch: bool, crew_fraction: f64, affects: Option<bool>) -> usize {
    let fake = sta_content::fake::db();
    let mut features = fake.features().clone();
    features.crew_weapon_delay = switch;
    let mut b = ContentDb::builder().profile(fake.profile()).features(features);
    for w in fake.weapons() {
        let mut w = w.clone();
        w.base.crew_affects_weapon_delay = affects;
        b = b.weapon(w);
    }
    for u in fake.units() {
        b = b.unit(u.clone());
    }
    let db = Arc::new(b.build().unwrap());
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let setup = GameSetup { seed: 7, map: Arc::new(sta_content::MapDef::empty()), content: db.clone(), players: vec![seat(1), seat(2)], options: GameOptions::default() };
    let mut sim = Sim::from_setup(&setup);
    let v = |z: i32| Vec3 { x: Fixed::from_int(2000), y: Fixed::ZERO, z: Fixed::from_int(z) };
    let shooter = sim.spawn_def(db.key("fx_cruiser").unwrap(), PlayerId(1), v(2000), (Fixed::ZERO, Fixed::ONE)).unwrap();
    let target = sim.spawn_def(db.key("fx_station").unwrap(), PlayerId(2), v(2300), (Fixed::ZERO, Fixed::ONE)).unwrap();
    let u = sim.units.get_mut(shooter).unwrap();
    let max = u.crew.map_or(f(100.0), |c| c[1]).max(f(1.0));
    u.crew = Some([max * f(crew_fraction), max]);
    let order = Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Attack { units: vec![shooter], target }) };
    let mut fired = 0;
    for t in 0..60 * sta_core::TICK_HZ {
        let cmds = if t == 0 { vec![order.clone()] } else { Vec::new() };
        fired += sim.step(&cmds).events.iter().filter(|e| matches!(e, SimEvent::Fired { shooter: s, .. } if *s == shooter)).count();
    }
    fired
}

#[test]
fn a_red_crew_reloads_slower_only_under_the_switch_and_the_weapon_key() {
    let full = shots(true, 1.0, None);
    let red = shots(true, 0.1, None);
    assert!(full > 6, "{full} shots");
    assert!(red * 4 < full, "red status: {red} shots against {full}");
    assert_eq!(shots(false, 0.1, None), shots(false, 1.0, None), "switch off: crew does not matter");
    assert_eq!(shots(true, 0.1, Some(false)), full, "`crewAffectsWeaponDelay = 0`: not slowed");
}

#[test]
fn the_new_race_keys_are_digested_only_when_set() {
    let fake = sta_content::fake::db();
    let with = |r: RaceRepair| fake.clone().with_rules(sta_content::Rules::default().with_race_repairs(vec![("fx".into(), r)])).hash();
    let base = RaceRepair { strength: Some(f(1.0)), ..RaceRepair::default() };
    let keyed = RaceRepair { weapon_red: Some(f(5.0)), ..base };
    assert_ne!(with(base), with(keyed));
}
