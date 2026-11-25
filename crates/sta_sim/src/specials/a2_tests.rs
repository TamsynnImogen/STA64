use std::sync::Arc;

use sta_content::{
    A2Effect, A2SpecialDef, A2Target, ContentDb, DefRef, Hardpoint, MapDef, MoveStats, Profile,
    UnitDef, WeaponDef, WeaponKind,
};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use crate::command::{Command, CommandKind, Order, Subsystem, Target};
use crate::event::SimEvent;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};

fn fixed(n: i32) -> Fixed {
    Fixed::from_int(n)
}

fn pos(x: i32) -> Vec3 {
    Vec3 {
        x: fixed(x),
        ..Vec3::ZERO
    }
}

fn ship(name: &str, mounts: &[&str]) -> UnitDef {
    let mut u = UnitDef::new(name, "craft");
    u.hardpoints = mounts
        .iter()
        .map(|&name| Hardpoint {
            weapon: DefRef::new(name),
            node: "hp01".into(),
            arc: None,
        })
        .collect();
    u.movement = MoveStats::from_f32(100.0, 1.5, 1.6, 1.6, 0.3);
    u.hull = fixed(200);
    u.crew = fixed(40);
    u.special_energy_max = fixed(200);
    u.system_hit_points = [fixed(100); 5];
    u.sensor = fixed(210);
    u
}

fn special(name: &str, effect: A2Effect, target: A2Target, projectile: bool) -> WeaponDef {
    let mut w = WeaponDef::new(
        name,
        name,
        if projectile {
            WeaponKind::Projectile
        } else {
            WeaponKind::Beam
        },
    );
    w.base.special = Some(true);
    w.range = fixed(200);
    w.energy_cost = fixed(30);
    w.shot_speed = fixed(1000);
    w.refire = 4;
    w.a2_special = Some(A2SpecialDef {
        effect,
        target,
        duration: 7,
        flight: 100,
        max_targets: 1,
        hit_condition: 0,
        projectile,
    });
    w
}

fn world() -> Sim {
    let mut unknown = WeaponDef::new("unknown", "odd", WeaponKind::Beam);
    unknown.base.special = Some(true);
    unknown.damage = fixed(200);
    unknown.range = fixed(200);
    let mut caster = ship(
        "caster",
        &["inhibitor", "jammer", "recharge", "destruct", "holding"],
    );
    caster.self_destruct_blast = Some(sta_content::SelfDestructBlast {
        damage: fixed(40),
        radius_factor: Fixed::ONE,
        rate: fixed(300),
    });
    let profile = Profile::Sta2Classic;
    let db = ContentDb::builder()
        .profile(profile)
        .features(profile.features())
        .unit(caster)
        .unit(ship("target", &["unknown"]))
        .weapon(special(
            "inhibitor",
            A2Effect::Myotron,
            A2Target::Immediate,
            true,
        ))
        .weapon(special(
            "jammer",
            A2Effect::SensorJammer,
            A2Target::Entity,
            false,
        ))
        .weapon(special(
            "recharge",
            A2Effect::RestoreEnergy(fixed(60)),
            A2Target::Immediate,
            false,
        ))
        .weapon(special(
            "destruct",
            A2Effect::SelfDestruct(fixed(120)),
            A2Target::Immediate,
            false,
        ))
        .weapon(special(
            "holding",
            A2Effect::HoldingBeam {
                delay: 2,
                interval: 2,
                party: fixed(4),
            },
            A2Target::Entity,
            false,
        ))
        .weapon(unknown)
        .build()
        .unwrap();
    let seat = |p| PlayerSetup {
        id: PlayerId(p),
        team: TeamId(p),
        race: "test".into(),
        control: Control::Idle,
    };
    Sim::from_setup(&GameSetup {
        seed: 3,
        content: Arc::new(db),
        map: Arc::new(MapDef::empty()),
        options: GameOptions::default(),
        players: vec![seat(0), seat(1)],
    })
}

fn spawn(sim: &mut Sim, name: &str, player: u8, x: i32) -> EntityId {
    let key = sim.content.as_ref().unwrap().key(name).unwrap();
    sim.spawn_def(key, PlayerId(player), pos(x), (Fixed::ONE, Fixed::ZERO))
        .unwrap()
}

fn cast(player: u8, id: EntityId, slot: u8, target: Target) -> Command {
    Command {
        player: PlayerId(player),
        seq: 0,
        kind: CommandKind::Order(Order::Special {
            units: vec![id],
            slot,
            target,
        }),
    }
}

#[test]
fn a2_area_projectile_charges_once_hits_nearest_and_round_trips() {
    let mut sim = world();
    let caster = spawn(&mut sim, "caster", 0, 0);
    let near = spawn(&mut sim, "target", 1, 60);
    let far = spawn(&mut sim, "target", 1, 140);
    let ally = spawn(&mut sim, "target", 0, -55);
    let out = sim.step(&[cast(0, caster, 0, Target::None)]);
    assert!(
        out.events.iter().any(
            |e| matches!(e, SimEvent::Fired { target: Target::Entity(id), .. } if *id == near)
        )
    );
    assert_eq!(
        out.view.unit(caster).unwrap().energy.unwrap()[0],
        fixed(170)
    );
    assert!(
        sim.subsystem_state(near, Subsystem::Weapons).unwrap().0,
        "projectile still in flight"
    );
    let bytes = sim.snapshot();
    let mut restored = Sim::from_snapshot(&bytes, sim.content.clone()).unwrap();
    let mut held = false;
    for _ in 0..20 {
        let a = sim.step(&[]);
        let b = restored.step(&[]);
        assert_eq!(a.view.digest, b.view.digest);
        assert_eq!(a.events, b.events);
        held |= !sim.subsystem_state(near, Subsystem::Weapons).unwrap().0;
        assert!(sim.subsystem_state(far, Subsystem::Weapons).unwrap().0);
        assert!(sim.subsystem_state(ally, Subsystem::Weapons).unwrap().0);
    }
    assert!(held, "the arriving shot disables only the nearest enemy");
    assert!(sim.subsystem_state(near, Subsystem::Weapons).unwrap().0);
}

#[test]
fn a2_targeted_sensor_jammer_checks_orders_and_expires() {
    let mut sim = world();
    let db = sim.content.clone().unwrap();
    let caster = spawn(&mut sim, "caster", 0, 0);
    let target = spawn(&mut sim, "target", 1, 75);
    let distant = spawn(&mut sim, "target", 1, -500);
    sim.step(&[]);
    let invalid = sim.step(&[
        cast(0, caster, 1, Target::Entity(distant)),
        cast(1, caster, 1, Target::Entity(target)),
    ]);
    assert_eq!(
        invalid
            .events
            .iter()
            .filter(|e| matches!(e, SimEvent::Rejected { .. }))
            .count(),
        2
    );
    assert_eq!(
        invalid.view.unit(caster).unwrap().energy.unwrap()[0],
        fixed(200)
    );
    assert!(super::a2_special_target_ok(
        &invalid.view,
        &db,
        caster,
        1,
        Target::Entity(target)
    ));
    let first = sim.step(&[cast(0, caster, 1, Target::Entity(target))]);
    assert!(
        first
            .view
            .visible_to(PlayerId(1))
            .iter()
            .all(|u| u.id != caster)
    );
    assert!(!sim.subsystem_state(target, Subsystem::Sensors).unwrap().0);
    assert!(first.view.unit(target).unwrap().hull[0] == fixed(200));
    for _ in 0..8 {
        sim.step(&[]);
    }
    assert!(sim.subsystem_state(target, Subsystem::Sensors).unwrap().0);
    assert!(
        sim.view()
            .visible_to(PlayerId(1))
            .iter()
            .any(|u| u.id == caster)
    );
}

#[test]
fn a2_energy_recharge_caps_the_target_and_self_destruct_can_cancel() {
    let mut sim = world();
    let db = sim.content.clone().unwrap();
    let caster = spawn(&mut sim, "caster", 0, 0);
    let target = spawn(&mut sim, "target", 0, 75);
    assert!(sim.specials.set_energy(&sim.units, &db, target, fixed(170)));
    let out = sim.step(&[cast(0, caster, 2, Target::None)]);
    assert_eq!(
        out.view.unit(target).unwrap().energy.unwrap()[0],
        fixed(200)
    );
    assert_eq!(
        out.view.unit(caster).unwrap().energy.unwrap()[0],
        fixed(170)
    );
    let out = sim.step(&[cast(0, caster, 3, Target::None)]);
    assert!(super::a1_special_active(&out.view, caster, 3));
    sim.step(&[cast(0, caster, 3, Target::None)]);
    for _ in 0..8 {
        sim.step(&[]);
    }
    assert!(sim.view().unit(caster).is_some());
    sim.step(&[cast(0, caster, 3, Target::None)]);
    let bytes = sim.snapshot();
    let mut restored = Sim::from_snapshot(&bytes, sim.content.clone()).unwrap();
    let mut destroyed = 0;
    for _ in 0..8 {
        let a = sim.step(&[]);
        let b = restored.step(&[]);
        assert_eq!(a.events, b.events);
        assert_eq!(a.view.digest, b.view.digest);
        destroyed += a
            .events
            .iter()
            .filter(|e| matches!(e, SimEvent::Destroyed { id, .. } if *id == caster))
            .count();
    }
    assert_eq!(destroyed, 1);
    assert!(sim.view().unit(caster).is_none());
}

#[test]
fn a2_holding_beam_boards_and_releases_a_live_target() {
    let mut sim = world();
    let caster = spawn(&mut sim, "caster", 0, 0);
    let target = spawn(&mut sim, "target", 1, 75);
    let out = sim.step(&[cast(0, caster, 4, Target::Entity(target))]);
    assert!(super::a1_special_active(&out.view, caster, 4));
    assert!(!sim.subsystem_state(target, Subsystem::Engines).unwrap().0);

    let mut restored = Sim::from_snapshot(&sim.snapshot(), sim.content.clone()).unwrap();
    for _ in 0..4 {
        let a = sim.step(&[]);
        let b = restored.step(&[]);
        assert_eq!(a.view.digest, b.view.digest);
        assert_eq!(a.events, b.events);
    }
    assert!(sim.view().unit(caster).unwrap().crew.unwrap()[0] < fixed(40));
    assert!(sim.view().unit(target).unwrap().crew.unwrap()[0] < fixed(40));
    let out = sim.step(&[cast(0, caster, 4, Target::None)]);
    assert!(!super::a1_special_active(&out.view, caster, 4));
    assert!(sim.subsystem_state(target, Subsystem::Engines).unwrap().0);
}

#[test]
fn a2_self_destruct_blast_expands_hits_once_and_survives_save() {
    let mut sim = world();
    let caster = spawn(&mut sim, "caster", 0, 0);
    let near = spawn(&mut sim, "target", 1, 75);
    let far = spawn(&mut sim, "target", 1, 175);
    sim.step(&[cast(0, caster, 3, Target::None)]);
    while sim.view().unit(caster).is_some() {
        sim.step(&[]);
        assert!(sim.view().tick < 12);
    }
    let mut restored = Sim::from_snapshot(&sim.snapshot(), sim.content.clone()).unwrap();
    assert_eq!(sim.view().unit(near).unwrap().hull[0], fixed(200));
    for _ in 0..25 {
        let a = sim.step(&[]);
        let b = restored.step(&[]);
        assert_eq!(a.view.digest, b.view.digest);
        assert_eq!(a.events, b.events);
    }
    assert_eq!(sim.view().unit(near).unwrap().hull[0], fixed(160));
    assert_eq!(sim.view().unit(far).unwrap().hull[0], fixed(200));
}
