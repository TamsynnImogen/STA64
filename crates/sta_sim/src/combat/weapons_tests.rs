use super::weapons::*;
use crate::command::{Command, CommandKind, Order};
use crate::event::{SimEvent, Target};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};
use sta_content::FireArc;
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, TeamId, Vec3};
use std::sync::Arc;

fn v(x: i32, y: i32, z: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::from_int(y),
        z: Fixed::from_int(z),
    }
}

const AHEAD: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);

/// The `firing` fixture (bench/scenes/firing.ini) on the fake content: a
/// shooter at (2000, 0, 2000) facing +z and a target, both stationary. Fake
/// ships stand in for the Galaxy and the Borg cube (CON-03 brings the real
/// ODFs).
fn firing(target_at: Vec3) -> (Sim, EntityId, EntityId) {
    let seat = |id| PlayerSetup {
        id: PlayerId(id),
        team: TeamId(id),
        race: "fx".into(),
        control: Control::Idle,
    };
    let db = sta_content::fake::db();
    let (cruiser, frigate) = (db.key("fx_cruiser").unwrap(), db.key("fx_frigate").unwrap());
    let setup = GameSetup {
        seed: 7,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::new(db),
        players: vec![seat(1), seat(2)],
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let shooter = sim
        .spawn_def(cruiser, PlayerId(1), v(2000, 0, 2000), AHEAD)
        .unwrap();
    let target = sim
        .spawn_def(frigate, PlayerId(2), target_at, AHEAD)
        .unwrap();
    (sim, shooter, target)
}

fn attack(player: u8, shooter: EntityId, target: EntityId) -> Command {
    Command {
        player: PlayerId(player),
        seq: 0,
        kind: CommandKind::Order(Order::Attack {
            units: vec![shooter],
            target,
        }),
    }
}

/// `(tick, event)` for every Fired and Hit over `ticks` ticks; tick 0
/// carries `cmd`.
fn run(sim: &mut Sim, cmd: Option<Command>, ticks: u64) -> Vec<(u64, SimEvent)> {
    let mut out = Vec::new();
    for t in 0..ticks {
        let cmds: Vec<Command> = if t == 0 {
            cmd.clone().into_iter().collect()
        } else {
            Vec::new()
        };
        for e in sim.step(&cmds).events {
            // The first ship (the shooter) only; the target may shoot back.
            if matches!(
                e,
                SimEvent::Fired {
                    shooter: EntityId(1),
                    ..
                } | SimEvent::Hit {
                    by: EntityId(1),
                    ..
                }
            ) {
                out.push((t, e));
            }
        }
    }
    out
}

/// Ticks of the `Fired`s of one hardpoint and of the `Hit`s of its weapon.
fn marks(ev: &[(u64, SimEvent)], hp: u8) -> (Vec<u64>, Vec<u64>) {
    let weapon: Option<ContentKey> = ev.iter().find_map(|(_, e)| match e {
        SimEvent::Fired {
            weapon, hardpoint, ..
        } if *hardpoint == hp => Some(*weapon),
        _ => None,
    });
    let fired = ev
        .iter()
        .filter(|(_, e)| matches!(e, SimEvent::Fired { hardpoint, .. } if *hardpoint == hp))
        .map(|(t, _)| *t)
        .collect();
    let hits = ev
        .iter()
        .filter(|(_, e)| matches!(e, SimEvent::Hit { weapon: w, .. } if Some(*w) == weapon))
        .map(|(t, _)| *t)
        .collect();
    (fired, hits)
}

/// Done-when for SIM-04 (TOOL-02's bench runner does not exist yet):
/// `firing`'s `fire.start` is a `Fired`, `fire.end` is `Fired` + `duration`.
/// Beam (refire 45, duration 15): fires at 0, 45, 90, ..., hitting at once.
/// Torpedo (refire 90, 400 u/s = 13.33 u/tick over 350 u): fires at 0, 90,
/// first move on the next tick, arrives 27 ticks after firing.
#[test]
fn firing_scene_marks_at_expected_ticks() {
    let (mut sim, s, t) = firing(v(2000, 0, 2350));
    let ev = run(&mut sim, Some(attack(1, s, t)), 200);
    let (beam_fired, beam_hits) = marks(&ev, 0);
    assert_eq!(beam_fired, [0, 45, 90, 135, 180]);
    assert_eq!(beam_hits, beam_fired);
    let beam_ends: Vec<u64> = beam_fired.iter().map(|f| f + 15).collect();
    assert_eq!(beam_ends, [15, 60, 105, 150, 195]);
    let (tp_fired, tp_hits) = marks(&ev, 1);
    assert_eq!(tp_fired, [0, 90, 180]);
    assert_eq!(tp_hits, [27, 117]);
    // Everything is the cruiser shooting the frigate, which faces away from it.
    assert!(ev.iter().all(|(_, e)| match e {
        SimEvent::Fired {
            shooter, target, ..
        } => *shooter == s && *target == Target::Entity(t),
        SimEvent::Hit {
            target, by, facing, ..
        } => *target == t && *by == s && *facing == 1,
        _ => false,
    }));
    // Pinned: the digest of the run, mounts and shots included. Moved from
    // 0xe740_56a9_e426_fdd1 when SIM-05 began applying damage (shields drop).
    assert_eq!(crate::unit_status_tests::digest_before_crew(&sim), 0x357d_b900_f0d7_a140);
    assert_eq!(sim.digest(), 0x785b_08b0_2193_3eff);
}

#[test]
fn fire_arc_blocks_the_beam_but_not_the_torpedo() {
    // Behind the shooter: the beam's fore arc excludes it, the torpedo's
    // all-round mount fires.
    let (mut sim, s, t) = firing(v(2000, 0, 1650));
    let ev = run(&mut sim, Some(attack(1, s, t)), 100);
    assert!(marks(&ev, 0).0.is_empty());
    assert_eq!(marks(&ev, 1).0, [0, 90]);
}

#[test]
fn range_limits_fire_and_enemies_are_found_without_an_order() {
    // 700 away: the beam's range of 600 falls short, the torpedo's 900 reaches.
    let (mut sim, ..) = firing(v(2000, 0, 2700));
    let ev = run(&mut sim, None, 60);
    assert!(marks(&ev, 0).0.is_empty());
    assert_eq!(marks(&ev, 1).0, [0]);
}

#[test]
fn arc_maths() {
    let arc = Some(FireArc {
        dir: (Fixed::ZERO, Fixed::ONE),
        min_dot: Fixed::from_f64(std::f64::consts::FRAC_1_SQRT_2),
    });
    let o = v(0, 0, 0);
    assert!(in_arc(AHEAD, arc, o, v(0, 0, 10)));
    assert!(in_arc(AHEAD, arc, o, v(9, 0, 10)));
    assert!(!in_arc(AHEAD, arc, o, v(11, 0, 10)));
    assert!(!in_arc(AHEAD, arc, o, v(0, 0, -10)));
    // Facing +x, ahead is +x; a starboard arc (dir +x) then looks toward -z.
    let east = (Fixed::ONE, Fixed::ZERO);
    assert!(in_arc(east, arc, o, v(10, 0, 0)));
    let stbd = Some(FireArc {
        dir: (Fixed::ONE, Fixed::ZERO),
        min_dot: Fixed::from_f64(std::f64::consts::FRAC_1_SQRT_2),
    });
    assert!(in_arc(east, stbd, o, v(0, 0, -10)));
    assert!(!in_arc(east, stbd, o, v(0, 0, 10)));
    assert!(in_arc(east, None, o, v(-5, 0, 5)));
}

#[test]
fn orders_are_checked() {
    let (mut sim, s, t) = firing(v(2000, 0, 2350));
    // Player 2 may not order player 1's ship; an unarmed or missing target fails.
    let out = sim.step(&[attack(2, s, t), attack(1, s, EntityId(99))]);
    let rejected = out
        .events
        .iter()
        .filter(|e| matches!(e, SimEvent::Rejected { .. }))
        .count();
    assert_eq!(rejected, 2);
    assert_eq!(
        sim.step(&[attack(1, s, t)])
            .events
            .iter()
            .filter(|e| matches!(e, SimEvent::Rejected { .. }))
            .count(),
        0
    );
}

#[test]
fn stop_clears_the_attack_order() {
    let (mut sim, s, t) = firing(v(2000, 0, 2350));
    sim.step(&[attack(1, s, t)]);
    assert_eq!(sim.weapons.attack_of(s), Some(t));
    // Another player's stop does nothing to it; the owner's clears it.
    let stop = |p| Command {
        player: PlayerId(p),
        seq: 1,
        kind: CommandKind::Order(Order::Stop { units: vec![s] }),
    };
    sim.step(&[stop(2)]);
    assert_eq!(sim.weapons.attack_of(s), Some(t));
    sim.step(&[stop(1)]);
    assert_eq!(sim.weapons.attack_of(s), None);
}

#[test]
fn same_run_same_digest() {
    let d = || {
        let (mut sim, s, t) = firing(v(2000, 0, 2350));
        run(&mut sim, Some(attack(1, s, t)), 150);
        sim.digest()
    };
    assert_eq!(d(), d());
}

/// Artillery range exceeds its own sensors. Player 3 shares player 1's vision.
fn artillery() -> (Sim, EntityId, EntityId) {
    artillery_kind(sta_content::WeaponKind::Beam)
}

fn artillery_kind(kind: sta_content::WeaponKind) -> (Sim, EntityId, EntityId) {
    use sta_content::{ContentDb, DefRef, Hardpoint, UnitDef, WeaponDef};
    let mut weapon = WeaponDef::new("sight_gun", "phaser", kind);
    weapon.range = Fixed::from_int(600);
    weapon.damage = Fixed::ONE;
    weapon.refire = 1;
    weapon.shot_speed = Fixed::from_int(3000);
    let mut gun = UnitDef::new("sight_artillery", "craft");
    gun.hull = Fixed::from_int(100);
    gun.sensor = Fixed::from_int(100);
    gun.hardpoints.push(Hardpoint { weapon: DefRef::new("sight_gun"), node: "hp01".into(), arc: None });
    let mut scout = UnitDef::new("sight_scout", "craft");
    scout.hull = Fixed::from_int(100);
    scout.sensor = Fixed::from_int(100);
    let db = ContentDb::builder().weapon(weapon).unit(gun).unit(scout).build().unwrap();
    let setup = GameSetup {
        seed: 340, map: Arc::new(sta_content::MapDef::empty()), content: Arc::new(db),
        players: [(1,1), (2,2), (3,1)].into_iter().map(|(id,team)| PlayerSetup {
            id: PlayerId(id), team: TeamId(team), race: "test".into(), control: Control::Idle,
        }).collect(), options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let gun = sim.content.as_ref().unwrap().key("sight_artillery").unwrap();
    let scout = sim.content.as_ref().unwrap().key("sight_scout").unwrap();
    let shooter = sim.spawn_def(gun, PlayerId(1), v(2000,0,2000), AHEAD).unwrap();
    let target = sim.spawn_def(scout, PlayerId(2), v(2000,0,2500), AHEAD).unwrap();
    (sim, shooter, target)
}

fn fired_by(events: &[SimEvent], shooter: EntityId) -> bool {
    events.iter().any(|e| matches!(e, SimEvent::Fired { shooter: s, .. } if *s == shooter))
}

#[test]
fn artillery_needs_current_shared_vision_for_ordered_and_automatic_fire() {
    for ordered in [false, true] {
        let (mut sim, shooter, target) = artillery();
        let cmds: Vec<_> = ordered.then(|| attack(1,shooter,target)).into_iter().collect();
        assert!(!fired_by(&sim.step(&cmds).events, shooter));
        let scout = sim.content.as_ref().unwrap().key("sight_scout").unwrap();
        let spotter = sim.spawn_def(scout, PlayerId(3), v(2000,0,2550), AHEAD).unwrap();
        assert!(fired_by(&sim.step(&[]).events, shooter), "allied spotter, ordered={ordered}");
        sim.despawn(spotter, None);
        assert!(!fired_by(&sim.step(&[]).events, shooter), "explored fog is not current vision");
        if ordered { assert_eq!(sim.weapons.attack_of(shooter), Some(target)); }
        // Even with current allied sight the weapon's range still applies.
        sim.units.get_mut(target).unwrap().pos = v(2000,0,2700);
        sim.spawn_def(scout, PlayerId(3), v(2000,0,2750), AHEAD).unwrap();
        assert!(!fired_by(&sim.step(&[]).events, shooter));
        // Return inside range, then cloak: allied sensors cannot reveal it.
        sim.units.get_mut(target).unwrap().pos = v(2000,0,2500);
        let entity = sim.units.get_mut(target).unwrap();
        entity.flags = entity.flags.with(crate::view::UnitFlags::CLOAKED, true);
        sim.spawn_def(scout, PlayerId(3), v(2000,0,2550), AHEAD).unwrap();
        assert!(!fired_by(&sim.step(&[]).events, shooter));
    }
}

#[test]
fn unknown_owners_get_no_seated_vision_but_seatless_legacy_api_is_omniscient() {
    let (mut sim, shooter, target) = artillery();
    sim.units.get_mut(shooter).unwrap().owner = PlayerId(9);
    assert!(!fired_by(&sim.step(&[attack(9,shooter,target)]).events, shooter));
    sim.players.clear();
    assert!(fired_by(&sim.step(&[]).events, shooter));
}


#[test]
fn launched_projectiles_survive_lost_sight_and_snapshot_resume() {
    let (mut sim, shooter,target)=artillery_kind(sta_content::WeaponKind::Projectile);
    sim.units.get_mut(target).unwrap().pos=v(2000,0,2080);
    assert!(fired_by(&sim.step(&[attack(1,shooter,target)]).events,shooter));
    sim.units.get_mut(target).unwrap().pos=v(2000,0,2500);
    let bytes=sim.snapshot();
    let mut restored=Sim::from_snapshot(&bytes,sim.content.clone()).unwrap();
    let mut hit=false;
    for _ in 0..10 {
        let a=sim.step(&[]); let b=restored.step(&[]);
        assert_eq!(a.view.digest,b.view.digest);
        assert_eq!(a.events,b.events);
        assert!(!fired_by(&a.events,shooter),"no new shot without sight");
        hit |= a.events.iter().any(|e|matches!(e,SimEvent::Hit {target:t,by,..} if *t==target && *by==shooter));
    }
    assert!(hit,"already launched projectile should still arrive");
}
