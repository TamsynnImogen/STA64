//! SIM-39: crew killed by shield hits (`Features::crew_loss_on_hit`).

use crate::event::SimEvent;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};
use sta_content::{ContentDb, MapDef, OrdnanceCrewLoss, Profile, UnitDef, WeaponDef, WeaponKind};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, TeamId, Vec3};
use std::sync::Arc;

fn fx(v: f64) -> Fixed {
    Fixed::from_f64(v)
}

fn db(on: bool, loss: Option<OrdnanceCrewLoss>) -> Arc<ContentDb> {
    let mut features = Profile::Sta2Classic.features();
    features.crew_loss_on_hit = on;
    let mut b = ContentDb::builder().profile(Profile::Sta2Classic).features(features);
    for (name, sane) in [("plain", false), ("sane", true)] {
        let mut u = UnitDef::new(name, "craft");
        u.hull = Fixed::from_int(500);
        u.shields.max = Fixed::from_int(500);
        u.crew = Fixed::from_int(250);
        u.sane_crew_loss = sane;
        b = b.unit(u);
    }
    let mut w = WeaponDef::new("gun", "phaser", WeaponKind::Beam);
    w.crew_loss = loss;
    Arc::new(b.weapon(w).build().unwrap())
}

fn sim(db: &Arc<ContentDb>) -> (Sim, EntityId, EntityId) {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup { seed: 1, map: Arc::new(MapDef::empty()), content: Arc::clone(db), players: vec![seat(0), seat(1)], options: GameOptions::default() });
    let at = Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: Fixed::ZERO };
    let ahead = (Fixed::ZERO, Fixed::ONE);
    let a = sim.spawn_def(db.key("plain").unwrap(), PlayerId(1), at, ahead).unwrap();
    let b = sim.spawn_def(db.key("sane").unwrap(), PlayerId(1), at, ahead).unwrap();
    (sim, a, b)
}

/// One hit of `damage` on `target`, applied through `damage::apply`.
fn hit(sim: &mut Sim, db: &ContentDb, target: EntityId, damage: Fixed) {
    let mut events = vec![SimEvent::Hit { target, by: EntityId(999), weapon: ContentKey(0), facing: 0, shield: Fixed::ZERO, hull: damage, at: Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: Fixed::ZERO } }];
    super::damage::apply(&mut sim.units, db, &mut events, 0);
}

fn crew(sim: &Sim, id: EntityId) -> Fixed {
    sim.units.get(id).unwrap().crew.unwrap()[0]
}

const LOSS: OrdnanceCrewLoss = OrdnanceCrewLoss { threshold: Fixed::from_int(25), shield: Fixed::from_bits(858_993_459), hull: Fixed::ZERO };

#[test]
fn a_shield_hit_kills_a_percent_of_max_crew_or_points_with_the_sane_method() {
    let d = db(true, Some(LOSS));
    let weapon = d.key("gun").unwrap();
    assert_eq!(weapon, ContentKey(0), "the test hit names the only weapon");
    let (mut s, plain, sane) = sim(&d);
    // 75 shield damage: (75 - 25) x 0.2 = 10.
    hit(&mut s, &d, plain, Fixed::from_int(75));
    hit(&mut s, &d, sane, Fixed::from_int(75));
    let close = |a: Fixed, b: f64| assert!((a.to_bits() as f64 / 4294967296.0 - b).abs() < 0.001, "{a:?} vs {b}");
    close(crew(&s, plain), 250.0 - 25.0); // 10 percent of 250
    close(crew(&s, sane), 250.0 - 10.0); // 10 points
}

#[test]
fn damage_under_the_threshold_costs_nothing_and_crew_stops_at_zero() {
    let d = db(true, Some(LOSS));
    let (mut s, plain, _) = sim(&d);
    hit(&mut s, &d, plain, Fixed::from_int(25));
    assert_eq!(crew(&s, plain), Fixed::from_int(250));
    for _ in 0..20 {
        s.units.get_mut(plain).unwrap().shields.now[0] = Fixed::from_int(500);
        hit(&mut s, &d, plain, Fixed::from_int(400));
    }
    assert_eq!(crew(&s, plain), Fixed::ZERO);
}

#[test]
fn only_the_shield_part_counts() {
    let d = db(true, Some(LOSS));
    let (mut s, plain, _) = sim(&d);
    s.units.get_mut(plain).unwrap().shields.now[0] = Fixed::from_int(55);
    hit(&mut s, &d, plain, Fixed::from_int(400)); // 55 to the shield: (55 - 25) x 0.2 = 6 percent
    assert!((crew(&s, plain).to_bits() as f64 / 4294967296.0 - (250.0 - 15.0)).abs() < 0.001);
    hit(&mut s, &d, plain, Fixed::from_int(400)); // shield already down: no more
    assert!((crew(&s, plain).to_bits() as f64 / 4294967296.0 - (250.0 - 15.0)).abs() < 0.001);
}

#[test]
fn off_without_the_feature_or_the_keys_and_the_hull_modifier_is_not_applied() {
    for d in [db(false, Some(LOSS)), db(true, None), db(true, Some(OrdnanceCrewLoss { threshold: Fixed::ZERO, shield: Fixed::ZERO, hull: fx(0.5) }))] {
        let (mut s, plain, _) = sim(&d);
        hit(&mut s, &d, plain, Fixed::from_int(75));
        assert_eq!(crew(&s, plain), Fixed::from_int(250));
    }
}

#[test]
fn crew_rules_reach_the_content_hash_and_the_state_snapshots() {
    assert_ne!(db(true, Some(LOSS)).hash(), db(true, None).hash());
    let d = db(true, Some(LOSS));
    let (mut s, plain, _) = sim(&d);
    hit(&mut s, &d, plain, Fixed::from_int(75));
    let mut copy = Sim::from_snapshot(&s.snapshot(), Some(Arc::clone(&d))).unwrap();
    assert_eq!(copy.digest(), s.digest());
    assert_eq!(crew(&copy, plain), crew(&s, plain));
    s.step(&[]);
    copy.step(&[]);
    assert_eq!(copy.digest(), s.digest());
}
