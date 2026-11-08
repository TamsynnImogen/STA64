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
    assert_eq!(sim.digest(), 0x357d_b900_f0d7_a140);
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
