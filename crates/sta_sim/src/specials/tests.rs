use super::*;
use crate::command::{Command, CommandKind, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::view::WorldView;
use crate::{Sim, WorldSource};
use sta_core::{TeamId, Vec3};
use std::sync::Arc;

#[path = "energy_edits.rs"]
mod energy_edits;

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::ZERO,
        z: Fixed::from_int(z),
    }
}

const AHEAD: (Fixed, Fixed) = (Fixed::ONE, Fixed::ZERO);

/// Seats 1 and 3 are allies (team 1), seat 2 the enemy (team 2).
fn world() -> (Sim, Arc<ContentDb>) {
    let db = Arc::new(fixture::db());
    let seat = |id: u8, team: u8| PlayerSetup {
        id: PlayerId(id),
        team: TeamId(team),
        race: "fx".into(),
        control: Control::Idle,
    };
    let setup = GameSetup {
        seed: 3,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::clone(&db),
        players: vec![seat(1, 1), seat(2, 2), seat(3, 1)],
        options: GameOptions::default(),
    };
    (Sim::from_setup(&setup), db)
}

/// A cloaker of `odf` for seat 1 at the origin facing +x, an enemy frigate
/// of seat 2 `gap` ahead and an ally frigate of seat 3 behind. Beams reach 600.
fn scene(odf: &str, gap: i32) -> (Sim, EntityId, EntityId, EntityId) {
    let (mut sim, db) = world();
    let k = |n: &str| db.key(n).unwrap();
    let c = sim.spawn_def(k(odf), PlayerId(1), v(0, 0), AHEAD).unwrap();
    let e = sim
        .spawn_def(
            k("fx_frigate"),
            PlayerId(2),
            v(gap, 0),
            (-Fixed::ONE, Fixed::ZERO),
        )
        .unwrap();
    let a = sim
        .spawn_def(k("fx_frigate"), PlayerId(3), v(-100, 0), AHEAD)
        .unwrap();
    (sim, c, e, a)
}

fn cloak(player: u8, id: EntityId, on: bool) -> Command {
    Command {
        player: PlayerId(player),
        seq: 0,
        kind: CommandKind::Order(Order::Cloak {
            units: vec![id],
            on,
        }),
    }
}

fn special(player: u8, id: EntityId, slot: u8) -> Command {
    Command {
        player: PlayerId(player),
        seq: 0,
        kind: CommandKind::Order(Order::Special {
            units: vec![id],
            slot,
            target: crate::command::Target::None,
        }),
    }
}

fn sees(view: &WorldView, player: u8, id: EntityId) -> bool {
    view.visibility
        .iter()
        .find(|x| x.0 == PlayerId(player))
        .is_some_and(|x| x.1.contains(&id))
}

/// Step `n` ticks (the first with `cmds`); every `(step number from 1, event)`.
fn run(sim: &mut Sim, cmds: &[Command], n: u64) -> Vec<(u64, SimEvent)> {
    let mut out = Vec::new();
    for i in 1..=n {
        let o = sim.step(if i == 1 { cmds } else { &[] });
        out.extend(o.events.into_iter().map(|e| (i, e)));
    }
    out
}

fn cloak_events(ev: &[(u64, SimEvent)]) -> Vec<(u64, bool)> {
    ev.iter()
        .filter_map(|(t, e)| match e {
            SimEvent::CloakChanged { on, .. } => Some((*t, *on)),
            _ => None,
        })
        .collect()
}

fn fired_at(ev: &[(u64, SimEvent)], id: EntityId) -> Vec<u64> {
    ev.iter()
        .filter_map(|(t, e)| match e {
            SimEvent::Fired {
                target: crate::event::Target::Entity(t2),
                ..
            } if *t2 == id => Some(*t),
            _ => None,
        })
        .collect()
}

fn fired_by(ev: &[(u64, SimEvent)], id: EntityId) -> Vec<u64> {
    ev.iter()
        .filter_map(|(t, e)| match e {
            SimEvent::Fired { shooter, .. } if *shooter == id => Some(*t),
            _ => None,
        })
        .collect()
}

#[test]
fn engage_takes_the_timer_then_hides_from_enemies_only() {
    let (mut sim, c, _, _) = scene("fx_cloaker", 900);
    let ev = run(&mut sim, &[cloak(1, c, true)], 29);
    assert!(cloak_events(&ev).is_empty());
    assert_eq!(sim.specials.cloak_state(c), Some(CloakState::Engaging(1)));
    assert!(sees(&sim.view(), 2, c), "still visible while engaging");
    let ev = run(&mut sim, &[], 1);
    assert_eq!(cloak_events(&ev), [(1, true)]);
    let view = sim.view();
    assert!(
        view.units
            .iter()
            .find(|u| u.id == c)
            .unwrap()
            .flags
            .contains(UnitFlags::CLOAKED)
    );
    assert!(!sees(&view, 2, c), "the enemy does not see it");
    assert!(sees(&view, 1, c) && sees(&view, 3, c), "owner and ally do");
}

#[test]
fn energy_is_paid_and_drains_while_cloaked() {
    let (mut sim, c, _, _) = scene("fx_cloaker", 2000);
    run(&mut sim, &[], 1);
    assert_eq!(sim.specials.energy_of(c), Some(Fixed::from_int(600)));
    run(&mut sim, &[cloak(1, c, true)], 1);
    // Paid 50 at the order, then one tick of drain (0.5) and regeneration (0.2).
    let want = 550.0 - 0.5 + 0.2;
    let got = sim.specials.energy_of(c).unwrap().to_f32();
    assert!((got - want).abs() < 0.01, "{got}");
    // Net loss per tick while cloaked is 0.3.
    run(&mut sim, &[], 99);
    let got = sim.specials.energy_of(c).unwrap().to_f32();
    assert!((got - (want - 99.0 * 0.3)).abs() < 0.05, "{got}");
}

#[test]
fn cloaked_units_do_not_fire_and_are_not_targeted() {
    // Enemy 400 ahead: both are in each other's beam range and arc.
    let (mut sim, c, e, _) = scene("fx_cloaker", 400);
    let open = run(&mut sim, &[], 1);
    assert_eq!(fired_by(&open, c), [1]);
    assert_eq!(fired_by(&open, e), [1]);
    let (mut sim, c, _, _) = scene("fx_cloaker", 400);
    let ev = run(&mut sim, &[cloak(1, c, true)], 120);
    assert!(
        fired_by(&ev, c).is_empty(),
        "weapons are off from the order on"
    );
    let early = fired_at(&ev, c);
    assert!(
        !early.is_empty() && early.iter().all(|t| *t <= 30),
        "the enemy shoots it until the cloak is up: {early:?}"
    );
    assert!(
        !fired_by(&ev, e).is_empty(),
        "the enemy still shoots the visible ally"
    );
    assert!(sim.unit(c).unwrap().flags.contains(UnitFlags::WEAPONS_OFF));
}

#[test]
fn disengage_shows_the_unit_and_returns_weapons_after_the_timer() {
    let (mut sim, c, e, _) = scene("fx_cloaker", 400);
    run(&mut sim, &[cloak(1, c, true)], 60);
    assert!(!sees(&sim.view(), 2, c));
    let ev = run(&mut sim, &[cloak(1, c, false)], 1);
    assert_eq!(cloak_events(&ev), [(1, false)], "visible at once");
    assert!(sees(&sim.view(), 2, c));
    assert!(
        sim.unit(c).unwrap().flags.contains(UnitFlags::WEAPONS_OFF),
        "weapons still off"
    );
    let ev = run(&mut sim, &[], 40);
    assert!(sim.specials.cloak_state(c).is_none());
    assert!(!sim.unit(c).unwrap().flags.contains(UnitFlags::WEAPONS_OFF));
    assert!(sim.specials.muted.is_empty());
    let shots = fired_by(&ev, c);
    assert!(
        !shots.is_empty() && shots[0] >= 29,
        "fires again once the timer ends: {shots:?}"
    );
    let _ = e;
}

#[test]
fn bench_weapons_off_survives_a_cloak_cycle() {
    let (mut sim, c, _, _) = scene("fx_cloaker", 2000);
    sim.units.get_mut(c).unwrap().flags = sim.unit(c).unwrap().flags | UnitFlags::WEAPONS_OFF;
    run(&mut sim, &[cloak(1, c, true)], 40);
    run(&mut sim, &[cloak(1, c, false)], 40);
    assert!(sim.unit(c).unwrap().flags.contains(UnitFlags::WEAPONS_OFF));
}

#[test]
fn cancelling_an_engage_never_shows_a_cloak() {
    let (mut sim, c, _, _) = scene("fx_cloaker", 2000);
    let mut ev = run(&mut sim, &[cloak(1, c, true)], 10);
    ev.extend(run(&mut sim, &[cloak(1, c, false)], 60));
    assert!(cloak_events(&ev).is_empty());
    assert!(sim.specials.cloak_state(c).is_none());
    assert!(!sim.unit(c).unwrap().flags.contains(UnitFlags::WEAPONS_OFF));
}

#[test]
fn running_out_of_energy_decloaks() {
    let (mut sim, c, _, _) = scene("fx_cloaker_frail", 2000);
    // 100 - 10 = 90 left, 1 a tick: cloaked at step 30, dry at step 90.
    let ev = run(&mut sim, &[cloak(1, c, true)], 150);
    assert_eq!(cloak_events(&ev), [(30, true), (90, false)]);
    assert_eq!(sim.specials.energy_of(c), Some(Fixed::ZERO));
    // Dry, no regeneration: the next order is refused.
    let out = sim.step(&[cloak(1, c, true)]);
    assert!(
        out.events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
}

#[test]
fn a_hit_decloaks_only_when_the_data_says_so() {
    let hit = |target| SimEvent::Hit {
        target,
        by: EntityId(99),
        weapon: sta_core::ContentKey(0),
        facing: 0,
        shield: Fixed::ZERO,
        hull: Fixed::ONE,
        at: v(0, 0),
    };
    for (odf, decloaks) in [("fx_cloaker_loud", true), ("fx_cloaker", false)] {
        let (mut sim, c, _, _) = scene(odf, 3000);
        run(&mut sim, &[cloak(1, c, true)], 40);
        assert_eq!(sim.specials.cloak_state(c), Some(CloakState::Cloaked));
        let db = sim.content().unwrap().clone();
        let mut events = vec![hit(c)];
        sim.specials.react(&mut sim.units, &db, &mut events, 0);
        assert_eq!(
            sim.unit(c).unwrap().flags.contains(UnitFlags::CLOAKED),
            !decloaks,
            "{odf}"
        );
        assert_eq!(
            events.contains(&SimEvent::CloakChanged { id: c, on: false }),
            decloaks,
            "{odf}"
        );
    }
}

#[test]
fn a_unit_that_fires_from_its_cloak_is_revealed() {
    // Enemy out of the cloaker's reach at first; the loud cloaker is
    // cloaked, then the enemy closes in and the cloaker fires on its order.
    let (mut sim, c, e, _) = scene("fx_cloaker_loud", 3000);
    run(&mut sim, &[cloak(1, c, true)], 2);
    assert_eq!(sim.specials.cloak_state(c), Some(CloakState::Cloaked));
    assert!(!sees(&sim.view(), 2, c));
    // Move the enemy next to it by an edit-free route: teleport in the test.
    sim.units.get_mut(e).unwrap().pos = v(300, 0);
    let ev = run(&mut sim, &[], 2);
    // The ally-team cloaker fires (enemy is visible to it), which decloaks it.
    assert!(!fired_by(&ev, c).is_empty(), "{ev:?}");
    assert!(
        cloak_events(&ev).contains(&(1, false)) || cloak_events(&ev).contains(&(2, false)),
        "{ev:?}"
    );
    assert!(!sim.unit(c).unwrap().flags.contains(UnitFlags::CLOAKED));
}

#[test]
fn only_the_owner_orders_and_only_cloak_capable_units() {
    let (mut sim, c, e, _) = scene("fx_cloaker", 2000);
    let out = sim.step(&[cloak(2, c, true), cloak(2, e, true), cloak(1, c, false)]);
    let rejected = out
        .events
        .iter()
        .filter(|e| matches!(e, SimEvent::Rejected { .. }))
        .count();
    assert_eq!(rejected, 3, "foreign, no cloak, nothing to disengage");
    assert!(sim.specials.cloak_state(c).is_none());
}

#[test]
fn special_slot_zero_toggles_the_cloak() {
    let (mut sim, c, _, _) = scene("fx_cloaker", 2000);
    let ev = run(&mut sim, &[special(1, c, 0)], 30);
    assert_eq!(cloak_events(&ev), [(30, true)]);
    let ev = run(&mut sim, &[special(1, c, 0)], 1);
    assert_eq!(cloak_events(&ev), [(1, false)]);
    let out = sim.step(&[special(1, c, 1)]);
    assert!(
        out.events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. })),
        "no second special"
    );
}

#[test]
fn destroyed_units_leave_no_rows() {
    let (mut sim, c, _, _) = scene("fx_cloaker", 2000);
    run(&mut sim, &[cloak(1, c, true)], 5);
    sim.despawn(c, None);
    run(&mut sim, &[], 1);
    assert!(sim.specials.is_empty());
}

#[test]
fn sims_without_specials_keep_their_digest() {
    let mut a = Sim::new(1);
    let mut b = Sim::new(1);
    a.step(&[]);
    b.step(&[]);
    assert!(a.specials.is_empty());
    assert_eq!(a.digest(), b.digest());
    let (mut sim, c, _, _) = scene("fx_cloaker", 2000);
    let before = sim.digest();
    run(&mut sim, &[cloak(1, c, true)], 1);
    assert_ne!(before, sim.digest());
}

#[test]
fn mid_cloak_snapshot_continues_identically() {
    let (mut live, _) = {
        let (sim, c, _, _) = scene("fx_cloaker", 400);
        (sim, c)
    };
    let c = EntityId(1);
    run(&mut live, &[cloak(1, c, true)], 15);
    assert_eq!(live.specials.cloak_state(c), Some(CloakState::Engaging(15)));
    let db = live.content().unwrap().clone();
    let mut loaded = Sim::from_snapshot(&live.snapshot(), Some(Arc::clone(&db))).unwrap();
    assert_eq!(loaded.digest(), live.digest());
    assert_eq!(loaded.specials.to_rec(), live.specials.to_rec());
    let mut saw_cloak = false;
    for tick in 0..120 {
        // Disengage part way through so both halves of the state are walked.
        let cmds = if tick == 60 {
            vec![cloak(1, c, false)]
        } else {
            vec![]
        };
        let a = live.step(&cmds);
        let b = loaded.step(&cmds);
        assert_eq!(a.view.digest, b.view.digest, "tick {tick}");
        assert_eq!(a.events, b.events, "tick {tick}");
        saw_cloak |= a
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::CloakChanged { .. }));
    }
    assert!(saw_cloak);
    assert_eq!(loaded.snapshot(), live.snapshot());
    // And at the cloaked stage, with the state read back.
    let (mut live, _, _, _) = scene("fx_cloaker", 400);
    run(&mut live, &[cloak(1, c, true)], 50);
    assert_eq!(live.specials.cloak_state(c), Some(CloakState::Cloaked));
    let loaded = Sim::from_snapshot(&live.snapshot(), Some(db)).unwrap();
    assert_eq!(loaded.specials.cloak_state(c), Some(CloakState::Cloaked));
    assert_eq!(loaded.specials.energy_of(c), live.specials.energy_of(c));
    assert!(loaded.unit(c).unwrap().flags.contains(UnitFlags::CLOAKED));
}
