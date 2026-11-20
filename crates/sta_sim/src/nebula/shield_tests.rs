//! SIM-75 slice 2: shields after leaving a shield-disabling nebula, on authored data only.

use sta_content::Profile;
use sta_core::{EntityId, Fixed};

use super::shields::RETURN_DELAY;
use super::tests::{content_in, neb, put, run, sim_with, v};
use crate::command::{Command, CommandKind, Edit, Subsystem};
use crate::{Sim, WorldSource};
use sta_core::PlayerId;

const NEAR: i32 = 1000;
const FAR: i32 = 5000;

fn entry() -> Fixed {
    Fixed::from_int(150)
}

fn shields(s: &Sim, id: EntityId) -> Fixed {
    s.unit(id).unwrap().shields.now[0]
}

fn world_in(profile: Option<Profile>, regions: &[(&str, i32)]) -> (Sim, EntityId) {
    world_crew(profile, regions, false)
}

fn world_crew(profile: Option<Profile>, regions: &[(&str, i32)], crew_loss: bool) -> (Sim, EntityId) {
    let regions = regions.iter().map(|(odf, x)| neb(odf, v(*x, 1000), 300)).collect();
    let mut s = sim_with(regions, content_in(crew_loss, profile));
    let id = put(&mut s, "fx_cruiser", 1, v(NEAR, 1000));
    s.units.get_mut(id).unwrap().shields.now = [entry(); 4];
    (s, id)
}

fn world() -> (Sim, EntityId) {
    world_in(None, &[("fx_still", NEAR)])
}

fn teleport(s: &mut Sim, id: EntityId, x: i32) {
    s.units.get_mut(id).unwrap().pos = v(x, 1000);
}

/// Steps until the shields are above 0; returns the steps taken and the value then.
fn until_back(s: &mut Sim, id: EntityId, limit: u64) -> Option<(u64, Fixed)> {
    (1..=limit).find_map(|n| {
        s.step(&[]);
        (shields(s, id) > Fixed::ZERO).then(|| (n, shields(s, id)))
    })
}

#[test]
fn the_entry_value_returns_in_one_step_after_the_delay_and_not_before() {
    let (mut s, id) = world();
    run(&mut s, 5);
    assert_eq!(shields(&s, id), Fixed::ZERO, "held at 0 inside");
    teleport(&mut s, id, FAR);
    // The shields stay at 0 for the whole delay, not recharging.
    run(&mut s, RETURN_DELAY as u32);
    assert_eq!(shields(&s, id), Fixed::ZERO, "no restore before the delay, no recharge from 0");
    // One step later: the entry value itself, not a recharge step above 0.
    s.step(&[]);
    assert_eq!(shields(&s, id), entry());
    assert!(s.nebula_shields.is_empty(), "the state is spent");
}

#[test]
fn the_value_on_entry_is_what_returns_even_when_it_was_not_full() {
    // Entry value, not the maximum, for a craft that entered below it: measured (stock Armada II 1.1, five samples, two classes).
    let (mut s, id) = world();
    s.units.get_mut(id).unwrap().shields.now = [Fixed::from_int(37), Fixed::from_int(11), Fixed::from_int(5), Fixed::from_int(2)];
    run(&mut s, 3);
    teleport(&mut s, id, FAR);
    let (n, now) = until_back(&mut s, id, 2 * RETURN_DELAY).unwrap();
    assert_eq!(n, RETURN_DELAY + 1);
    assert_eq!(s.unit(id).unwrap().shields.now, [Fixed::from_int(37), Fixed::from_int(11), Fixed::from_int(5), Fixed::from_int(2)]);
    assert_eq!(now, Fixed::from_int(37));
}

#[test]
fn back_inside_during_the_delay_keeps_the_entry_value_and_restarts_the_delay() {
    let (mut s, id) = world();
    run(&mut s, 3);
    teleport(&mut s, id, FAR);
    run(&mut s, RETURN_DELAY as u32 / 2);
    teleport(&mut s, id, NEAR);
    run(&mut s, RETURN_DELAY as u32);
    assert_eq!(shields(&s, id), Fixed::ZERO, "held again inside, even past the first delay");
    teleport(&mut s, id, FAR);
    run(&mut s, RETURN_DELAY as u32);
    assert_eq!(shields(&s, id), Fixed::ZERO, "the delay counts from the second leaving");
    s.step(&[]);
    assert_eq!(shields(&s, id), entry(), "the value of the first entry, not 0");
}

#[test]
fn entering_with_the_shields_at_zero_gives_nothing_back() {
    let (mut s, id) = world();
    s.units.get_mut(id).unwrap().shields.now = [Fixed::ZERO; 4];
    run(&mut s, 3);
    teleport(&mut s, id, FAR);
    run(&mut s, RETURN_DELAY as u32 + 1);
    assert_eq!(shields(&s, id), Fixed::ZERO);
    assert!(s.nebula_shields.is_empty());
    run(&mut s, 30);
    assert!(shields(&s, id) > Fixed::ZERO, "the ordinary recharge goes on from there");
}

#[test]
fn a_switched_off_generator_or_a_derelict_gets_nothing_back() {
    for derelict in [false, true] {
        let (mut s, id) = world_crew(None, &[("fx_still", NEAR)], derelict);
        run(&mut s, 3);
        teleport(&mut s, id, FAR);
        run(&mut s, RETURN_DELAY as u32 / 2);
        if derelict {
            let mut ctx = crate::combat::casualty::Ctx { subsystems: &mut s.subsystems, rng: &mut s.rng, switches: &s.switches, colonies: &mut s.colonies };
            crate::combat::casualty::kill_crew(s.units.get_mut(id).unwrap(), &mut ctx, Fixed::from_int(10_000));
            assert!(s.subsystems.get(id).unwrap().derelict, "the empty crew latches the derelict");
        } else {
            s.step(&[Command { player: PlayerId::SCRIPT, seq: 0, kind: CommandKind::Edit(Edit::Subsystem(id, Subsystem::Shields, false)) }]);
        }
        run(&mut s, RETURN_DELAY as u32);
        assert_eq!(shields(&s, id), Fixed::ZERO, "derelict {derelict}");
        assert!(s.nebula_shields.is_empty(), "the remembered value is dropped (derelict {derelict})");
    }
}

#[test]
fn a_craft_destroyed_in_the_delay_takes_its_state_with_it_and_an_owner_change_keeps_it() {
    let (mut s, id) = world();
    run(&mut s, 3);
    teleport(&mut s, id, FAR);
    run(&mut s, 5);
    assert!(!s.nebula_shields.is_empty());
    assert!(s.despawn(id, None));
    assert!(s.nebula_shields.is_empty());

    let (mut s, id) = world();
    run(&mut s, 3);
    teleport(&mut s, id, FAR);
    run(&mut s, 5);
    s.units.get_mut(id).unwrap().owner = PlayerId(2);
    run(&mut s, RETURN_DELAY as u32 - 4);
    assert_eq!(shields(&s, id), entry(), "restored for the new owner");
}

#[test]
fn leaving_one_of_two_overlapping_kinds_is_not_leaving() {
    let (mut s, id) = world_in(None, &[("fx_still", NEAR), ("fx_still2", NEAR + 500)]);
    // In both, then only in the second: never out.
    run(&mut s, 3);
    teleport(&mut s, id, NEAR + 500);
    run(&mut s, 2 * RETURN_DELAY as u32);
    assert_eq!(shields(&s, id), Fixed::ZERO);
    teleport(&mut s, id, FAR);
    run(&mut s, RETURN_DELAY as u32);
    assert_eq!(shields(&s, id), Fixed::ZERO);
    s.step(&[]);
    assert_eq!(shields(&s, id), entry(), "the value from the first entry");
}

#[test]
fn armada_one_keeps_recharging_from_zero_and_keeps_no_state() {
    let (mut s, id) = world_in(Some(Profile::Sta1Classic), &[("fx_still", NEAR)]);
    run(&mut s, 3);
    assert!(s.nebula_shields.is_empty());
    teleport(&mut s, id, FAR);
    let (n, now) = until_back(&mut s, id, 10).unwrap();
    assert_eq!(n, 1, "recharging at once");
    assert!(now < entry(), "from 0 in small steps");
}

#[test]
fn a_craft_in_no_such_nebula_has_no_state() {
    let mut s = sim_with(vec![], content_in(false, None));
    put(&mut s, "fx_cruiser", 1, v(NEAR, 1000));
    run(&mut s, 5);
    assert!(s.nebula_shields.is_empty());
}

#[test]
fn a_save_in_the_middle_of_the_delay_loads_exactly_and_finishes_the_same() {
    let (mut s, id) = world();
    run(&mut s, 3);
    teleport(&mut s, id, FAR);
    run(&mut s, RETURN_DELAY as u32 / 2);
    let bytes = s.snapshot();
    let mut back = Sim::from_snapshot(&bytes, s.content().cloned()).unwrap();
    assert_eq!(back.snapshot(), bytes, "save, load, save");
    assert_eq!(back.digest(), s.digest());
    assert!(!back.nebula_shields.is_empty());
    for _ in 0..RETURN_DELAY + 2 {
        s.step(&[]);
        back.step(&[]);
        assert_eq!(back.digest(), s.digest());
    }
    assert_eq!(shields(&back, id), shields(&s, id));
    assert!(shields(&s, id) >= entry(), "back at the entry value, recharging from there");
}

#[test]
fn a_save_inside_the_nebula_round_trips_and_the_record_is_in_the_combat_digest() {
    let (mut s, _) = world();
    run(&mut s, 3);
    let bytes = s.snapshot();
    let back = Sim::from_snapshot(&bytes, s.content().cloned()).unwrap();
    assert_eq!(back.snapshot(), bytes);
    assert_eq!(back.digest(), s.digest());
    // The same tick, differing only in the record.
    let mut bare = s.clone();
    bare.nebula_shields = Default::default();
    assert_eq!(s.section_digests().differing(&bare.section_digests()), ["combat"]);
}

#[test]
fn a_damaged_generator_is_not_switched_off_by_the_nebula_so_the_shields_come_back() {
    // Today's behaviour. The original sometimes gave nothing back; a generator below full hit points
    // that stays off is the likely cause (read from the executable, static; not measured): TODO.
    let (mut s, id) = world();
    s.step(&[Command { player: PlayerId::SCRIPT, seq: 0, kind: CommandKind::Edit(Edit::SubsystemDamage(id, Subsystem::Shields, Fixed::from_int(30))) }]);
    s.units.get_mut(id).unwrap().shields.now = [entry(); 4];
    run(&mut s, 3);
    teleport(&mut s, id, FAR);
    run(&mut s, RETURN_DELAY as u32 + 1);
    assert!(shields(&s, id) >= entry());
    assert!(s.subsystem_state(id, Subsystem::Shields).unwrap().0, "the generator is still on");
}
