//! SIM-68: Armada I hull regeneration (`a1_compat`: the only pool is `maxHealth`, held in the
//! hull, regenerating at `shields.recharge` per tick). Authored numbers, no stock values.

use crate::command::{Command, CommandKind, Edit, Subsystem};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};
use sta_content::{ContentDb, MapDef, Profile, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use std::sync::Arc;

fn fx(v: f64) -> Fixed {
    Fixed::from_f64(v)
}

fn db(profile: Profile) -> Arc<ContentDb> {
    let mut b = ContentDb::builder().profile(profile).features(profile.features());
    let mut ship = UnitDef::new("ship", "craft");
    ship.hull = Fixed::from_int(300);
    ship.shields.recharge = fx(1.5);
    ship.crew = Fixed::from_int(100);
    ship.race = "fx".into();
    ship.system_hit_points = [Fixed::from_int(40), Fixed::from_int(40), Fixed::from_int(30), Fixed::from_int(40), Fixed::ZERO];
    b = b.unit(ship);
    Arc::new(b.build().unwrap())
}

fn world(profile: Profile) -> (Sim, EntityId) {
    let d = db(profile);
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let setup = GameSetup { seed: 1, map: Arc::new(MapDef::empty()), content: Arc::clone(&d), players: vec![seat(0), seat(1)], options: GameOptions::default() };
    let mut sim = Sim::from_setup(&setup);
    let at = Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: Fixed::ZERO };
    let id = sim.spawn_def(d.key("ship").unwrap(), PlayerId(1), at, (Fixed::ZERO, Fixed::ONE)).unwrap();
    (sim, id)
}

fn script(sim: &mut Sim, e: Edit) {
    sim.step(&[Command { player: PlayerId::SCRIPT, seq: 0, kind: CommandKind::Edit(e) }]);
}

fn hull(sim: &Sim, id: EntityId) -> Fixed {
    sim.units.get(id).unwrap().hull[0]
}

fn damaged(profile: Profile) -> (Sim, EntityId) {
    let (mut sim, id) = world(profile);
    sim.units.get_mut(id).unwrap().hull[0] = Fixed::from_int(100);
    (sim, id)
}

/// One recharge pass on its own (no other system moves the hull).
fn pass(sim: &mut Sim) {
    let db = sim.content.clone().unwrap();
    super::damage::recharge(&mut sim.units, &db, &sim.switches, &sim.subsystems, &sim.transports);
}

#[test]
fn the_hull_regains_the_rate_per_tick_up_to_its_maximum_and_no_further() {
    let (mut sim, id) = damaged(Profile::Sta1Classic);
    pass(&mut sim);
    pass(&mut sim);
    assert_eq!(hull(&sim, id), fx(103.0));
    for _ in 0..200 {
        pass(&mut sim);
    }
    assert_eq!(hull(&sim, id), Fixed::from_int(300));
    // An overcharged hull (SetShieldPercent above 1) stays where it is.
    sim.units.get_mut(id).unwrap().hull[0] = Fixed::from_int(450);
    pass(&mut sim);
    assert_eq!(hull(&sim, id), Fixed::from_int(450));
    // The shield pool is untouched: the unit has none.
    assert_eq!(sim.units.get(id).unwrap().shields.now[0], Fixed::ZERO);
}

#[test]
fn nothing_regenerates_with_the_generator_off_or_destroyed() {
    let (mut sim, id) = damaged(Profile::Sta1Classic);
    script(&mut sim, Edit::Subsystem(id, Subsystem::Shields, false));
    let before = hull(&sim, id);
    pass(&mut sim);
    assert_eq!(hull(&sim, id), before);
    // `script` steps the whole sim once, after applying the edit: that step regenerates too.
    script(&mut sim, Edit::Subsystem(id, Subsystem::Shields, true));
    assert_eq!(hull(&sim, id), before + fx(1.5), "switching it back on resumes at once");
    // Destroyed: damage at its hit points, the switch left on.
    script(&mut sim, Edit::SubsystemDamage(id, Subsystem::Shields, Fixed::from_int(30)));
    let before = hull(&sim, id);
    pass(&mut sim);
    assert_eq!(hull(&sim, id), before);
    // Damaged but not destroyed still regenerates.
    script(&mut sim, Edit::SubsystemDamage(id, Subsystem::Shields, Fixed::from_int(-5)));
    assert_eq!(hull(&sim, id), before + fx(1.5));
}

#[test]
fn nothing_regenerates_while_derelict_without_repair_or_at_zero() {
    let (mut sim, id) = damaged(Profile::Sta1Classic);
    sim.subsystems.edit(id, |r| r.derelict = true);
    pass(&mut sim);
    assert_eq!(hull(&sim, id), Fixed::from_int(100));
    sim.subsystems.edit(id, |r| r.derelict = false);
    script(&mut sim, Edit::CraftCanRepair(id, false));
    pass(&mut sim);
    assert_eq!(hull(&sim, id), Fixed::from_int(100));
    script(&mut sim, Edit::CraftCanRepair(id, true));
    sim.units.get_mut(id).unwrap().hull[0] = Fixed::ZERO;
    pass(&mut sim);
    assert_eq!(hull(&sim, id), Fixed::ZERO);
}

#[test]
fn nothing_regenerates_during_a_boarding_hold() {
    let (mut sim, id) = damaged(Profile::Sta1Classic);
    sim.transports.hold(id, super::boarding::TARGET_HOLD, 1000);
    pass(&mut sim);
    assert_eq!(hull(&sim, id), Fixed::from_int(100));
}

/// The recharge pass runs before the hits of a step: a hull of 1 is lifted to 2.5 first, so a
/// hit of 2 leaves it and a hit of 2.5 kills it.
#[test]
fn regeneration_comes_before_the_hit_in_one_step() {
    for (damage, survives) in [(2.0, true), (2.5, false)] {
        let (mut sim, id) = damaged(Profile::Sta1Classic);
        sim.units.get_mut(id).unwrap().hull[0] = Fixed::ONE;
        pass(&mut sim);
        let db = sim.content.clone().unwrap();
        let mut events = vec![crate::SimEvent::Hit {
            target: id,
            by: id,
            weapon: sta_content::ContentKey::NONE,
            facing: 0,
            shield: Fixed::ZERO,
            hull: fx(damage),
            at: Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: Fixed::ZERO },
        }];
        let mut ctx = super::casualty::Ctx { subsystems: &mut sim.subsystems, rng: &mut sim.rng, switches: &sim.switches, colonies: &mut sim.colonies };
        let killed = super::damage::apply(&mut sim.units, &db, &mut events, 0, &mut ctx);
        assert_eq!(killed.is_empty(), survives, "hit of {damage}");
    }
}

#[test]
fn a_maximum_near_the_fixed_range_cannot_wrap() {
    let (mut sim, id) = damaged(Profile::Sta1Classic);
    let top = Fixed::from_bits(i64::MAX - 5);
    {
        let u = sim.units.get_mut(id).unwrap();
        u.hull = [top - Fixed::ONE, top];
    }
    pass(&mut sim);
    assert_eq!(hull(&sim, id), top);
}

#[test]
fn other_profiles_never_regenerate_the_hull() {
    for profile in [Profile::Native, Profile::Sta2Classic, Profile::Fo4, Profile::Fo3] {
        let (mut sim, id) = damaged(profile);
        for _ in 0..5 {
            pass(&mut sim);
        }
        assert_eq!(hull(&sim, id), Fixed::from_int(100), "{profile:?}");
    }
}

#[test]
fn the_whole_step_regenerates_the_hull_and_a_damaged_state_digest_is_pinned() {
    let (mut sim, id) = damaged(Profile::Sta1Classic);
    script(&mut sim, Edit::SubsystemDamage(id, Subsystem::Weapons, Fixed::from_int(7)));
    for _ in 0..4 {
        sim.step(&[]);
    }
    assert_eq!(hull(&sim, id), fx(107.5), "five ticks of the rate, the first one with the edit");
    assert_eq!(sim.digest(), PINNED, "{:#x}", sim.digest());
}

/// Pinned with the authored content above.
const PINNED: u64 = 0xc15e3ed348d251a5;
