//! SIM-39: the casualty model (`casualty.rs`). The numbers are the bench
//! figures of stock Armada II (a cruiser, a station-class portal, system
//! kills), set as values on typed definitions; no game file is read.

use super::casualty::Ctx;
use crate::event::SimEvent;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::view::UnitFlags;
use crate::{Sim, WorldSource};
use sta_content::{ContentDb, MapDef, OrdnanceCrewLoss, Profile, Rules, UnitDef, WeaponDef, WeaponKind};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, TeamId, Vec3};
use std::sync::Arc;

fn fx(v: f64) -> Fixed {
    Fixed::from_f64(v)
}

fn num(v: Fixed) -> f64 {
    v.to_bits() as f64 / 4294967296.0
}

/// Systems in `system_hit_points` order: engines, weapons, shield generator, life support, sensors.
#[derive(Clone)]
struct Cfg {
    on: bool,
    protection: Option<f64>,
    cfg_protection: Option<f64>,
    crew: f64,
    sane: bool,
    hit_percent: [f64; 5],
    crew_hit: f64,
    hp: f64,
    crew_loss: [f64; 5],
    ls_loss: f64,
    loss: Option<OrdnanceCrewLoss>,
    variance: f64,
}

impl Default for Cfg {
    fn default() -> Cfg {
        Cfg {
            on: true,
            protection: None,
            cfg_protection: Some(0.8),
            crew: 250.0,
            sane: false,
            hit_percent: [0.0; 5],
            crew_hit: 0.0,
            hp: 0.0,
            crew_loss: [0.0; 5],
            ls_loss: 0.0,
            loss: Some(PHASER),
            variance: 0.0,
        }
    }
}

const PHASER: OrdnanceCrewLoss = OrdnanceCrewLoss { threshold: Fixed::from_int(25), shield: Fixed::from_bits(858_993_459), hull: Fixed::from_bits(429_496_730) };

fn db(c: &Cfg) -> Arc<ContentDb> {
    let mut features = Profile::Sta2Classic.features();
    features.crew_loss_on_hit = c.on;
    let mut b = ContentDb::builder().profile(Profile::Sta2Classic).features(features);
    let mut u = UnitDef::new("ship", "craft");
    u.hull = Fixed::from_int(100_000);
    u.shields.max = Fixed::from_int(100_000);
    u.crew = fx(c.crew);
    u.maximum_crew = Some(fx(c.crew));
    u.sane_crew_loss = c.sane;
    u.system_hit_points = [fx(c.hp); 5];
    u.casualty.shield_protection = c.protection.map(fx);
    u.casualty.system_hit_percent = c.hit_percent.map(fx);
    u.casualty.crew_hit_percent = fx(c.crew_hit);
    u.casualty.system_crew_loss = c.crew_loss.map(fx);
    u.casualty.life_support_loss = fx(c.ls_loss);
    b = b.unit(u);
    let mut w = WeaponDef::new("gun", "phaser", WeaponKind::Beam);
    w.crew_loss = c.loss;
    w.damage_variance = fx(c.variance);
    let mut rules = Rules::default();
    rules.cfg.shield_damage_protection = c.cfg_protection.map(fx);
    Arc::new(b.weapon(w).build().unwrap().with_rules(rules))
}

fn sim(db: &Arc<ContentDb>, seed: u64) -> (Sim, EntityId) {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup { seed, map: Arc::new(MapDef::empty()), content: Arc::clone(db), players: vec![seat(0), seat(1)], options: GameOptions::default() });
    let at = Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: Fixed::ZERO };
    let id = sim.spawn_def(db.key("ship").unwrap(), PlayerId(1), at, (Fixed::ZERO, Fixed::ONE)).unwrap();
    (sim, id)
}

fn hit(sim: &mut Sim, db: &ContentDb, target: EntityId, damage: f64) -> SimEvent {
    let mut events = vec![SimEvent::Hit { target, by: EntityId(999), weapon: ContentKey(0), facing: 0, shield: Fixed::ZERO, hull: fx(damage), at: Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: Fixed::ZERO } }];
    let mut ctx = Ctx { subsystems: &mut sim.subsystems, rng: &mut sim.rng, switches: &sim.switches };
    super::damage::apply(&mut sim.units, db, &mut events, 0, &mut ctx);
    events.remove(0)
}

fn crew(sim: &Sim, id: EntityId) -> f64 {
    num(sim.units.get(id).unwrap().crew.unwrap()[0])
}

fn set_crew(sim: &mut Sim, id: EntityId, v: f64) {
    sim.units.get_mut(id).unwrap().crew.as_mut().unwrap()[0] = fx(v);
}

fn hull(sim: &Sim, id: EntityId) -> f64 {
    num(sim.units.get(id).unwrap().hull[0])
}

fn near(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} vs {b} (+-{tol})");
}

#[test]
fn a_non_sane_cruiser_loses_crew_by_current_crew_per_shield_hit() {
    // Phaser 70, protection 0.8 (cfg): shield part 56, over the threshold 31.
    // Bench (M): -16.5 at 250 crew, then -12.4 at 190.9, within 0.5.
    let d = db(&Cfg::default());
    let (mut s, id) = sim(&d, 1);
    hit(&mut s, &d, id, 70.0);
    near(250.0 - crew(&s, id), 16.5, 0.5);
    set_crew(&mut s, id, 190.9);
    hit(&mut s, &d, id, 70.0);
    near(190.9 - crew(&s, id), 12.4, 0.5);
    // The bleed-through (20 percent, 14) goes to the only location, the hull.
    near(100_000.0 - hull(&s, id), 28.0, 0.001);
}

#[test]
fn the_sane_method_counts_points_above_100_crew_and_the_cap_is_the_excess() {
    let d = db(&Cfg { sane: true, ..Cfg::default() });
    let (mut s, id) = sim(&d, 1);
    hit(&mut s, &d, id, 70.0); // 0.2 x 31 + 1
    near(250.0 - crew(&s, id), 7.2, 0.01);
    set_crew(&mut s, id, 100.0); // at 100 or less the percent branch
    hit(&mut s, &d, id, 70.0); // 0.2 x 100 x 31 / 100 + 1
    near(100.0 - crew(&s, id), 7.2, 0.01);
    // S = 25.5: excess 0.5, law 0.2 x 250 x 0.5 / 100 + 0.5 = 0.75, capped at 0.5.
    let d = db(&Cfg::default());
    let (mut s, id) = sim(&d, 1);
    hit(&mut s, &d, id, 25.5 / 0.8);
    near(250.0 - crew(&s, id), 0.5, 0.001);
}

#[test]
fn a_crew_location_hit_kills_by_hull_modifier_and_current_crew_without_hull_damage() {
    // Portal-class: protection 1.0, only the crew location, hull modifier 0.1.
    // Bench (M): -101.8 at 1000 crew, -25.4 at 246.7, `0.1 x crew x 100 / 100 + 1`.
    let c = Cfg { protection: Some(1.0), crew: 1000.0, crew_hit: 100.0, ..Cfg::default() };
    let d = db(&c);
    let (mut s2, id2) = sim(&d, 1);
    s2.units.get_mut(id2).unwrap().shields.now = [Fixed::ZERO; 4];
    hit(&mut s2, &d, id2, 100.0);
    near(1000.0 - crew(&s2, id2), 101.0, 1.0);
    assert_eq!(hull(&s2, id2), 100_000.0, "a crew hit does not touch the hull");
    set_crew(&mut s2, id2, 246.7);
    hit(&mut s2, &d, id2, 100.0);
    near(246.7 - crew(&s2, id2), 25.7, 0.5);
}

#[test]
fn full_protection_has_no_shield_law_and_no_bleed() {
    // A station (protection 1.0): shield hits cost no crew and no hull.
    let d = db(&Cfg { protection: Some(1.0), ..Cfg::default() });
    let (mut s, id) = sim(&d, 1);
    for _ in 0..50 {
        hit(&mut s, &d, id, 70.0);
    }
    assert_eq!(crew(&s, id), 250.0);
    assert_eq!(hull(&s, id), 100_000.0);
    // With no protection key anywhere the shield takes it all.
    let d = db(&Cfg { cfg_protection: None, ..Cfg::default() });
    let (mut s, id) = sim(&d, 1);
    hit(&mut s, &d, id, 70.0);
    assert_eq!(crew(&s, id), 250.0);
}

#[test]
fn a_shield_breaking_hit_applies_the_law_to_the_full_shield_part_and_adds_the_overflow() {
    // Bench (M): 35.7 shield left, S = 56: crew -12.1 (the law on 56), hull = bleed 14 + overflow 20.3.
    let d = db(&Cfg::default());
    let (mut s, id) = sim(&d, 1);
    s.units.get_mut(id).unwrap().shields.now[0] = fx(35.7);
    set_crew(&mut s, id, 190.9);
    hit(&mut s, &d, id, 70.0);
    near(190.9 - crew(&s, id), 12.4, 0.5);
    near(100_000.0 - hull(&s, id), 14.0 + 20.3, 0.001);
    assert_eq!(s.units.get(id).unwrap().shields.now[0], Fixed::ZERO);
    // Shields at 0: no shield law (the regenerated fraction would apply it).
    let before = crew(&s, id);
    hit(&mut s, &d, id, 70.0);
    assert_eq!(crew(&s, id), before);
    near(100_000.0 - hull(&s, id), 14.0 + 20.3 + 70.0, 0.001);
}

#[test]
fn a_system_death_costs_its_percent_of_crew_plus_one_and_overflows_to_the_hull() {
    // Bench (M): life support 3 percent + 1, weapons and sensors 2 + 1, a station's 5 + 1;
    // hull = hit - remaining system hit points (-9.6 of a 69.6 hit on 60).
    for (n, percent) in [(3usize, 3.0), (1, 2.0), (4, 2.0), (0, 5.0)] {
        let mut c = Cfg { protection: Some(1.0), crew: 115.0, hp: 60.0, loss: None, ..Cfg::default() };
        c.hit_percent[n] = 100.0;
        c.crew_loss[n] = percent;
        let d = db(&c);
        let (mut s, id) = sim(&d, 1);
        s.units.get_mut(id).unwrap().shields.now = [Fixed::ZERO; 4];
        hit(&mut s, &d, id, 69.6);
        near(115.0 - crew(&s, id), 115.0 * percent / 100.0 + 1.0, 0.01);
        near(100_000.0 - hull(&s, id), 9.6, 0.001);
        // Dead: it weighs 0 afterwards, so the next hit goes to the hull.
        hit(&mut s, &d, id, 10.0);
        near(100_000.0 - hull(&s, id), 19.6, 0.001);
    }
}

#[test]
fn a_hit_that_does_not_kill_the_system_costs_hit_points_not_hull() {
    let mut c = Cfg { protection: Some(1.0), hp: 60.0, loss: None, ..Cfg::default() };
    c.hit_percent[2] = 100.0;
    let d = db(&c);
    let (mut s, id) = sim(&d, 1);
    s.units.get_mut(id).unwrap().shields.now = [Fixed::ZERO; 4];
    hit(&mut s, &d, id, 25.0);
    assert_eq!(hull(&s, id), 100_000.0);
    assert_eq!(crew(&s, id), 250.0);
    assert_eq!(s.subsystem_state(id, crate::command::Subsystem::Shields).unwrap().1, Fixed::from_int(25));
}

#[test]
fn life_support_down_drains_percent_of_crew_plus_one_per_second() {
    // Bench (M): craft 2 percent/s + 1/s (3.3/s at 115 crew), station 5 + 1.
    for (ls, crew0, per_second) in [(2.0, 115.0, 3.3), (5.0, 400.0, 21.0)] {
        let c = Cfg { crew: crew0, ls_loss: ls, hp: 60.0, loss: None, ..Cfg::default() };
        let d = db(&c);
        let (mut s, id) = sim(&d, 1);
        s.edit_subsystem_switch(id, crate::command::Subsystem::LifeSupport, false);
        for _ in 0..30 {
            s.step(&[]);
        }
        near(crew0 - crew(&s, id), per_second, per_second * 0.05);
    }
}

#[test]
fn nothing_drains_while_life_support_is_on() {
    let d = db(&Cfg { ls_loss: 2.0, ..Cfg::default() });
    let (mut s, id) = sim(&d, 1);
    for _ in 0..60 {
        s.step(&[]);
    }
    assert_eq!(crew(&s, id), 250.0);
}

#[test]
fn crew_under_one_latches_derelict_and_one_or_more_clears_it() {
    let d = db(&Cfg { protection: Some(1.0), crew: 100.0, crew_hit: 100.0, ..Cfg::default() });
    let (mut s, id) = sim(&d, 1);
    s.units.get_mut(id).unwrap().shields.now = [Fixed::ZERO; 4];
    set_crew(&mut s, id, 1.5);
    hit(&mut s, &d, id, 100.0);
    assert_eq!(crew(&s, id), 0.0);
    let row = s.subsystems.get(id).unwrap();
    assert!(row.derelict && row.off == 0b1_1100);
    let u = s.units.get(id).unwrap();
    assert!(u.flags.contains(UnitFlags::ENGINES_OFF) && u.flags.contains(UnitFlags::WEAPONS_OFF));
    assert_eq!(u.shields.now, [Fixed::ZERO; 4]);
    // No further loss, and the shields stay at 0 through the tick.
    hit(&mut s, &d, id, 100.0);
    assert_eq!(crew(&s, id), 0.0);
    s.step(&[]);
    assert_eq!(s.units.get(id).unwrap().shields.now, [Fixed::ZERO; 4]);
    // Recrewed: the latch clears and the systems are on again.
    set_crew(&mut s, id, 5.0);
    s.step(&[]);
    assert!(s.subsystems.get(id).is_none());
    assert!(!s.units.get(id).unwrap().flags.contains(UnitFlags::ENGINES_OFF));
}

#[test]
fn a_script_immortal_unit_gets_ten_crew_instead_unless_derelictable() {
    let d = db(&Cfg { protection: Some(1.0), crew_hit: 100.0, ..Cfg::default() });
    let (mut s, id) = sim(&d, 1);
    s.units.get_mut(id).unwrap().shields.now = [Fixed::ZERO; 4];
    s.units.get_mut(id).unwrap().flags = s.units.get(id).unwrap().flags.with(UnitFlags::IMMORTAL, true);
    set_crew(&mut s, id, 1.5);
    hit(&mut s, &d, id, 100.0);
    near(crew(&s, id), 10.35, 0.01); // 0.35 left + 10
    assert!(s.subsystems.get(id).is_none());
}

#[test]
fn units_without_crew_capacity_lose_nothing_and_only_roll_the_hull() {
    let mut c = Cfg { protection: Some(1.0), crew: 0.0, crew_hit: 100.0, ..Cfg::default() };
    c.crew_hit = 100.0;
    let d = db(&c);
    let (mut s, id) = sim(&d, 1);
    s.units.get_mut(id).unwrap().shields.now = [Fixed::ZERO; 4];
    hit(&mut s, &d, id, 40.0);
    assert_eq!(hull(&s, id), 100_000.0 - 40.0, "a crew location weighs 0 without crew capacity");
    assert!(s.subsystems.get(id).is_none());
}

#[test]
fn locations_follow_their_weights() {
    let mut c = Cfg { protection: Some(1.0), crew_hit: 30.0, hp: 1.0e9, loss: None, ..Cfg::default() };
    c.hit_percent[0] = 20.0;
    let d = db(&c);
    let (mut s, id) = sim(&d, 7);
    s.units.get_mut(id).unwrap().shields.now = [Fixed::ZERO; 4];
    let (mut hull_hits, mut crew_hits, mut engine_hits) = (0, 0, 0);
    for _ in 0..2000 {
        let (h, c0, e0) = (hull(&s, id), crew(&s, id), s.subsystems.get(id).map_or(0, |r| r.damage[0]));
        hit(&mut s, &d, id, 1.0);
        if hull(&s, id) < h { hull_hits += 1 }
        if crew(&s, id) < c0 { crew_hits += 1 }
        if s.subsystems.get(id).map_or(0, |r| r.damage[0]) > e0 { engine_hits += 1 }
        set_crew(&mut s, id, 250.0);
    }
    // 20 percent engines, 30 crew, 50 hull.
    for (got, want) in [(engine_hits, 400_i32), (crew_hits, 600), (hull_hits, 1000)] {
        assert!((got - want).abs() < 120, "{got} vs {want}");
    }
}

#[test]
fn damage_variance_is_uniform_around_the_base_and_replays_from_the_seed() {
    let mut c = Cfg { protection: Some(1.0), loss: None, ..Cfg::default() };
    c.variance = 10.0;
    let d = db(&c);
    let run = |seed| {
        let (mut s, id) = sim(&d, seed);
        s.units.get_mut(id).unwrap().shields.now = [Fixed::ZERO; 4];
        let mut taken = Vec::new();
        for _ in 0..400 {
            let before = hull(&s, id);
            hit(&mut s, &d, id, 50.0);
            taken.push(before - hull(&s, id));
        }
        (taken, s.digest())
    };
    let (a, digest_a) = run(3);
    let (b, digest_b) = run(3);
    assert_eq!((a.clone(), digest_a), (b, digest_b), "same seed, same hits");
    assert!(a.iter().all(|t| (40.0..=60.0).contains(t)));
    assert!(a.iter().any(|t| *t < 45.0) && a.iter().any(|t| *t > 55.0));
    near(a.iter().sum::<f64>() / 400.0, 50.0, 2.0);
    assert_ne!(run(4).1, digest_a);
}

#[test]
fn off_without_the_feature_a_hit_is_shields_then_hull() {
    let d = db(&Cfg { on: false, crew_hit: 100.0, ..Cfg::default() });
    let (mut s, id) = sim(&d, 1);
    hit(&mut s, &d, id, 70.0);
    assert_eq!(crew(&s, id), 250.0);
    assert_eq!(hull(&s, id), 100_000.0);
    assert_eq!(s.units.get(id).unwrap().shields.now[0], Fixed::from_int(100_000) - Fixed::from_int(70));
}

#[test]
fn the_keys_reach_the_content_hash_and_the_state_snapshots_and_replays() {
    let c = Cfg { protection: Some(1.0), crew_hit: 100.0, crew: 100.0, ..Cfg::default() };
    assert_ne!(db(&c).hash(), db(&Cfg { crew_hit: 50.0, ..c.clone() }).hash());
    assert_ne!(db(&c).hash(), db(&Cfg { variance: 2.0, ..c.clone() }).hash());
    assert_ne!(db(&c).hash(), db(&Cfg { cfg_protection: Some(0.9), ..c.clone() }).hash());
    let d = db(&c);
    let (mut s, id) = sim(&d, 1);
    s.units.get_mut(id).unwrap().shields.now = [Fixed::ZERO; 4];
    set_crew(&mut s, id, 2.0);
    hit(&mut s, &d, id, 100.0);
    assert!(s.subsystems.get(id).unwrap().derelict);
    let mut copy = Sim::from_snapshot(&s.snapshot(), Some(Arc::clone(&d))).unwrap();
    assert_eq!(copy.digest(), s.digest());
    assert!(copy.subsystems.get(id).unwrap().derelict);
    s.step(&[]);
    copy.step(&[]);
    assert_eq!(copy.digest(), s.digest());
    // The latch is in the digest.
    let (mut other, id2) = sim(&d, 1);
    other.units.get_mut(id2).unwrap().shields.now = [Fixed::ZERO; 4];
    set_crew(&mut other, id2, 0.0);
    assert_ne!(other.digest(), s.digest());
}
