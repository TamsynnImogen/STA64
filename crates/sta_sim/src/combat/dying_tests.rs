//! `dying_phase` (issue #1100): a unit at zero hull stays for its dying phase and is removed
//! when it ends. Through the whole sim, on the fake content with the switch set as given and
//! an authored slow torpedo (speed, reload and radius invented for these tests). Every
//! expected tick is derived from the constants in `dying`.

use std::sync::Arc;

use sta_content::{ContentDb, WeaponKind};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use super::dying::{
    phase_ticks, Dying, Row, DYING_DRIFT_DECAY, DYING_HIT_STEP_TICKS, DYING_LATE_TICKS, DYING_MOVING_SPEED,
    DYING_MOVING_TICKS, DYING_STANDING_TICKS,
};
use crate::command::{Command, CommandKind, Edit, Order};
use crate::event::SimEvent;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::view::UnitFlags;
use crate::{Sim, WorldSource};

/// The authored torpedo's speed (slow, so a shot is in flight for longer than a phase).
const TORPEDO_SPEED: i32 = 50;
/// A reload longer than any test runs: one torpedo per launcher.
const RELOAD: u64 = 100_000;
/// A body radius for the craft (the fake content has none), so they collide.
const RADIUS: i32 = 60;

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

/// `fake::db()` with `dying_phase` (and `beam_damage_at_end`) as given, the torpedo slowed, a
/// launcher (`fx_launcher`) that carries only it, and a radius on every craft.
fn db_with(dying: bool, beams_at_end: bool) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut features = fake.features().clone();
    features.dying_phase = dying;
    features.beam_damage_at_end = beams_at_end;
    let mut b = ContentDb::builder().profile(fake.profile()).features(features);
    for w in fake.weapons() {
        let mut w = w.clone();
        if w.odf == "fx_torpedo" {
            assert_eq!(w.kind, WeaponKind::Projectile);
            w.shot_speed = Fixed::from_int(TORPEDO_SPEED);
            w.refire = RELOAD;
        }
        b = b.weapon(w);
    }
    for u in fake.units() {
        let mut u = u.clone();
        if u.class.0 == "craft" {
            u.radius = Fixed::from_int(RADIUS);
        }
        b = b.unit(u);
    }
    let mut launcher = fake.units().find(|u| u.odf == "fx_cruiser").unwrap().clone();
    launcher.odf = "fx_launcher".into();
    launcher.radius = Fixed::from_int(RADIUS);
    launcher.hardpoints.retain(|h| h.weapon.odf == "fx_torpedo");
    assert_eq!(launcher.hardpoints.len(), 1);
    Arc::new(b.unit(launcher).build().unwrap())
}

fn db(dying: bool) -> Arc<ContentDb> {
    db_with(dying, false)
}

struct Game {
    sim: Sim,
    db: Arc<ContentDb>,
    /// Every event so far, with the tick of the step that sent it.
    log: Vec<(u64, SimEvent)>,
    seq: u32,
}

/// Two seats, nothing spawned.
fn game(db: Arc<ContentDb>) -> Game {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let setup = GameSetup { seed: 7, map: Arc::new(sta_content::MapDef::empty()), content: db.clone(), players: vec![seat(1), seat(2)], options: GameOptions::default() };
    Game { sim: Sim::from_setup(&setup), db, log: Vec::new(), seq: 0 }
}

impl Game {
    fn put(&mut self, def: &str, owner: u8, at: Vec3) -> EntityId {
        self.sim.spawn_def(self.db.key(def).unwrap(), PlayerId(owner), at, (Fixed::ZERO, Fixed::ONE)).unwrap()
    }

    fn step_with(&mut self, cmds: &[Command]) {
        let tick = self.sim.tick;
        for e in self.sim.step(cmds).events {
            self.log.push((tick, e));
        }
    }

    fn run(&mut self, ticks: u64) {
        for _ in 0..ticks {
            self.step_with(&[]);
        }
    }

    fn command(&mut self, player: PlayerId, kind: CommandKind) -> Command {
        self.seq += 1;
        Command { player, seq: self.seq, kind }
    }

    /// A script's lethal damage on `id`, in this step. Returns the step's tick.
    fn kill_by_damage(&mut self, id: EntityId) -> u64 {
        let tick = self.sim.tick;
        let c = self.command(PlayerId::SCRIPT, CommandKind::Edit(Edit::Damage(id, Fixed::from_int(1_000_000))));
        self.step_with(&[c]);
        tick
    }

    fn order(&mut self, player: u8, order: Order) {
        let c = self.command(PlayerId(player), CommandKind::Order(order));
        self.step_with(&[c]);
    }

    /// Steps until `id` is removed; the tick of its `Destroyed`.
    fn until_removed(&mut self, id: EntityId) -> u64 {
        for _ in 0..1000 {
            if let Some(t) = self.destroyed(id) {
                return t;
            }
            self.step_with(&[]);
        }
        panic!("{id:?} was never removed");
    }

    fn destroyed(&self, id: EntityId) -> Option<u64> {
        self.log.iter().find(|(_, e)| matches!(e, SimEvent::Destroyed { id: d, .. } if *d == id)).map(|(t, _)| *t)
    }

    /// The death events of `id`, in order, with their ticks.
    fn deaths(&self, id: EntityId) -> Vec<(u64, SimEvent)> {
        self.log
            .iter()
            .filter(|(_, e)| matches!(e, SimEvent::Dying { id: d, .. } | SimEvent::Destroyed { id: d, .. } if *d == id))
            .cloned()
            .collect()
    }

    /// The ticks of the hits `by` landed on `target`, with the damage each did.
    fn hits(&self, by: EntityId, target: EntityId) -> Vec<(u64, Fixed)> {
        self.log
            .iter()
            .filter_map(|(t, e)| match e {
                SimEvent::Hit { target: hit, by: b, shield, hull, .. } if *hit == target && *b == by => Some((*t, *shield + *hull)),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn a_unit_at_rest_stays_for_the_standing_phase_and_is_removed_when_it_ends() {
    let mut g = game(db(true));
    let t = g.put("fx_frigate", 2, v(0, 0));
    g.put("fx_frigate", 1, v(3000, 0));
    g.run(1);
    let k = g.kill_by_damage(t);
    let unit = g.sim.unit(t).expect("still in the world");
    assert!(unit.flags.contains(UnitFlags::DYING));
    assert_eq!(unit.hull[0], Fixed::ZERO);
    let ends = k + DYING_STANDING_TICKS + DYING_LATE_TICKS;
    assert_eq!(g.sim.dying_ends(t), Some(ends));
    g.run(ends - k - 1);
    assert!(g.sim.unit(t).is_some(), "present on every tick of the phase");
    assert_eq!(g.destroyed(t), None);
    g.run(1);
    assert!(g.sim.unit(t).is_none());
    assert_eq!(g.destroyed(t), Some(ends), "removed on the tick the phase ends");
    assert!(!g.sim.is_dying(t));
}

#[test]
fn a_unit_faster_than_the_threshold_takes_the_long_phase_and_drifts_on_its_heading() {
    assert_eq!(phase_ticks(Fixed::ZERO), DYING_STANDING_TICKS + DYING_LATE_TICKS);
    assert_eq!(phase_ticks(DYING_MOVING_SPEED), DYING_STANDING_TICKS + DYING_LATE_TICKS, "at the threshold is not faster");
    assert_eq!(phase_ticks(-DYING_MOVING_SPEED - Fixed::from_bits(1)), DYING_MOVING_TICKS + DYING_LATE_TICKS, "the speed's size counts");

    let mut g = game(db(true));
    let t = g.put("fx_frigate", 2, v(0, 0));
    g.put("fx_frigate", 1, v(3000, 0));
    g.run(1);
    let speed = Fixed::from_int(100);
    g.sim.units.get_mut(t).unwrap().speed = speed;
    let k = g.kill_by_damage(t);
    let ends = k + DYING_MOVING_TICKS + DYING_LATE_TICKS;
    assert_eq!(g.sim.dying_ends(t), Some(ends));

    // The drift: straight on along the heading, no turn, the speed decaying by the rate.
    let hz = Fixed::from_int(sta_core::TICK_HZ as i32);
    let mut last = g.sim.unit(t).unwrap().clone();
    for _ in 0..(ends - k - 1) {
        g.run(1);
        let u = g.sim.unit(t).unwrap().clone();
        assert_eq!(u.heading, last.heading, "no turn");
        assert_eq!(u.pos.x, last.pos.x, "on its heading (+z)");
        assert_eq!(u.pos.z, last.pos.z + last.heading.1 * last.speed / hz);
        assert_eq!(u.speed, last.speed - last.speed * DYING_DRIFT_DECAY / hz);
        assert!(u.speed > Fixed::ZERO && u.speed < last.speed);
        last = u;
    }
    assert_eq!(g.until_removed(t), ends);
}

#[test]
fn a_unit_killed_at_warp_drifts_from_no_more_than_its_impulse_speed() {
    let mut g = game(db(true));
    let t = g.put("fx_frigate", 2, v(0, 0));
    g.put("fx_frigate", 1, v(3000, 0));
    g.run(1);
    let impulse = g.sim.unit(t).unwrap().stats.max_speed;
    g.sim.units.get_mut(t).unwrap().speed = impulse * Fixed::from_int(5);
    let k = g.kill_by_damage(t);
    assert_eq!(g.sim.dying_ends(t), Some(k + DYING_MOVING_TICKS + DYING_LATE_TICKS), "the long phase");
    assert!(g.sim.unit(t).unwrap().speed <= impulse, "capped from the first dying tick");
    let z = g.sim.unit(t).unwrap().pos.z;
    g.run(1);
    let u = g.sim.unit(t).unwrap();
    assert!(u.speed > Fixed::ZERO && u.speed <= impulse);
    assert!(u.pos.z - z <= impulse / Fixed::from_int(sta_core::TICK_HZ as i32), "no step faster than impulse");
}

/// A launcher whose one torpedo is in flight to a frigate at rest. Returns the game, the
/// launcher, the target and the tick that torpedo arrives on, from a copy run ahead.
fn torpedo_in_flight(db: Arc<ContentDb>) -> (Game, EntityId, EntityId, u64) {
    let mut g = game(db);
    let l = g.put("fx_launcher", 1, v(2000, 2000));
    let t = g.put("fx_frigate", 2, v(2000, 2300));
    g.order(1, Order::Attack { units: vec![l], target: t });
    for _ in 0..50 {
        if g.sim.weapons.shots_in_flight() == 1 {
            break;
        }
        g.run(1);
    }
    assert_eq!(g.sim.weapons.shots_in_flight(), 1);
    let mut ahead = Game { sim: g.sim.clone(), db: g.db.clone(), log: Vec::new(), seq: g.seq };
    for _ in 0..1000 {
        if !ahead.hits(l, t).is_empty() {
            break;
        }
        ahead.run(1);
    }
    let arrives = ahead.hits(l, t)[0].0;
    (g, l, t, arrives)
}

#[test]
fn a_torpedo_in_flight_lands_on_a_dying_unit_and_shortens_its_phase_by_the_step() {
    let phase = phase_ticks(Fixed::ZERO);
    let (mut g, l, t, arrives) = torpedo_in_flight(db(true));
    // Killed so that the torpedo lands well inside the phase.
    let into = DYING_HIT_STEP_TICKS * 2;
    assert!(arrives - g.sim.tick > into);
    g.run(arrives - into - g.sim.tick);
    let k = g.kill_by_damage(t);
    let removed = g.until_removed(t);
    assert_eq!(g.hits(l, t), [(arrives, Fixed::ZERO)], "the shot lands, and does no damage");
    assert_eq!(removed, k + phase - DYING_HIT_STEP_TICKS);
}

#[test]
fn a_hit_with_less_than_the_step_left_removes_the_unit_on_the_next_tick() {
    let phase = phase_ticks(Fixed::ZERO);
    let (mut g, l, t, arrives) = torpedo_in_flight(db(true));
    let left = DYING_HIT_STEP_TICKS / 2;
    assert!(left > 0 && arrives - g.sim.tick > phase - left);
    g.run(arrives - (phase - left) - g.sim.tick);
    let k = g.kill_by_damage(t);
    assert_eq!(k + phase - arrives, left, "this much of the phase is left when the hit lands");
    let removed = g.until_removed(t);
    assert_eq!(g.hits(l, t), [(arrives, Fixed::ZERO)]);
    assert_eq!(removed, arrives + 1);
}

#[test]
fn a_weapon_kill_sends_dying_then_destroyed_with_the_same_killer() {
    let (mut g, l, t, arrives) = torpedo_in_flight(db(true));
    // One point of hull and no shields left: the torpedo kills.
    {
        let u = g.sim.units.get_mut(t).unwrap();
        u.hull[0] = Fixed::ONE;
        u.shields.now = [Fixed::ZERO; 4];
        u.shields.max = Fixed::ZERO;
    }
    let removed = g.until_removed(t);
    assert_eq!(
        g.deaths(t),
        [(arrives, SimEvent::Dying { id: t, by: Some(l) }), (removed, SimEvent::Destroyed { id: t, by: Some(l) })]
    );
    assert_eq!(removed, arrives + phase_ticks(Fixed::ZERO));
    // On the lethal tick the `Dying` follows the lethal `Hit`.
    let lethal: Vec<_> = g.log.iter().filter(|(tick, _)| *tick == arrives).map(|(_, e)| e.clone()).collect();
    let hit = lethal.iter().position(|e| matches!(e, SimEvent::Hit { target, .. } if *target == t)).unwrap();
    let dying = lethal.iter().position(|e| matches!(e, SimEvent::Dying { .. })).unwrap();
    assert!(hit < dying);
}

#[test]
fn a_scripted_kill_removes_at_once_also_during_the_phase() {
    let mut g = game(db(true));
    let a = g.put("fx_frigate", 2, v(0, 0));
    let b = g.put("fx_frigate", 2, v(500, 0));
    g.put("fx_frigate", 1, v(3000, 0));
    g.run(1);
    let tick = g.sim.tick;
    let c = g.command(PlayerId::SCRIPT, CommandKind::Edit(Edit::Kill(a)));
    g.step_with(&[c]);
    assert!(g.sim.unit(a).is_none());
    assert_eq!(g.deaths(a), [(tick, SimEvent::Destroyed { id: a, by: None })], "no dying phase");

    g.kill_by_damage(b);
    g.run(3);
    assert!(g.sim.is_dying(b));
    let tick = g.sim.tick;
    let c = g.command(PlayerId::SCRIPT, CommandKind::Edit(Edit::Kill(b)));
    g.step_with(&[c]);
    assert!(g.sim.unit(b).is_none() && !g.sim.is_dying(b));
    assert_eq!(g.destroyed(b), Some(tick));
    g.run(phase_ticks(Fixed::ZERO));
    assert_eq!(g.deaths(b).len(), 2, "one Dying, one Destroyed");
}

#[test]
fn a_dying_unit_is_not_chosen_as_a_new_target_and_an_attack_on_it_ends() {
    let mut g = game(db(true));
    let s = g.put("fx_frigate", 1, v(0, 0));
    let near = g.put("fx_frigate", 2, v(0, 300));
    let far = g.put("fx_frigate", 2, v(0, 500));
    g.order(1, Order::Attack { units: vec![s], target: near });
    let k = g.kill_by_damage(near);
    g.run(phase_ticks(Fixed::ZERO) - 1);
    assert!(g.sim.is_dying(near), "still in its phase");
    assert_eq!(g.sim.weapons.attack_of(s), None, "the attack on it ended");
    let fired: Vec<_> = g.log.iter().filter_map(|(t, e)| match e {
        SimEvent::Fired { shooter, target: crate::event::Target::Entity(at), .. } if *shooter == s && *t >= k => Some(*at),
        _ => None,
    }).collect();
    assert!(!fired.is_empty(), "the shooter kept firing");
    assert!(fired.iter().all(|&at| at == far), "never at the dying unit: {fired:?}");
    // An attack ordered on it is refused.
    let c = g.command(PlayerId(1), CommandKind::Order(Order::Attack { units: vec![s], target: near }));
    let seq = c.seq;
    g.step_with(&[c]);
    assert!(g.log.iter().any(|(_, e)| *e == SimEvent::Rejected { player: PlayerId(1), seq }));
}

#[test]
fn a_dying_unit_fires_nothing_takes_no_orders_and_is_not_selectable() {
    let mut g = game(db(true));
    let s = g.put("fx_frigate", 1, v(0, 0));
    let other = g.put("fx_frigate", 1, v(1000, 0));
    g.put("fx_frigate", 2, v(0, 300));
    g.run(1);
    g.kill_by_damage(s);
    let from = g.log.len();
    g.run(10);
    assert!(!g.log[from..].iter().any(|(_, e)| matches!(e, SimEvent::Fired { shooter, .. } if *shooter == s)));
    let view = g.sim.view();
    assert!(!view.units.iter().find(|u| u.id == s).unwrap().flags.contains(UnitFlags::SELECTABLE));
    assert!(view.units.iter().find(|u| u.id == other).unwrap().flags.contains(UnitFlags::SELECTABLE));
    // An order for it alone is refused; one for it and a live craft goes to the live one.
    let c = g.command(PlayerId(1), CommandKind::Order(Order::move_to(vec![s], v(0, -2000), false)));
    let seq = c.seq;
    g.step_with(&[c]);
    assert!(g.log.iter().any(|(_, e)| *e == SimEvent::Rejected { player: PlayerId(1), seq }));
    g.order(1, Order::move_to(vec![s, other], v(1000, -2000), false));
    assert_eq!(g.sim.unit(s).unwrap().target, None);
    assert!(g.sim.unit(other).unwrap().target.is_some());
}

#[test]
fn other_craft_do_not_collide_with_a_dying_one() {
    // Two craft at rest overlapping: a live pair is pushed apart, a dying one is not an obstacle.
    let pair = |dying: bool| {
        let mut g = game(db(true));
        let a = g.put("fx_frigate", 2, v(0, 0));
        let b = g.put("fx_frigate", 1, v(RADIUS / 2, 0));
        g.put("fx_frigate", 1, v(5000, 0));
        if dying {
            g.kill_by_damage(a);
        } else {
            g.run(1);
        }
        g.run(5);
        (g.sim.unit(a).unwrap().pos, g.sim.unit(b).unwrap().pos)
    };
    let (a, b) = pair(false);
    assert!(a != v(0, 0) || b != v(RADIUS / 2, 0), "the live control is pushed apart");
    assert_eq!(pair(true), (v(0, 0), v(RADIUS / 2, 0)), "neither moves");
}

#[test]
fn the_seat_is_eliminated_when_its_last_unit_is_removed_not_when_it_starts_to_die() {
    let mut g = game(db(true));
    let t = g.put("fx_frigate", 2, v(0, 0));
    g.put("fx_frigate", 1, v(3000, 0));
    g.run(1);
    g.kill_by_damage(t);
    let ends = g.sim.dying_ends(t).unwrap();
    g.run(ends - g.sim.tick);
    assert!(!g.sim.is_eliminated(PlayerId(2)), "alive while its unit is dying");
    assert!(!g.log.iter().any(|(_, e)| matches!(e, SimEvent::PlayerEliminated { .. })));
    g.run(1);
    assert!(g.sim.is_eliminated(PlayerId(2)));
    let at: Vec<_> = g.log.iter().filter(|(_, e)| matches!(e, SimEvent::PlayerEliminated { player } if *player == PlayerId(2))).map(|(t, _)| *t).collect();
    assert_eq!(at, [ends]);
    assert_eq!(g.sim.winners(), Some(vec![PlayerId(1)]));
}

#[test]
fn a_snapshot_taken_mid_phase_reloads_and_removes_on_the_same_tick() {
    let mut g = game(db(true));
    let t = g.put("fx_frigate", 2, v(0, 0));
    g.put("fx_frigate", 1, v(3000, 0));
    g.run(1);
    g.sim.units.get_mut(t).unwrap().speed = Fixed::from_int(80);
    g.kill_by_damage(t);
    g.run(DYING_STANDING_TICKS / 2);
    let bytes = g.sim.snapshot();
    let sim = Sim::from_snapshot(&bytes, Some(g.db.clone())).unwrap();
    assert_eq!(sim.snapshot(), bytes, "save, load, save");
    assert_eq!(sim.digest(), g.sim.digest());
    assert_eq!(sim.dying_ends(t), g.sim.dying_ends(t));
    let mut back = Game { sim, db: g.db.clone(), log: Vec::new(), seq: g.seq };
    for _ in 0..DYING_MOVING_TICKS {
        g.run(1);
        back.run(1);
        assert_eq!(back.sim.digest(), g.sim.digest(), "tick {}", g.sim.tick);
    }
    assert!(g.destroyed(t).is_some());
    assert_eq!(back.destroyed(t), g.destroyed(t));
}

#[test]
fn a_dying_row_is_in_the_digest_and_a_bad_row_is_refused() {
    let mut g = game(db(true));
    let t = g.put("fx_frigate", 2, v(0, 0));
    let live = g.put("fx_frigate", 1, v(3000, 0));
    g.run(1);
    g.kill_by_damage(t);
    let with = g.sim.digest();
    let mut other = g.sim.clone();
    other.dying = Dying::default();
    assert_ne!(other.digest(), with, "the row is hashed");
    let tick = g.sim.tick;
    let ends = g.sim.dying_ends(t).unwrap();
    let row = |ends| Row { ends, by: None };
    assert!(Dying::from_rows(vec![(t, row(ends))], tick, &g.sim.units).is_some());
    assert!(Dying::from_rows(vec![(t, row(tick - 1))], tick, &g.sim.units).is_none(), "ended before the snapshot's tick");
    assert!(Dying::from_rows(vec![(live, row(ends))], tick, &g.sim.units).is_none(), "a unit that is not dying");
    assert!(Dying::from_rows(vec![(EntityId(999), row(ends))], tick, &g.sim.units).is_none(), "no such unit");
}

#[test]
fn a_beam_in_flight_from_a_shooter_that_starts_to_die_is_lost() {
    let mut g = game(db_with(true, true));
    let s = g.put("fx_frigate", 1, v(0, 0));
    let t = g.put("fx_frigate", 2, v(0, 300));
    g.order(1, Order::Attack { units: vec![s], target: t });
    for _ in 0..100 {
        if g.sim.weapons.beams_in_flight() == 1 {
            break;
        }
        g.run(1);
    }
    assert_eq!(g.sim.weapons.beams_in_flight(), 1);
    g.kill_by_damage(s);
    g.run(1);
    assert_eq!(g.sim.weapons.beams_in_flight(), 0);
    g.run(60);
    assert!(g.hits(s, t).is_empty(), "{:?}", g.hits(s, t));
}

#[test]
fn with_the_switch_off_a_unit_at_zero_hull_is_removed_at_once() {
    let mut g = game(db(false));
    let a = g.put("fx_frigate", 2, v(0, 0));
    g.put("fx_frigate", 1, v(3000, 0));
    g.run(1);
    let k = g.kill_by_damage(a);
    assert!(g.sim.unit(a).is_none());
    assert_eq!(g.deaths(a), [(k, SimEvent::Destroyed { id: a, by: None })]);

    let (mut g, l, t, arrives) = torpedo_in_flight(db(false));
    {
        let u = g.sim.units.get_mut(t).unwrap();
        u.hull[0] = Fixed::ONE;
        u.shields.now = [Fixed::ZERO; 4];
        u.shields.max = Fixed::ZERO;
    }
    assert_eq!(g.until_removed(t), arrives);
    assert_eq!(g.deaths(t), [(arrives, SimEvent::Destroyed { id: t, by: Some(l) })]);
    // A content without the switch never has a dying row: the snapshot's record stays empty.
    let bytes = g.sim.snapshot();
    assert_eq!(&bytes[bytes.len() - 2..], [1, 0]);
}
