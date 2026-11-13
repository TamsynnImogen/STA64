//! SIM-36: targets of opportunity under an attack order, and construction
//! ships holding fire, per profile switch.

use std::sync::Arc;

use sta_content::{ContentDb, MoveStats, Profile, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use crate::command::{Command, CommandKind, Edit, Order};
use crate::event::{SimEvent, Target};
use crate::movement::approach_tests;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

const AHEAD: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);

/// Who the shooter (id 1) fired at over `ticks`.
fn targets_of_shooter(sim: &mut Sim, first: Vec<Command>, ticks: u32) -> Vec<EntityId> {
    let mut out = Vec::new();
    for i in 0..ticks {
        let cmds = if i == 0 { first.clone() } else { Vec::new() };
        for e in WorldSource::step(sim, &cmds).events {
            if let SimEvent::Fired { shooter: EntityId(1), target: Target::Entity(t), .. } = e
                && !out.contains(&t)
            {
                out.push(t);
            }
        }
    }
    out
}

/// A destroyer ordered onto a target 1000 ahead past a bystander 250 off
/// its path (the measured scene `t1`, lane a).
fn en_route(profile: Profile) -> Vec<EntityId> {
    let mut sim = approach_tests::sim(approach_tests::db(profile, Some(1100)));
    let db = Arc::clone(sim.content.as_ref().unwrap());
    let (ship, target) = (db.key("st_destroy").unwrap(), db.key("st_target").unwrap());
    let s = sim.spawn_def(ship, PlayerId(1), v(0, 0), AHEAD).unwrap();
    let t = sim.spawn_def(target, PlayerId(2), v(0, 1000), AHEAD).unwrap();
    sim.spawn_def(target, PlayerId(2), v(250, 500), AHEAD).unwrap();
    let order = Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Attack { units: vec![s], target: t }) };
    targets_of_shooter(&mut sim, vec![order], 30 * 20)
}

#[test]
fn stock_attacker_fires_only_at_its_target_roots_also_at_a_bystander() {
    assert_eq!(en_route(Profile::Sta2Classic), [EntityId(2)]);
    let roots = en_route(Profile::Fo4);
    assert_eq!(roots[0], EntityId(3), "the bystander comes into range first");
    assert!(roots.contains(&EntityId(2)));
}

#[test]
fn a_moving_craft_fires_at_enemies_in_range_without_leaving_its_course() {
    // Measured on both games at every movement autonomy (scene `t1`, m0-m2).
    for profile in [Profile::Sta2Classic, Profile::Fo4] {
        let mut sim = approach_tests::sim(approach_tests::db(profile, Some(1100)));
        let db = Arc::clone(sim.content.as_ref().unwrap());
        let s = sim.spawn_def(db.key("st_destroy").unwrap(), PlayerId(1), v(0, 0), AHEAD).unwrap();
        sim.spawn_def(db.key("st_target").unwrap(), PlayerId(2), v(250, 1000), AHEAD).unwrap();
        let mv = Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Move { units: vec![s], to: v(0, 2000), queue: false }) };
        assert_eq!(targets_of_shooter(&mut sim, vec![mv], 30 * 25), [EntityId(2)]);
        let at = sim.unit(s).unwrap().pos;
        assert!((at.z - Fixed::from_int(2000)).abs() < Fixed::ONE && at.x == Fixed::ZERO, "{:?}", at);
    }
}

/// The fake content plus an armed construction rig, `builder_hold_fire` as asked.
fn builder_game(hold: bool) -> Sim {
    let fake = sta_content::fake::db();
    let mut features = fake.features().clone();
    features.builder_hold_fire = hold;
    let mut rig = UnitDef::new("fx_rig", "constructionrig");
    rig.race = "fx".into();
    rig.movement = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
    rig.hull = Fixed::from_int(100);
    rig.hardpoints = fake.unit(fake.key("fx_frigate").unwrap()).unwrap().hardpoints.clone();
    rig.hardpoints.iter_mut().for_each(|h| h.arc = None);
    let mut b = ContentDb::builder().profile(fake.profile()).features(features).unit(rig);
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    for u in fake.units() {
        b = b.unit(u.clone());
    }
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Human };
    Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::new(b.build().unwrap()),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    })
}

/// Shots of the rig at an enemy in range while it flies to its site and builds.
fn rig_shots_while_building(hold: bool) -> usize {
    let mut sim = builder_game(hold);
    let db = Arc::clone(sim.content.as_ref().unwrap());
    let rig = sim.spawn_def(db.key("fx_rig").unwrap(), PlayerId(0), v(0, 0), AHEAD).unwrap();
    let enemy = sim.spawn_def(db.key("fx_miner").unwrap(), PlayerId(1), v(300, 0), AHEAD).unwrap();
    assert_eq!(rig, EntityId(1));
    let cargo = Command { player: PlayerId::BENCH, seq: 0, kind: CommandKind::Edit(Edit::Cargo(PlayerId(0), 0, Fixed::from_int(5000))) };
    WorldSource::step(&mut sim, &[cargo]);
    let what = db.key("fx_station").unwrap();
    let build = Command { player: PlayerId(0), seq: 0, kind: CommandKind::Order(Order::Build { builder: rig, what, at: Some(v(0, 500)) }) };
    let mut shots = 0;
    for i in 0..30 * 20 {
        let cmds = if i == 0 { vec![build.clone()] } else { Vec::new() };
        let out = WorldSource::step(&mut sim, &cmds);
        assert!(sim.production.is_deploying(rig) || sim.production.is_welding(rig), "the rig is at work for the whole run");
        shots += out.events.iter().filter(|e| matches!(e, SimEvent::Fired { shooter, target: Target::Entity(t), .. } if *shooter == rig && *t == enemy)).count();
    }
    shots
}

#[test]
fn stock_construction_ships_hold_fire_while_going_to_build_and_building_roots_fire() {
    assert_eq!(rig_shots_while_building(true), 0);
    assert!(rig_shots_while_building(false) > 0);
}

#[test]
fn the_profiles_switch_the_attack_rules_as_measured() {
    let stock = Profile::Sta2Classic.features();
    assert!(stock.attack_range_buffer && stock.attack_warp_out_at_distance && stock.attack_combat_speed && stock.builder_hold_fire);
    assert!(!stock.attack_opportunity_fire);
    let roots = Profile::Fo4.features();
    assert!(roots.attack_opportunity_fire);
    assert!(!(roots.attack_range_buffer || roots.attack_warp_out_at_distance || roots.attack_combat_speed || roots.builder_hold_fire));
}
