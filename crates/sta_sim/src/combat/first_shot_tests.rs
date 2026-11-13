//! SIM-53: when the first shot after an order, or after the mission starts,
//! leaves. Measured on stock A2 1.1 and Roots + STA2 Classic with paired
//! scenes (`bench/pairs/first-shot.ini`): an `Attack` order adds no delay;
//! a new unit fires when its initial reload (SIM-35) runs out; a re-order
//! onto another target takes effect at each mount's next shot without
//! resetting its timer; and nothing fires in the first two seconds of
//! mission time (`start_fire_hold`). The engine code behind the hold was not
//! found (measured only).
use std::sync::Arc;

use sta_content::{ContentDb, Features};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use super::weapons::START_HOLD_TICKS;
use crate::command::{Command, CommandKind, Order};
use crate::event::{SimEvent, Target};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};

const AHEAD: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

/// The fake content, shot timing on, the hold as asked, every `fx_beam`
/// with `initialShotDelayModifier` = `modifier` and no jitter.
fn db(hold: bool, modifier: Option<f64>) -> Arc<ContentDb> {
    db_with(hold, modifier, None)
}

/// [`db`] with `fx_beam`'s refire set to `refire` ticks.
fn db_with(hold: bool, modifier: Option<f64>, refire: Option<u64>) -> Arc<ContentDb> {
    let fake = sta_content::fake::db();
    let mut f: Features = fake.features().clone();
    f.weapon_shot_timing = true;
    f.start_fire_hold = hold;
    f.fire_arcs = false;
    let mut b = ContentDb::builder().profile(fake.profile()).features(f);
    for mut w in fake.weapons().cloned() {
        if w.odf == "fx_beam" {
            w.initial_shot_delay_modifier = modifier.map(Fixed::from_f64);
            w.ignore_shot_delay_effects = true;
            if let Some(r) = refire {
                w.refire = r;
            }
        }
        b = b.weapon(w);
    }
    for u in fake.units().cloned() {
        b = b.unit(u);
    }
    Arc::new(b.build().unwrap())
}

fn sim(db: &Arc<ContentDb>) -> Sim {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    Sim::from_setup(&GameSetup {
        seed: 11,
        map: Arc::new(sta_content::MapDef::empty()),
        content: db.clone(),
        players: vec![seat(1), seat(2)],
        options: GameOptions::default(),
    })
}

/// (step index, shooter, target) of every `Fired` over `ticks` steps.
fn fired(sim: &mut Sim, ticks: u64, cmds: &mut dyn FnMut(u64) -> Vec<Command>) -> Vec<(u64, EntityId, EntityId)> {
    let mut out = Vec::new();
    for t in 0..ticks {
        for e in sim.step(&cmds(t)).events {
            if let SimEvent::Fired { shooter, target: Target::Entity(target), .. } = e {
                out.push((t, shooter, target));
            }
        }
    }
    out
}

fn first_of(f: &[(u64, EntityId, EntityId)], id: EntityId) -> u64 {
    f.iter().find(|x| x.1 == id).expect("it fired").0
}

#[test]
fn nothing_fires_in_the_first_two_seconds_of_the_mission() {
    for (hold, want) in [(false, 0), (true, START_HOLD_TICKS)] {
        let db = db(hold, Some(0.0)); // ready at creation
        let mut s = sim(&db);
        let a = s.spawn_def(db.key("fx_frigate").unwrap(), PlayerId(1), v(2000, 2000), AHEAD).unwrap();
        s.spawn_def(db.key("fx_station").unwrap(), PlayerId(2), v(2000, 2300), AHEAD).unwrap();
        let f = fired(&mut s, 90, &mut |_| vec![]);
        assert_eq!(first_of(&f, a), want, "hold {hold}");
    }
}

#[test]
fn an_attack_order_adds_no_delay_and_a_late_unit_is_not_held() {
    let db = db(true, Some(0.0));
    let mut s = sim(&db);
    let st = s.spawn_def(db.key("fx_station").unwrap(), PlayerId(2), v(2000, 2300), AHEAD).unwrap();
    fired(&mut s, 120, &mut |_| vec![]);
    // A frigate made after the hold, ordered on its first tick: it fires on
    // that tick (initial reload 0), with no order latency.
    let a = s.spawn_def(db.key("fx_frigate").unwrap(), PlayerId(1), v(2000, 2000), AHEAD).unwrap();
    let order = Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Attack { units: vec![a], target: st }) };
    let f = fired(&mut s, 10, &mut |t| if t == 0 { vec![order.clone()] } else { vec![] });
    assert_eq!(f.iter().find(|x| x.1 == a), Some(&(0, a, st)));
}

#[test]
fn the_initial_reload_still_runs_from_creation_under_the_hold() {
    // Random initial reloads (key absent) of U[0,1) x 150 ticks: a draw that
    // ran out during the hold fires on the first tick after it, a longer one
    // on its own tick counted from creation (not from the hold's end).
    let refire = 150;
    let db = db_with(true, None, Some(refire));
    let mut s = sim(&db);
    let ids: Vec<EntityId> =
        (0..24).map(|i| s.spawn_def(db.key("fx_frigate").unwrap(), PlayerId(1), v(2000 + 10 * i, 2000), AHEAD).unwrap()).collect();
    s.spawn_def(db.key("fx_station").unwrap(), PlayerId(2), v(2100, 2300), AHEAD).unwrap();
    let f = fired(&mut s, 2 * refire, &mut |_| vec![]);
    let firsts: Vec<u64> = ids.iter().map(|&i| first_of(&f, i)).collect();
    assert!(firsts.iter().all(|t| (START_HOLD_TICKS..refire).contains(t)), "{firsts:?}");
    let held = firsts.iter().filter(|t| **t == START_HOLD_TICKS).count();
    let later = firsts.iter().filter(|t| **t > START_HOLD_TICKS).count();
    // With 24 draws both kinds occur (seeded): some held, some after the hold.
    assert!(held > 0 && later > 0, "{firsts:?}");
}

#[test]
fn a_reorder_takes_effect_at_the_next_shot_without_resetting_the_timer() {
    let db = db(false, Some(0.0));
    let refire = db.weapon(db.key("fx_beam").unwrap()).unwrap().refire;
    let mut s = sim(&db);
    let a = s.spawn_def(db.key("fx_frigate").unwrap(), PlayerId(1), v(2000, 2000), AHEAD).unwrap();
    let near = s.spawn_def(db.key("fx_station").unwrap(), PlayerId(2), v(2000, 2250), AHEAD).unwrap();
    let far = s.spawn_def(db.key("fx_station").unwrap(), PlayerId(2), v(2200, 2250), AHEAD).unwrap();
    // Re-ordered half way through a reload.
    let at = refire + refire / 2;
    let order = Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Attack { units: vec![a], target: far }) };
    let f = fired(&mut s, 3 * refire + 1, &mut |t| if t == at { vec![order.clone()] } else { vec![] });
    let mine: Vec<(u64, EntityId)> = f.iter().filter(|x| x.1 == a).map(|x| (x.0, x.2)).collect();
    assert_eq!(mine, vec![(0, near), (refire, near), (2 * refire, far), (3 * refire, far)]);
}
