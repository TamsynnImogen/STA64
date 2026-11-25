use std::sync::Arc;

use sta_content::{
    A1SpecialDef, A1SpecialKind as K, ContentDb, DefRef, Hardpoint, MapDef, MoveStats, Profile,
    TechTree, UnitDef, WeaponDef, WeaponKind,
};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use crate::command::{Command, CommandKind, Order, Subsystem, Target};
use crate::event::SimEvent;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::specials::{a1_special_active, a1_special_ready, a1_special_researched};
use crate::{Sim, WorldSource};

fn pos(x: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        ..Vec3::ZERO
    }
}

fn mount(name: &str) -> Hardpoint {
    Hardpoint {
        weapon: DefRef::new(name),
        node: "hp01".into(),
        arc: None,
    }
}

fn craft(name: &str, weapons: &[&str]) -> UnitDef {
    let mut u = UnitDef::new(name, "craft");
    u.hardpoints = weapons.iter().map(|s| mount(s)).collect();
    u.movement = MoveStats::from_f32(100.0, 1.5, 1.6, 1.6, 0.3);
    u.hull = Fixed::from_int(200);
    u.system_hit_points = [Fixed::from_int(100); 5];
    u.special_energy_max = Fixed::from_int(1000);
    u.sensor = Fixed::from_int(400);
    u
}

fn world(tech: Option<&str>, cases: &[K]) -> Sim {
    let mut builder = ContentDb::builder()
        .profile(Profile::Sta1Classic)
        .features(Profile::Sta1Classic.features());
    let mut weapon_names = Vec::new();
    for (i, &kind) in cases.iter().enumerate() {
        let name = format!("special{i}");
        let mut w = WeaponDef::new(
            &name,
            "test_special",
            if kind == K::PointDefense {
                WeaponKind::PointDefence
            } else {
                WeaponKind::Projectile
            },
        );
        w.range = Fixed::from_int(400);
        w.shot_speed = Fixed::from_int(6000);
        w.energy_cost = Fixed::from_int(60);
        w.refire = 2;
        w.damage = if matches!(kind, K::Mines | K::ChainPulsar) {
            Fixed::from_int(10)
        } else {
            Fixed::ZERO
        };
        w.a1_special = Some(A1SpecialDef {
            duration_ticks: 8,
            flight_ticks: 90,
            radius: Fixed::from_int(180),
            limit: 2,
            bonus_damage: Fixed::from_int(20),
            reflect_beam: true,
            reflect_missile: true,
            hit_condition: 2,
            shields_returned: Fixed::ONE / Fixed::from_int(2),
            detect_class: 3,
            ..A1SpecialDef::new(kind)
        });
        weapon_names.push(name);
        builder = builder.weapon(w);
    }
    let mut beam = WeaponDef::new("beam", "phaser", WeaponKind::Beam);
    beam.range = Fixed::from_int(350);
    beam.damage = Fixed::from_int(40);
    beam.refire = 30;
    let mut torpedo = WeaponDef::new("torpedo", "missile", WeaponKind::Projectile);
    torpedo.range = beam.range;
    torpedo.damage = beam.damage;
    torpedo.refire = beam.refire;
    torpedo.shot_speed = Fixed::from_int(90);
    let mut lab = craft("lab", &[]);
    lab.class = sta_content::ClassLabel::new("research");
    if let Some(tech) = tech {
        builder = builder.tech_tree(TechTree::parse("test", tech).0);
    }
    let db = builder
        .weapon(beam)
        .weapon(torpedo)
        .unit(craft(
            "caster",
            &weapon_names.iter().map(String::as_str).collect::<Vec<_>>(),
        ))
        .unit(craft("ship", &[]))
        .unit(craft("gun", &["beam"]))
        .unit(craft("missile_ship", &["torpedo"]))
        .unit(lab)
        .build()
        .unwrap();
    let seat = |p, team| PlayerSetup {
        id: PlayerId(p),
        team: TeamId(team),
        race: "test".into(),
        control: Control::Idle,
    };
    Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(MapDef::empty()),
        content: Arc::new(db),
        players: vec![seat(1, 1), seat(2, 2), seat(3, 1)],
        options: GameOptions::default(),
    })
}

fn spawn(sim: &mut Sim, name: &str, owner: u8, x: i32) -> EntityId {
    let key = sim.content().unwrap().key(name).unwrap();
    sim.spawn_def(key, PlayerId(owner), pos(x), (Fixed::ONE, Fixed::ZERO))
        .unwrap()
}

fn order(player: u8, order: Order) -> Command {
    Command {
        player: PlayerId(player),
        seq: 0,
        kind: CommandKind::Order(order),
    }
}

fn special(id: EntityId, slot: u8, target: Target) -> Command {
    order(
        1,
        Order::Special {
            units: vec![id],
            slot,
            target,
        },
    )
}

fn rejected(events: &[SimEvent]) -> bool {
    events
        .iter()
        .any(|ev| matches!(ev, SimEvent::Rejected { .. }))
}

#[test]
fn mines_wait_for_a_hostile_and_follow_it_only_after_research() {
    let mut sim = world(Some("special0.odf 1 lab.odf\n"), &[K::Mines]);
    let caster = spawn(&mut sim, "caster", 1, 0);
    let ally = spawn(&mut sim, "ship", 3, 60);
    let enemy = spawn(&mut sim, "ship", 2, 500);
    let v = sim.step(&[]).view;
    let db = sim.content().unwrap().clone();
    assert!(!a1_special_researched(&v, &db, caster, 0));
    let cast = special(caster, 0, Target::None);
    assert!(rejected(&sim.step(std::slice::from_ref(&cast)).events));
    let lab = spawn(&mut sim, "lab", 1, -350);
    sim.units.get_mut(lab).unwrap().progress = Some(Fixed::ONE / Fixed::from_int(2));
    assert!(rejected(&sim.step(std::slice::from_ref(&cast)).events));
    sim.units.get_mut(lab).unwrap().progress = None;
    let out = sim.step(&[cast]);
    assert!(!rejected(&out.events));
    assert_eq!(out.view.a1_specials.effects.len(), 1);
    assert_eq!(out.view.a1_specials.effects[0].target, None);
    assert!(!a1_special_ready(&out.view, &db, caster, 0));
    assert_eq!(
        out.view.unit(caster).unwrap().energy.unwrap()[0],
        Fixed::from_int(940)
    );
    for _ in 0..3 {
        sim.step(&[]);
    }
    sim.units.get_mut(enemy).unwrap().pos = pos(140);
    let out = sim.step(&[]);
    assert!(
        out.events
            .iter()
            .any(|e| matches!(e, SimEvent::Hit { target, .. } if *target == enemy))
    );
    assert_eq!(out.view.unit(ally).unwrap().hull[0], Fixed::from_int(200));
    assert!(out.view.a1_specials.effects.is_empty());
}

#[test]
fn chain_hits_each_target_once_and_increases_damage() {
    let mut sim = world(None, &[K::ChainPulsar]);
    let caster = spawn(&mut sim, "caster", 1, 0);
    let a = spawn(&mut sim, "ship", 2, 110);
    let b = spawn(&mut sim, "ship", 2, 225);
    let c = spawn(&mut sim, "ship", 2, 340);
    sim.step(&[special(caster, 0, Target::None)]);
    let mut hits = Vec::new();
    for _ in 0..4 {
        hits.extend(sim.step(&[]).events.iter().filter_map(|e| match e {
            SimEvent::Hit { target, hull, .. } => Some((*target, *hull)),
            _ => None,
        }));
    }
    assert_eq!(
        hits,
        [(b, Fixed::from_int(30))],
        "first leg hit during cast tick"
    );
    assert_eq!(sim.unit(a).unwrap().hull[0], Fixed::from_int(190));
    assert_eq!(sim.unit(c).unwrap().hull[0], Fixed::from_int(200));
}

#[test]
fn holds_affect_only_enemies_pause_movement_and_survive_saving() {
    let mut sim = world(None, &[K::EngineOverload, K::ShieldDisruptor, K::Stasis]);
    let caster = spawn(&mut sim, "caster", 1, 0);
    let enemy = spawn(&mut sim, "gun", 2, 150);
    let ally = spawn(&mut sim, "ship", 3, 170);
    sim.step(&[
        special(caster, 0, Target::None),
        order(
            2,
            Order::Move {
                units: vec![enemy],
                to: pos(500),
                queue: false,
            },
        ),
    ]);
    assert!(!sim.subsystem_state(enemy, Subsystem::Engines).unwrap().0);
    assert!(sim.subsystem_state(ally, Subsystem::Engines).unwrap().0);
    sim.step(&[special(caster, 1, Target::None)]);
    assert!(!sim.subsystem_state(enemy, Subsystem::Shields).unwrap().0);
    let out = sim.step(&[special(caster, 2, Target::Point(pos(150)))]);
    assert_eq!(out.view.a1_specials.stasis, [enemy]);
    assert!(!sim.subsystem_state(enemy, Subsystem::Weapons).unwrap().0);
    let bytes = sim.snapshot();
    let mut loaded = Sim::from_snapshot(&bytes, sim.content().cloned()).unwrap();
    let initial = sim.unit(enemy).unwrap().pos;
    for _ in 0..10 {
        let a = sim.step(&[]);
        let b = loaded.step(&[]);
        assert_eq!(a.view.digest, b.view.digest);
        assert_eq!(a.events, b.events);
    }
    assert!(sim.subsystem_state(enemy, Subsystem::Shields).unwrap().0);
    assert!(sim.subsystem_state(enemy, Subsystem::Engines).unwrap().0);
    assert!(sim.subsystem_state(enemy, Subsystem::Weapons).unwrap().0);
    assert_ne!(sim.unit(enemy).unwrap().pos, initial);
}

#[test]
fn engineering_repairs_a1_health_and_gemini_spawns_a_timed_copy() {
    let mut sim = world(None, &[K::EngineeringTeam, K::Gemini]);
    let caster = spawn(&mut sim, "caster", 1, 0);
    let ally = spawn(&mut sim, "ship", 3, 130);
    let enemy = spawn(&mut sim, "ship", 2, 210);
    sim.units.get_mut(ally).unwrap().hull[0] = Fixed::from_int(50);
    sim.edit_subsystem_damage(ally, Subsystem::Engines, Fixed::from_int(100));
    assert!(rejected(
        &sim.step(&[special(caster, 0, Target::Entity(enemy))])
            .events
    ));
    sim.step(&[special(caster, 0, Target::Entity(ally))]);
    assert_eq!(sim.unit(ally).unwrap().hull[0], Fixed::from_int(100));
    assert_eq!(
        sim.subsystem_state(ally, Subsystem::Engines),
        Some((true, Fixed::ZERO))
    );
    let out = sim.step(&[special(caster, 1, Target::Entity(ally))]);
    let copy = out
        .events
        .iter()
        .find_map(|ev| match ev {
            SimEvent::Spawned { id } => Some(*id),
            _ => None,
        })
        .unwrap();
    assert_eq!(out.view.unit(copy).unwrap().owner, PlayerId(1));
    assert_eq!(out.view.unit(copy).unwrap().hull[0], Fixed::from_int(100));
    assert!(out.view.a1_specials.copies.contains(&copy));
    for _ in 0..8 {
        sim.step(&[]);
    }
    assert!(sim.unit(copy).is_none());
    assert!(sim.unit(ally).is_some());
}

#[test]
fn reflector_returns_an_incoming_beam_and_drains_while_active() {
    let mut sim = world(None, &[K::Reflector]);
    let caster = spawn(&mut sim, "caster", 1, 0);
    let enemy = spawn(&mut sim, "gun", 2, 150);
    let out = sim.step(&[special(caster, 0, Target::None)]);
    assert!(out.events.iter().any(
        |e| matches!(e, SimEvent::Hit { target, by, .. } if *target == enemy && *by == caster)
    ));
    assert_eq!(out.view.unit(caster).unwrap().hull[0], Fixed::from_int(200));
    assert_eq!(
        out.view.unit(caster).unwrap().energy.unwrap()[0],
        Fixed::from_int(998)
    );
    assert!(a1_special_active(&out.view, caster, 0));
    assert!(!a1_special_active(
        &sim.step(&[special(caster, 0, Target::None)]).view,
        caster,
        0
    ));
}

#[test]
fn detection_reveals_a_cloak_without_turning_the_cloak_off() {
    let mut sim = world(Some("special0.odf 1 lab.odf\n"), &[K::DetectCloak]);
    let scout = spawn(&mut sim, "caster", 1, 0);
    let enemy = spawn(&mut sim, "ship", 2, 165);
    sim.units.get_mut(enemy).unwrap().flags =
        crate::view::UnitFlags::SELECTABLE | crate::view::UnitFlags::CLOAKED;
    assert!(
        !sim.step(&[])
            .view
            .visible_to(PlayerId(1))
            .iter()
            .any(|u| u.id == enemy)
    );
    spawn(&mut sim, "lab", 1, -220);
    let out = sim.step(&[special(scout, 0, Target::None)]);
    assert!(rejected(&out.events));
    assert!(
        out.view
            .visible_to(PlayerId(1))
            .iter()
            .any(|u| u.id == enemy)
    );
    assert!(
        out.view
            .unit(enemy)
            .unwrap()
            .flags
            .contains(crate::view::UnitFlags::CLOAKED)
    );
}

#[test]
fn nebula_point_defence_uses_the_a1_toggle_even_without_a2_feature_flags() {
    let mut sim = world(None, &[K::PointDefense]);
    let nebula = spawn(&mut sim, "caster", 1, 0);
    spawn(&mut sim, "missile_ship", 2, 150);
    let out = sim.step(&[special(nebula, 0, Target::None)]);
    assert!(out.view.point_defence_on(nebula, 0));
    assert!(sim.weapons.shots_in_flight() > 0);
    let out = sim.step(&[]);
    assert!(
        out.events
            .iter()
            .any(|e| matches!(e, SimEvent::Fired { shooter, .. } if *shooter == nebula))
    );
    assert_eq!(sim.weapons.shots_in_flight(), 0);
}
