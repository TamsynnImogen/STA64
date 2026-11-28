//! #1039: a burst of Move orders (many in one tick, or one per tick, or alternating targets).
//!
//! The issue reports, from a playtest capture, a target that flickered between set and unset on
//! every tick and a turn rate far above the craft's limit. These tests pin what the sim does with
//! a burst on invented data (a capital ship that cannot turn at rest, smooth model). Bursts of
//! Move orders did not turn a craft past its full-speed limit either; that guard was dropped as
//! it passed with or without the turn-rate fixes of this PR (see `kinematics/tests.rs` for those).
//!
//! - after the last order of a burst the target is that order's point (or, in a formation, its slot)
//!   on every tick until arrival: never a superseded order's, never unset;
//! - queued orders keep their order;
//! - an order to a point the craft has already reached ends at once: the order sets the target and
//!   the craft's own arrival test clears it in the same tick, so a burst of such orders shows the
//!   target set by the command and unset by the step, tick after tick. That is the arrival rule,
//!   not a loop, and a turn left over at that moment settles (#1038).
//!
//! Evidence grade: STA64's own rule; what the originals do with a rapid repeat of a move order is
//! not measured.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, MoveStats, Profile};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use crate::command::{Command, CommandKind, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

fn f(x: f64) -> Fixed {
    Fixed::from_f64(x)
}

/// A capital ship of invented stats: it cannot turn or accelerate its turn at rest.
fn capital() -> MoveStats {
    MoveStats {
        combat_speed: f(36.0),
        turn_alpha_at_rest: Fixed::ZERO,
        too_close_to_turn: f(60.0),
        path_lead: f(140.0),
        turn_control_angle: f(25.0),
        turn_control_squared: false,
        forward_control_distance: f(90.0),
        ..MoveStats::from_f32(72.0, 2.5, 0.06, 0.8, 0.0)
    }
}

fn db(stats: MoveStats) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut b = ContentDb::builder().profile(Profile::Fo4).features(Profile::Fo4.features());
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    for u in fake.units() {
        let mut u = u.clone();
        if u.odf == "fx_cruiser" {
            u.movement = stats;
        }
        b = b.unit(u);
    }
    Arc::new(b.build().unwrap())
}

/// `n` capital ships of player 1 in a column along +Z, facing +Z, in a line formation when `n > 1`.
fn world(n: i32) -> (Sim, Vec<EntityId>) {
    let d = db(capital());
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup { seed: 7, map: Arc::new(MapDef::empty()), content: Arc::clone(&d), players: vec![seat(1), seat(2)], options: GameOptions::default() });
    let ids: Vec<EntityId> = (0..n).map(|i| sim.spawn_def(d.key("fx_cruiser").unwrap(), PlayerId(1), v(i * 150, 0), (Fixed::ZERO, Fixed::ONE)).unwrap()).collect();
    if n > 1 {
        sim.step(&[Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Formation { units: ids.clone(), kind: 1 }) }]);
    }
    (sim, ids)
}

fn mv(seq: u32, ids: &[EntityId], to: Vec3, queue: bool) -> Command {
    Command { player: PlayerId(1), seq, kind: CommandKind::Order(Order::Move { units: ids.to_vec(), to, queue, mode: None }) }
}

/// Gets the craft moving at cruise speed (a long leg), so a burst finds it under way.
fn get_going(sim: &mut Sim, ids: &[EntityId]) {
    sim.step(&[mv(1, ids, v(0, 6000), false)]);
    for _ in 0..150 {
        sim.step(&[]);
    }
}

/// One burst: the commands of each tick of its first ticks.
type Burst = Vec<Vec<(Vec3, bool)>>;

fn bursts() -> Vec<(&'static str, Burst)> {
    let (a, b) = (v(2000, 2000), v(-2000, 2000));
    let alternating = |queue| (0..24).map(|t| vec![(if t % 2 == 0 { a } else { b }, queue)]).collect();
    let mut same_tick: Burst = vec![(0..20).map(|i| (if i % 2 == 0 { a } else { b }, false)).collect()];
    same_tick.extend((0..8).map(|_| vec![]));
    vec![
        ("alternating targets, one order per tick", alternating(false)),
        ("alternating targets, queued, one order per tick", alternating(true)),
        ("twenty alternating orders in one tick", same_tick),
        ("identical order every tick", (0..24).map(|_| vec![(a, false)]).collect()),
        ("identical queued order every tick", (0..24).map(|_| vec![(a, true)]).collect()),
        ("an order every third tick", (0..24).map(|t| if t % 3 == 0 { vec![(if t % 2 == 0 { a } else { b }, false)] } else { vec![] }).collect()),
    ]
}

/// Steps the burst and then 300 ticks more, calling `check` after every tick with the commands
/// given in it.
fn run_burst(sim: &mut Sim, ids: &[EntityId], burst: &Burst, mut check: impl FnMut(&Sim, &[Vec3])) {
    let mut seq = 100;
    for t in 0..burst.len() + 300 {
        let given: Vec<(Vec3, bool)> = burst.get(t).cloned().unwrap_or_default();
        let cmds: Vec<Command> = given.iter().map(|&(to, q)| { seq += 1; mv(seq, ids, to, q) }).collect();
        sim.step(&cmds);
        check(sim, &given.iter().map(|g| g.0).collect::<Vec<_>>());
    }
}

#[test]
fn after_a_burst_the_target_is_the_last_order_every_tick_until_arrival() {
    // The last plain (not queued) order of each burst; a craft ordered far away keeps it as its
    // target until it gets there, so a superseded point or an unset target is a flicker.
    for (name, burst) in bursts() {
        let queued = name.contains("queued");
        for moving in [false, true] {
            let (mut sim, ids) = world(1);
            if moving {
                get_going(&mut sim, &ids);
            }
            let last = burst.iter().flatten().last().map(|g| g.0).unwrap();
            let mut seen_last_order = false;
            let orders_total: usize = burst.iter().map(Vec::len).sum();
            let mut given = 0;
            run_burst(&mut sim, &ids, &burst, |sim, now| {
                given += now.len();
                seen_last_order |= given == orders_total;
                let u = sim.unit(ids[0]).unwrap();
                if !seen_last_order {
                    return;
                }
                if queued {
                    // The first queued order of the burst is the leg being flown, the rest wait.
                    assert!(u.target.is_some(), "{name}, moving {moving}: target unset after the last order");
                } else {
                    assert_eq!(u.target, Some(last), "{name}, moving {moving}: the target is not the last order's");
                    assert!(sim.movement.course(ids[0]).is_none(), "{name}: queued legs left over");
                }
            });
        }
    }
}

#[test]
fn queued_orders_of_a_burst_are_flown_in_the_order_given() {
    let (mut sim, ids) = world(1);
    let legs = [v(0, 400), v(100, 800), v(200, 1200), v(300, 1600)];
    let cmds: Vec<Command> = legs.iter().enumerate().map(|(i, &p)| mv(i as u32 + 1, &ids, p, true)).collect();
    sim.step(&cmds);
    let mut order = vec![legs[0]];
    for _ in 0..3000 {
        sim.step(&[]);
        if let Some(t) = sim.unit(ids[0]).unwrap().target
            && order.last() != Some(&t)
        {
            order.push(t);
        }
    }
    assert_eq!(order, legs);
    assert_eq!(sim.unit(ids[0]).unwrap().target, None);
}

#[test]
fn a_repeat_of_an_order_the_craft_has_already_reached_ends_at_once_each_tick() {
    // The order sets the target; the craft's arrival test (inside tooCloseToTurn of the final
    // point, slower than half of combatSpeed) clears it in the same tick. A burst of these shows
    // a target set by each command and unset by each step. The craft stays put.
    let (mut sim, ids) = world(1);
    let here = v(0, 20);
    let mut seq = 0;
    for _ in 0..40 {
        seq += 1;
        sim.step(&[mv(seq, &ids, here, false)]);
        let u = sim.unit(ids[0]).unwrap();
        assert_eq!(u.target, None);
        assert_eq!((u.speed, u.omega), (Fixed::ZERO, Fixed::ZERO));
        assert!(sim.movement.course(ids[0]).is_none());
    }
}
