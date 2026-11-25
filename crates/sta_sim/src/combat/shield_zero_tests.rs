//! #1101: authored shield pools and delays, without original-game assets.
use crate::event::SimEvent;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};
use sta_content::{ContentDb, MapDef, Profile, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use std::sync::Arc;

fn setup(profile: Profile, delay: Option<f64>) -> (Sim, EntityId) {
    let mut def = UnitDef::new("hold_ship", "craft");
    def.hull = Fixed::from_int(100);
    def.shields.max = Fixed::from_int(10);
    def.shields.recharge = Fixed::ONE;
    def.transport.shield_delay = delay.map(Fixed::from_f64);
    let db = Arc::new(
        ContentDb::builder()
            .profile(profile)
            .features(profile.features())
            .unit(def)
            .build()
            .unwrap(),
    );
    let setup = GameSetup {
        seed: 1,
        map: Arc::new(MapDef::empty()),
        content: db.clone(),
        players: vec![PlayerSetup {
            id: PlayerId(1),
            team: TeamId(1),
            race: "fx".into(),
            control: Control::Idle,
        }],
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let id = sim
        .spawn_def(
            db.key("hold_ship").unwrap(),
            PlayerId(1),
            Vec3::ZERO,
            (Fixed::ZERO, Fixed::ONE),
        )
        .unwrap();
    (sim, id)
}

fn hit(sim: &mut Sim, id: EntityId, damage: i32) {
    let db = sim.content.clone().unwrap();
    let mut events = vec![SimEvent::Hit {
        target: id,
        by: id,
        weapon: sta_content::ContentKey::NONE,
        facing: 0,
        shield: Fixed::ZERO,
        hull: Fixed::from_int(damage),
        at: Vec3::ZERO,
    }];
    let mut ctx = super::casualty::Ctx { upgrades: &Default::default(), subsystems: &mut sim.subsystems,
        rng: &mut sim.rng,
        switches: &sim.switches,
        colonies: &mut sim.colonies,
    };
    super::damage::apply(
        &mut sim.units,
        &db,
        &mut events,
        0,
        &mut ctx,
        &mut sim.zero_shield_holds,
        sim.tick,
        &sim.specials,
    );
}

fn pass(sim: &mut Sim, tick: u64) {
    sim.tick = tick;
    let db = sim.content.clone().unwrap();
    super::damage::recharge(
        &mut sim.units,
        &db,
        &sim.switches,
        &sim.subsystems,
        &sim.transports,
        &mut sim.zero_shield_holds,
        tick,
        &sim.specials,
    );
}

fn shield(sim: &Sim, id: EntityId) -> Fixed {
    sim.unit(id).unwrap().shields.now[0]
}

#[test]
fn emptying_hit_holds_exactly_two_seconds_and_a_later_hit_restarts_it() {
    let (mut sim, id) = setup(Profile::Sta2Classic, Some(2.0));
    sim.tick = 10;
    hit(&mut sim, id, 10);
    assert_eq!(shield(&sim, id), Fixed::ZERO);
    assert_eq!(sim.zero_shield_holds.until(id, 0), 70);
    pass(&mut sim, 69);
    assert_eq!(shield(&sim, id), Fixed::ZERO);
    hit(&mut sim, id, 1);
    assert_eq!(sim.zero_shield_holds.until(id, 0), 129);
    pass(&mut sim, 128);
    assert_eq!(shield(&sim, id), Fixed::ZERO);
    pass(&mut sim, 129);
    assert_eq!(shield(&sim, id), Fixed::ONE);
    assert!(sim.zero_shield_holds.is_empty());
}

#[test]
fn positive_pool_never_holds_and_missing_key_uses_transport_fallback() {
    let (mut sim, id) = setup(Profile::Sta2Classic, None);
    hit(&mut sim, id, 3);
    assert!(sim.zero_shield_holds.is_empty());
    pass(&mut sim, 1);
    assert_eq!(shield(&sim, id), Fixed::from_int(8));
    hit(&mut sim, id, 8);
    assert_eq!(sim.zero_shield_holds.until(id, 0), 91);
}

#[test]
fn generator_and_transport_holds_combine_with_zero_hold_and_snapshot_restores_it() {
    let (mut sim, id) = setup(Profile::Sta2Classic, Some(1.0));
    hit(&mut sim, id, 10);
    let bytes = sim.snapshot();
    let mut sim = Sim::from_snapshot(&bytes, sim.content.clone()).unwrap();
    assert_eq!(sim.snapshot(), bytes);
    sim.transports.hold(id, super::boarding::TARGET_HOLD, 40);
    pass(&mut sim, 30);
    assert_eq!(shield(&sim, id), Fixed::ZERO);
    pass(&mut sim, 31);
    assert_eq!(shield(&sim, id), Fixed::ZERO);
    sim.transports.expire(40);
    pass(&mut sim, 40);
    assert_eq!(shield(&sim, id), Fixed::ONE);

    // A switched-off generator still blocks a fresh empty-pool hold.
    sim.units.get_mut(id).unwrap().shields.now[0] = Fixed::ZERO;
    hit(&mut sim, id, 1);
    sim.subsystems.edit(id, |r| r.off |= 0b100);
    pass(&mut sim, 70);
    assert_eq!(shield(&sim, id), Fixed::ZERO);
    sim.subsystems.edit(id, |r| r.off = 0);
    pass(&mut sim, 71);
    assert_eq!(shield(&sim, id), Fixed::ONE);
}

#[test]
fn switch_off_preserves_old_recharge_and_creates_no_hold() {
    for profile in [
        Profile::Native,
        Profile::Fo4,
        Profile::Fo3,
        Profile::Sta1Classic,
    ] {
        let (mut sim, id) = setup(profile, Some(2.0));
        let before = sim.digest();
        hit(&mut sim, id, 10);
        assert!(sim.zero_shield_holds.is_empty());
        pass(&mut sim, 1);
        assert_eq!(shield(&sim, id), Fixed::ONE, "{profile:?}");
        assert_ne!(sim.digest(), before);
    }
}

#[test]
fn one_tick_hold_round_trips_at_the_step_boundary_then_recharges() {
    let (mut sim, id) = setup(Profile::Sta2Classic, Some(1.0 / 30.0));
    hit(&mut sim, id, 10);
    assert_eq!(sim.zero_shield_holds.until(id, 0), 1);
    WorldSource::step(&mut sim, &[]);
    assert_eq!(sim.tick, 1);
    assert_eq!(shield(&sim, id), Fixed::ZERO);
    let bytes = sim.snapshot();
    let mut loaded = Sim::from_snapshot(&bytes, sim.content.clone()).unwrap();
    assert_eq!(loaded.snapshot(), bytes);
    WorldSource::step(&mut loaded, &[]);
    assert_eq!(shield(&loaded, id), Fixed::ONE);
    assert!(loaded.zero_shield_holds.is_empty());
}

#[test]
fn an_explicit_refill_during_the_hold_can_be_saved_and_resumes_recharge() {
    let (mut sim, id) = setup(Profile::Sta2Classic, Some(2.0));
    hit(&mut sim, id, 10);
    sim.units.get_mut(id).unwrap().shields.now[0] = Fixed::from_int(4);
    let bytes = sim.snapshot();
    let mut loaded = Sim::from_snapshot(&bytes, sim.content.clone()).unwrap();
    assert_eq!(loaded.snapshot(), bytes);
    WorldSource::step(&mut loaded, &[]);
    assert_eq!(shield(&loaded, id), Fixed::from_int(5));
    assert!(loaded.zero_shield_holds.is_empty());
}
