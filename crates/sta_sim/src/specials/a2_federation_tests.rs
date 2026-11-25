//! Command-path regressions for Armada II Federation utility specials.

use std::sync::Arc;

use sta_content::{
    A2Effect, A2SpecialDef, A2Target, ContentDb, DefRef, FederationWeaponDef, Hardpoint, MapDef,
    MoveStats, Profile, SelfDestructBlast, ShieldDef, UnitDef, WeaponDef, WeaponKind,
};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use crate::command::{Command, CommandKind, Edit, Order, Subsystem, Target};
use crate::event::SimEvent;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};

fn fixed(n: i32) -> Fixed {
    Fixed::from_int(n)
}
fn at(x: i32) -> Vec3 {
    Vec3 {
        x: fixed(x),
        ..Vec3::ZERO
    }
}

fn ship(name: &str, mounts: &[&str]) -> UnitDef {
    let mut def = UnitDef::new(name, "craft");
    def.hardpoints = mounts
        .iter()
        .map(|&weapon| Hardpoint {
            weapon: DefRef::new(weapon),
            node: "hp01".into(),
            arc: None,
        })
        .collect();
    def.movement = MoveStats::from_f32(120.0, 1.5, 1.6, 1.6, 0.3);
    def.hull = fixed(200);
    def.shields = ShieldDef {
        max: fixed(100),
        recharge: Fixed::ZERO,
    };
    def.crew = fixed(40);
    def.special_energy_max = fixed(200);
    def
}

fn special(name: &str, effect: A2Effect, target: A2Target) -> WeaponDef {
    let mut weapon = WeaponDef::new(name, name, WeaponKind::Beam);
    weapon.base.special = Some(true);
    weapon.range = fixed(600);
    weapon.a2_special = Some(A2SpecialDef {
        effect,
        target,
        duration: 60,
        flight: 60,
        max_targets: 1,
        hit_condition: 0,
        projectile: false,
    });
    weapon
}

fn world(profile: Profile) -> Sim {
    let mut gate = special("gate", A2Effect::TranswarpGate, A2Target::Point);
    gate.federation = Some(FederationWeaponDef::Gate {
        portal: "portal".into(),
    });
    let mut fuse = special(
        "fuse",
        A2Effect::GalaxyIntegrate {
            initial: fixed(40),
            final_distance: fixed(10),
        },
        A2Target::Immediate,
    );
    fuse.federation = Some(FederationWeaponDef::Integrate {
        combined: "fgalaxy".into(),
        parts: ["fsaucer".into(), "fbbridge".into()],
    });
    let mut destruct = special(
        "destruct",
        A2Effect::SelfDestruct(fixed(90)),
        A2Target::Immediate,
    );
    destruct.a2_special.as_mut().unwrap().duration = 5;
    let mut source = ship(
        "source",
        &["gate", "restore", "holo", "tractor", "destruct"],
    );
    source.self_destruct_blast = Some(SelfDestructBlast {
        damage: fixed(40),
        radius_factor: Fixed::ONE,
        rate: fixed(300),
    });
    let mut portal = UnitDef::new("portal", "wormhole_obj");
    portal.hull = fixed(100);
    let db = ContentDb::builder()
        .profile(profile)
        .features(profile.features())
        .unit(source)
        .unit(ship("target", &[]))
        .unit(ship("fgalaxy", &["split"]))
        .unit(ship("fsaucer", &["fuse"]))
        .unit(ship("fbbridge", &[]))
        .unit(portal)
        .weapon(gate)
        .weapon(special(
            "restore",
            A2Effect::RestoreShields(fixed(80)),
            A2Target::Entity,
        ))
        .weapon(special("holo", A2Effect::HoloGenerator, A2Target::Entity))
        .weapon(special(
            "tractor",
            A2Effect::TractorBeam(fixed(40)),
            A2Target::Entity,
        ))
        .weapon(destruct)
        .weapon(special(
            "split",
            A2Effect::GalaxySeparate,
            A2Target::Immediate,
        ))
        .weapon(fuse)
        .build()
        .unwrap();
    let seat = |id| PlayerSetup {
        id: PlayerId(id),
        team: TeamId(id),
        race: "test".into(),
        control: Control::Idle,
    };
    Sim::from_setup(&GameSetup {
        seed: 7,
        content: Arc::new(db),
        map: Arc::new(MapDef::empty()),
        options: GameOptions::default(),
        players: vec![seat(0), seat(1)],
    })
}

fn spawn(sim: &mut Sim, name: &str, owner: u8, x: i32) -> EntityId {
    let key = sim.content.as_ref().unwrap().key(name).unwrap();
    sim.spawn_def(key, PlayerId(owner), at(x), (Fixed::ONE, Fixed::ZERO))
        .unwrap()
}

fn cast(owner: u8, unit: EntityId, slot: u8, target: Target) -> Command {
    Command {
        player: PlayerId(owner),
        seq: 0,
        kind: CommandKind::Order(Order::Special {
            units: vec![unit],
            slot,
            target,
        }),
    }
}

#[test]
fn transwarp_gate_traversal_cancel_and_snapshot() {
    let mut sim = world(Profile::Sta2Classic);
    let source = spawn(&mut sim, "source", 0, 0);
    let opened = sim.step(&[cast(0, source, 0, Target::Point(at(300)))]);
    assert!(
        opened
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::Fired { shooter, .. } if *shooter == source))
    );
    let pairs: Vec<_> = sim.specials.a2.federation.gate_pairs(sim.tick).collect();
    assert_eq!(pairs.len(), 2);
    let entry = sim.units.get(pairs[0].0).unwrap().pos;
    let exit = sim.units.get(pairs[0].1).unwrap().pos;
    let traveler = spawn(&mut sim, "target", 0, 0);
    sim.units.get_mut(traveler).unwrap().pos = Vec3 {
        x: entry.x - fixed(10),
        ..entry
    };
    let move_to_gate = Command {
        player: PlayerId(0),
        seq: 1,
        kind: CommandKind::Order(Order::Move {
            units: vec![traveler],
            to: entry,
            queue: false,
        }),
    };
    let mut restored = Sim::from_snapshot(&sim.snapshot(), sim.content.clone()).unwrap();
    let mut traversed = false;
    for tick in 0..30 {
        let orders = if tick == 0 {
            std::slice::from_ref(&move_to_gate)
        } else {
            &[]
        };
        let a = sim.step(orders);
        let b = restored.step(orders);
        assert_eq!(a.view.digest, b.view.digest);
        traversed |= (a.view.unit(traveler).unwrap().pos.x - exit.x).abs() < fixed(100);
        if traversed {
            break;
        }
    }
    assert!(
        traversed,
        "the ordered traveler must arrive at the temporary partner"
    );
    sim.step(&[cast(0, source, 0, Target::None)]);
    assert!(
        sim.specials
            .a2
            .federation
            .gate_pairs(sim.tick)
            .next()
            .is_none()
    );
    assert!(sim.units.get(pairs[0].0).is_none());
    assert!(sim.units.get(pairs[0].1).is_none());
}

#[test]
fn restore_shields_caps_an_allied_target() {
    let mut sim = world(Profile::Sta2Classic);
    let source = spawn(&mut sim, "source", 0, 0);
    let ally = spawn(&mut sim, "target", 0, 80);
    sim.units.get_mut(ally).unwrap().shields.now[0] = fixed(30);
    let out = sim.step(&[cast(0, source, 1, Target::Entity(ally))]);
    assert!(
        out.events
            .iter()
            .any(|e| matches!(e, SimEvent::Fired { shooter, .. } if *shooter == source))
    );
    assert_eq!(out.view.unit(ally).unwrap().shields[0][0], fixed(100));
}

#[test]
fn galaxy_halves_rejoin_after_real_rendezvous() {
    let mut sim = world(Profile::Sta2Classic);
    let galaxy = spawn(&mut sim, "fgalaxy", 0, 0);
    let split = sim.step(&[cast(0, galaxy, 0, Target::None)]);
    assert!(split.view.unit(galaxy).is_none());
    let saucer = split
        .view
        .units
        .iter()
        .find(|u| sim.content.as_ref().unwrap().unit(u.def).unwrap().odf == "fsaucer")
        .unwrap()
        .id;
    let bridge = split
        .view
        .units
        .iter()
        .find(|u| sim.content.as_ref().unwrap().unit(u.def).unwrap().odf == "fbbridge")
        .unwrap()
        .id;
    // Both stock halves brake 50 units from an ordinary move target.
    // The final docking leg still has to reach the authored 10-unit clearance.
    for id in [saucer, bridge] {
        sim.units.get_mut(id).unwrap().stats.too_close_to_turn = fixed(50);
    }
    sim.units.get_mut(bridge).unwrap().pos.x = fixed(250);
    let started = sim.step(&[cast(0, saucer, 0, Target::None)]);
    assert!(
        started
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::Fired { shooter, .. } if *shooter == saucer))
    );
    assert!(sim.units.get(saucer).unwrap().target.is_some());
    assert!(sim.units.get(bridge).unwrap().target.is_some());
    let mut joined = false;
    for _ in 0..300 {
        let out = sim.step(&[]);
        if out
            .events
            .iter()
            .filter(
                |e| matches!(e, SimEvent::Replaced { old, .. } if *old == saucer || *old == bridge),
            )
            .count()
            == 2
        {
            joined = true;
            break;
        }
    }
    assert!(
        joined,
        "both halves must travel to the rendezvous before fusing"
    );
    assert!(sim.view().unit(saucer).is_none() && sim.view().unit(bridge).is_none());
    assert!(
        sim.view()
            .units
            .iter()
            .any(|u| sim.content.as_ref().unwrap().unit(u.def).unwrap().odf == "fgalaxy")
    );
}

#[test]
fn hologram_projects_then_cancels_without_replacing_source() {
    let mut sim = world(Profile::Sta2Classic);
    let source = spawn(&mut sim, "source", 0, 0);
    let enemy = spawn(&mut sim, "target", 1, 80);
    let model = sim.units.get(enemy).unwrap().def;
    let out = sim.step(&[cast(0, source, 2, Target::Entity(enemy))]);
    assert_eq!(out.view.unit(source).unwrap().owner, PlayerId(0));
    assert_eq!(out.view.appearance.projected_model(source), Some(model));
    assert_eq!(out.view.perceived_owner(source), Some(PlayerId(1)));
    let cancelled = sim.step(&[cast(0, source, 2, Target::None)]);
    assert_eq!(cancelled.view.appearance.projected_model(source), None);
    assert_eq!(cancelled.view.perceived_owner(source), Some(PlayerId(0)));
    assert_eq!(
        cancelled.view.unit(source).unwrap().def,
        sim.content.as_ref().unwrap().key("source").unwrap()
    );
}

#[test]
fn tractor_tows_only_a_disabled_target() {
    let mut sim = world(Profile::Sta2Classic);
    let source = spawn(&mut sim, "source", 0, 0);
    let target = spawn(&mut sim, "target", 1, 180);
    let rejected = sim.step(&[cast(0, source, 3, Target::Entity(target))]);
    assert!(
        rejected
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
    let disable = Command {
        player: PlayerId::SCRIPT,
        seq: 1,
        kind: CommandKind::Edit(Edit::Subsystem(target, Subsystem::Engines, false)),
    };
    sim.step(&[disable]);
    let before = sim.units.get(target).unwrap().pos.x;
    let fired = sim.step(&[cast(0, source, 3, Target::Entity(target))]);
    assert!(
        fired
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::Fired { shooter, .. } if *shooter == source))
    );
    for _ in 0..3 {
        sim.step(&[]);
    }
    assert!(
        sim.units.get(target).unwrap().pos.x < before,
        "disabled target should be pulled toward caster"
    );
    sim.step(&[cast(0, source, 3, Target::None)]);
    assert!(!super::a1::a1_special_active(&sim.view(), source, 3));
}

#[test]
fn a1_self_destruct_uses_immediate_special_and_can_cancel() {
    let mut sim = world(Profile::Sta1Classic);
    let source = spawn(&mut sim, "source", 0, 0);
    let started = sim.step(&[cast(0, source, 4, Target::None)]);
    assert!(super::a1::a1_special_active(&started.view, source, 4));
    let cancelled = sim.step(&[cast(0, source, 4, Target::None)]);
    assert!(!super::a1::a1_special_active(&cancelled.view, source, 4));
    for _ in 0..8 {
        sim.step(&[]);
    }
    assert!(sim.view().unit(source).is_some());
}
