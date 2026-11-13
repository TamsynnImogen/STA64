//! SIM-34 (point-defence energy and rate) and SIM-35 (initial reload, shot
//! jitter, special orders while reloading). Implements D5, D9.
//!
//! Evidence grades: PD energy drain and switch-off, the stock "no rate limit"
//! and the Roots debt timer are measured; Roots switch-on needs energy >=
//! cost and the FO `ordnancedefenseweapon` timer are read from the engine's
//! code only (not measured); the initial-reload rule is measured on a special
//! launcher only. TOOL-16 parity scenes are not covered (TOOL-16 is not
//! merged).
use std::sync::Arc;

use sta_content::{ContentDb, Features, UnitDef, WeaponDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};

use crate::command::{Command, CommandKind, Edit, Order};
use crate::event::{SimEvent, Target};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::ZERO,
        z: Fixed::from_int(z),
    }
}

const AHEAD: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);

/// The fake content with `features`, `units` and `weapons` edited by `edit`.
fn content(
    features: impl FnOnce(&mut Features),
    edit: impl FnOnce(&mut Vec<UnitDef>, &mut Vec<WeaponDef>),
) -> ContentDb {
    let fake = sta_content::fake::db();
    let mut f = fake.features().clone();
    features(&mut f);
    let mut units: Vec<UnitDef> = fake.units().cloned().collect();
    let mut weapons: Vec<WeaponDef> = fake.weapons().cloned().collect();
    edit(&mut units, &mut weapons);
    let mut b = ContentDb::builder().profile(fake.profile()).features(f);
    for w in weapons {
        b = b.weapon(w);
    }
    for u in units {
        b = b.unit(u);
    }
    b.build().unwrap()
}

fn seat(id: u8) -> PlayerSetup {
    PlayerSetup {
        id: PlayerId(id),
        team: TeamId(id),
        race: "fx".into(),
        control: Control::Idle,
    }
}

fn cmd(player: PlayerId, kind: CommandKind) -> Command {
    Command {
        player,
        seq: 0,
        kind,
    }
}

// ---------------------------------------------------------------- SIM-34

/// `pd_energy` (and the Roots debt timer, and the FO class) on; the station's
/// PD costs 40/s from a pool of 500 refilling 10/s.
fn pd_db(debt: bool, fo_class: bool) -> ContentDb {
    content(
        |f| {
            f.pd_energy = true;
            f.pd_debt_timer = debt;
        },
        |units, weapons| {
            for w in weapons.iter_mut().filter(|w| w.odf == "fx_pd") {
                w.energy_cost = Fixed::from_int(40);
                w.ordnance_defence = fo_class;
            }
            for u in units.iter_mut().filter(|u| u.odf == "fx_station") {
                // `maxSpecialEnergy = 500`, `specialEnergyRate = 10`, as the loader types them.
                u.special_energy_max = Fixed::from_int(500);
                u.special_energy_rate = Fixed::from_int(10);
            }
        },
    )
}

/// `boats` torpedo cruisers of player 1 and the PD station of player 2
/// 800 away, as in `pd_tests`.
fn pd_scene(db: ContentDb, boats: usize) -> (Sim, Vec<EntityId>, EntityId) {
    let (cruiser, station) = (db.key("fx_cruiser").unwrap(), db.key("fx_station").unwrap());
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 7,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::new(db),
        players: vec![seat(1), seat(2)],
        options: GameOptions::default(),
    });
    let ids = (0..boats)
        .map(|_| sim.spawn_def(cruiser, PlayerId(1), v(2000, 2000), AHEAD).unwrap())
        .collect();
    let st = sim.spawn_def(station, PlayerId(2), v(2000, 2800), AHEAD).unwrap();
    (sim, ids, st)
}

fn toggle_pd(st: EntityId) -> Command {
    cmd(
        PlayerId(2),
        CommandKind::Order(Order::Special {
            units: vec![st],
            slot: 0,
            target: Target::Point(v(0, 0)),
        }),
    )
}

/// PD `Fired` ticks and `Hit` ticks on the station over `ticks` ticks, the
/// PD switched on at tick `on_at` and the boats ordered to attack at 0.
fn run(sim: &mut Sim, boats: Vec<EntityId>, st: EntityId, on_at: u64, ticks: u64) -> (Vec<u64>, Vec<u64>) {
    let (mut pd, mut hits) = (vec![], vec![]);
    for t in 0..ticks {
        let mut cmds = vec![];
        if t == 0 && !boats.is_empty() {
            cmds.push(cmd(
                PlayerId(1),
                CommandKind::Order(Order::Attack {
                    units: boats.clone(),
                    target: st,
                }),
            ));
        }
        if t == on_at {
            cmds.push(toggle_pd(st));
        }
        for e in sim.step(&cmds).events {
            match e {
                SimEvent::Fired { shooter, hardpoint: 1, .. } if shooter == st => pd.push(t),
                SimEvent::Hit { target, .. } if target == st => hits.push(t),
                _ => {}
            }
        }
    }
    (pd, hits)
}

#[test]
fn pd_is_off_until_toggled_and_drains_energy_per_second_then_switches_off() {
    let (mut sim, _, st) = pd_scene(pd_db(false, false), 0);
    sim.step(&[]);
    assert!(!sim.weapons.pd_on(st, 1));
    assert_eq!(sim.specials.energy_of(st), Some(Fixed::from_int(500)));
    // On, with nothing to shoot: -40/s drain, +10/s recharge, so -1 per tick.
    run(&mut sim, vec![], st, 0, 30);
    assert!(sim.weapons.pd_on(st, 1));
    let e = sim.specials.energy_of(st).unwrap().to_f32();
    assert!((469.0..471.0).contains(&e), "{e}");
    run(&mut sim, vec![], st, u64::MAX, 400);
    assert!(sim.weapons.pd_on(st, 1), "still on with energy left");
    run(&mut sim, vec![], st, u64::MAX, 120);
    assert!(!sim.weapons.pd_on(st, 1), "off at 0 energy");
}

#[test]
fn switching_on_needs_energy_above_cost_on_roots_and_above_zero_on_stock() {
    for (debt, low, expect) in [(true, 39, false), (true, 40, true), (false, 1, true), (false, 0, false)] {
        let (mut sim, _, st) = pd_scene(pd_db(debt, false), 0);
        sim.step(&[cmd(
            PlayerId::BENCH,
            CommandKind::Edit(Edit::SetEnergy(st, Fixed::from_int(low))),
        )]);
        sim.step(&[toggle_pd(st)]);
        assert_eq!(sim.weapons.pd_on(st, 1), expect, "debt {debt}, energy {low}");
    }
}

#[test]
fn stock_pd_takes_every_eligible_shot_with_no_rate_limit() {
    let (mut sim, boats, st) = pd_scene(pd_db(false, false), 3);
    let (pd, hits) = run(&mut sim, boats, st, 0, 80);
    assert_eq!(pd.len(), 3, "{pd:?}");
    assert!(pd.iter().all(|t| *t == pd[0]), "all on one tick: {pd:?}");
    assert!(hits.is_empty());
}

#[test]
fn roots_pd_pays_its_idle_debt_in_a_burst_but_a_fresh_switch_on_is_rate_limited() {
    // Active for 43 idle ticks: the timer is about -44, three shots clear it.
    let (mut sim, boats, st) = pd_scene(pd_db(true, false), 3);
    let (pd, hits) = run(&mut sim, boats, st, 0, 80);
    assert_eq!(pd.len(), 3, "{pd:?}");
    assert!(pd.iter().all(|t| *t == pd[0]), "{pd:?}");
    assert!(hits.is_empty());
    // Switched on two ticks before the first torpedo is in range: a small
    // debt, one shot, then one per shotDelay (10 ticks) plus jitter.
    let (mut sim2, boats, st) = pd_scene(pd_db(true, false), 3);
    let (pd, _) = run(&mut sim2, boats, st, 41, 80);
    assert!(pd.len() >= 2, "{pd:?}");
    assert!((7..=8).contains(&(pd[1] - pd[0])), "{pd:?}");
    // Same seed, same run.
    let (mut again, boats, st) = pd_scene(pd_db(true, false), 3);
    run(&mut again, boats, st, 41, 80);
    assert_eq!(again.digest(), sim2.digest());
    let _ = &mut sim;
}

#[test]
fn fo_class_resets_the_timer_to_the_raw_shot_delay_after_each_shot() {
    // NOT MEASURED: read from the engine's code only. No debt carries over.
    let (mut sim, boats, st) = pd_scene(pd_db(true, true), 3);
    let (pd, hits) = run(&mut sim, boats, st, 0, 80);
    assert_eq!(pd, [43, 53], "{pd:?}");
    assert_eq!(hits, [61]);
}

/// Three cruisers against the PD station, PD on, 45 ticks in; for the
/// snapshot test.
pub(crate) fn mid_saturation_energy() -> (Sim, Arc<ContentDb>) {
    let (mut sim, boats, st) = pd_scene(pd_db(true, false), 3);
    run(&mut sim, boats, st, 41, 45);
    let db = Arc::clone(sim.content().unwrap());
    (sim, db)
}

#[test]
fn pd_rows_round_trip_and_legacy_rows_load_unchanged() {
    use super::pd::Pd;
    let (sim, _) = mid_saturation_energy();
    let rec = sim.weapons.pd_rec();
    assert_eq!(rec[0].1[0], u64::MAX, "energy-model row");
    assert_eq!(Pd::from_rec(rec.clone()).unwrap().to_rec(), rec);
    let legacy = vec![(EntityId(3), vec![5, 6])];
    assert_eq!(Pd::from_rec(legacy.clone()).unwrap().to_rec(), legacy);
    assert!(Pd::from_rec(vec![(EntityId(3), vec![u64::MAX, 1])]).is_none());
}

// ---------------------------------------------------------------- SIM-35

const SHOOTERS: i32 = 20;

/// `SHOOTERS` frigates of player 1 side by side, 400 from an enemy station;
/// every one has the same beam (refire 45 ticks) with `modifier` as its
/// `initialShotDelayModifier`. First and later `Fired` ticks per shooter over
/// `ticks` ticks.
fn volleys(timing: bool, modifier: Option<f64>, ignore: bool, ticks: u64) -> Vec<Vec<u64>> {
    let db = content(
        |f| {
            f.weapon_shot_timing = timing;
            f.fire_arcs = false;
        },
        |_, weapons| {
            for w in weapons.iter_mut().filter(|w| w.odf == "fx_beam") {
                w.initial_shot_delay_modifier = modifier.map(Fixed::from_f64);
                w.ignore_shot_delay_effects = ignore;
            }
        },
    );
    let (frigate, station) = (db.key("fx_frigate").unwrap(), db.key("fx_station").unwrap());
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 11,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::new(db),
        players: vec![seat(1), seat(2)],
        options: GameOptions::default(),
    });
    let ids: Vec<EntityId> = (0..SHOOTERS)
        .map(|i| sim.spawn_def(frigate, PlayerId(1), v(2000 + 10 * i, 2000), AHEAD).unwrap())
        .collect();
    sim.spawn_def(station, PlayerId(2), v(2100, 2400), AHEAD).unwrap();
    let mut out = vec![vec![]; ids.len()];
    for t in 0..ticks {
        for e in sim.step(&[]).events {
            if let SimEvent::Fired { shooter, .. } = e
                && let Some(k) = ids.iter().position(|i| *i == shooter)
            {
                out[k].push(t);
            }
        }
    }
    out
}

fn firsts(v: &[Vec<u64>]) -> Vec<u64> {
    v.iter().map(|s| *s.first().expect("every shooter fired")).collect()
}

fn spread(f: &[u64]) -> u64 {
    f.iter().max().unwrap() - f.iter().min().unwrap()
}

#[test]
fn without_the_feature_weapons_are_ready_at_once() {
    let f = firsts(&volleys(false, None, false, 20));
    assert!(f.iter().all(|t| *t == 0), "{f:?}");
}

#[test]
fn initial_reload_follows_the_modifier() {
    // Key absent (the engine reads -1): U[0, 1) x shotDelay.
    let f = firsts(&volleys(true, None, false, 100));
    assert!(f.iter().all(|t| *t < 45), "{f:?}");
    assert!(spread(&f) > 10, "random, not constant: {f:?}");
    // 0: ready at once (Classic ships 0.0, which is not stock).
    let f = firsts(&volleys(true, Some(0.0), false, 100));
    assert!(f.iter().all(|t| *t == 0), "{f:?}");
    // 1: one whole shotDelay.
    let f = firsts(&volleys(true, Some(1.0), false, 100));
    assert!(f.iter().all(|t| *t == 45), "{f:?}");
    // 0.5: half of it.
    let f = firsts(&volleys(true, Some(0.5), false, 100));
    assert!(f.iter().all(|t| *t == 22), "{f:?}");
    // -0.5: U[0, 0.5) x shotDelay.
    let f = firsts(&volleys(true, Some(-0.5), false, 100));
    assert!(f.iter().all(|t| *t < 23), "{f:?}");
    assert!(spread(&f) > 5, "{f:?}");
    // Deterministic.
    assert_eq!(volleys(true, None, false, 100), volleys(true, None, false, 100));
}

#[test]
fn every_shot_reloads_shot_delay_plus_up_to_five_percent() {
    let shots = volleys(true, Some(0.0), false, 300);
    let gaps: Vec<u64> = shots.iter().flat_map(|s| s.windows(2).map(|w| w[1] - w[0])).collect();
    assert!(gaps.len() > 40);
    assert!(gaps.iter().all(|g| (45..=47).contains(g)), "{gaps:?}");
    assert!(gaps.iter().any(|g| *g > 45), "{gaps:?}");
    // ignoreShotDelayEffects: no jitter.
    let shots = volleys(true, Some(0.0), true, 300);
    assert!(shots.iter().all(|s| s.windows(2).all(|w| w[1] - w[0] == 45)));
}

#[test]
fn a_special_order_while_reloading_is_refused_not_queued() {
    let fake_db = crate::specials::fixture::db();
    let mut f = fake_db.features().clone();
    f.weapon_shot_timing = true;
    let (units, weapons) = crate::specials::fixture::defs();
    let mut b = ContentDb::builder().profile(fake_db.profile()).features(f);
    // `specials::fixture::db()` is the fake content plus the fixture: its
    // own definitions already include the fixture's, so edit those.
    let _ = (units, &weapons);
    for mut w in fake_db.weapons().cloned() {
        if w.odf.starts_with("fx_cloak") {
            w.refire = 90;
            w.initial_shot_delay_modifier = Some(Fixed::ONE);
        }
        b = b.weapon(w);
    }
    for u in fake_db.units().cloned() {
        b = b.unit(u);
    }
    let db = b.build().unwrap();
    let cloaker = db.key("fx_cloaker").unwrap();
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::new(db),
        players: vec![seat(1)],
        options: GameOptions::default(),
    });
    let c = sim.spawn_def(cloaker, PlayerId(1), v(0, 0), AHEAD).unwrap();
    let order = || {
        cmd(
            PlayerId(1),
            CommandKind::Order(Order::Special {
                units: vec![c],
                slot: 0,
                target: Target::Point(v(0, 0)),
            }),
        )
    };
    sim.step(&[]);
    for _ in 0..10 {
        sim.step(&[]);
    }
    sim.step(&[order()]);
    assert!(sim.specials.cloak_state(c).is_none(), "refused while reloading");
    // Not queued either: nothing happens on its own later.
    for _ in 0..100 {
        sim.step(&[]);
    }
    assert!(sim.specials.cloak_state(c).is_none());
    sim.step(&[order()]);
    assert!(sim.specials.cloak_state(c).is_some(), "accepted once ready");
}

/// The fixture cloaker with a PD hardpoint ahead of its cloak.
fn pd_then_cloak(pd_energy: bool) -> (Sim, EntityId) {
    let fixture = crate::specials::fixture::db();
    let mut f = fixture.features().clone();
    f.pd_energy = pd_energy;
    let mut b = ContentDb::builder().profile(fixture.profile()).features(f);
    for w in fixture.weapons().cloned() {
        b = b.weapon(w);
    }
    for mut u in fixture.units().cloned() {
        if u.odf == "fx_cloaker" {
            let pd = sta_content::Hardpoint {
                weapon: sta_content::DefRef::new("fx_pd"),
                node: "hp03".into(),
                arc: None,
            };
            u.hardpoints.insert(0, pd);
        }
        b = b.unit(u);
    }
    let db = b.build().unwrap();
    let key = db.key("fx_cloaker").unwrap();
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::new(db),
        players: vec![seat(1)],
        options: GameOptions::default(),
    });
    let c = sim.spawn_def(key, PlayerId(1), v(0, 0), AHEAD).unwrap();
    (sim, c)
}

#[test]
fn a_pd_hardpoint_takes_a_special_slot_only_under_pd_energy() {
    let order = |c| {
        cmd(
            PlayerId(1),
            CommandKind::Order(Order::Special {
                units: vec![c],
                slot: 0,
                target: Target::Point(v(0, 0)),
            }),
        )
    };
    // Without pd_energy the cloak keeps slot 0 (recorded orders stay valid).
    let (mut sim, c) = pd_then_cloak(false);
    sim.step(&[order(c)]);
    assert!(sim.specials.cloak_state(c).is_some());
    // With it the PD comes first and slot 0 toggles it, not the cloak.
    let (mut sim, c) = pd_then_cloak(true);
    sim.step(&[order(c)]);
    assert!(sim.specials.cloak_state(c).is_none());
    assert!(sim.weapons.pd_on(c, 0));
}

#[test]
fn a_unit_that_starts_with_weapons_off_or_is_ordered_first_still_gets_its_initial_reload() {
    // Order on the spawn tick, before any weapons step: the cloak (modifier 1,
    // shotDelay 90) is refused, not ready at once.
    let fixture = crate::specials::fixture::db();
    let mut f = fixture.features().clone();
    f.weapon_shot_timing = true;
    let mut b = ContentDb::builder().profile(fixture.profile()).features(f);
    for mut w in fixture.weapons().cloned() {
        if w.odf.starts_with("fx_cloak") {
            w.refire = 90;
            w.initial_shot_delay_modifier = Some(Fixed::ONE);
        }
        b = b.weapon(w);
    }
    for u in fixture.units().cloned() {
        b = b.unit(u);
    }
    let db = b.build().unwrap();
    let key = db.key("fx_cloaker").unwrap();
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::new(db),
        players: vec![seat(1)],
        options: GameOptions::default(),
    });
    let c = sim.spawn_def(key, PlayerId(1), v(0, 0), AHEAD).unwrap();
    sim.step(&[cmd(
        PlayerId(1),
        CommandKind::Order(Order::Special {
            units: vec![c],
            slot: 0,
            target: Target::Point(v(0, 0)),
        }),
    )]);
    assert!(sim.specials.cloak_state(c).is_none(), "refused on the spawn tick");
}

#[test]
fn pd_changes_raise_events_and_the_slot_helper_finds_the_weapon() {
    let (mut sim, _, st) = pd_scene(pd_db(false, false), 0);
    let db = Arc::clone(sim.content().unwrap());
    let def = db.unit(sim.unit(st).unwrap().def).unwrap();
    assert_eq!(crate::specials::point_defence_slot(&db, def), Some(0));
    let changes = |sim: &mut Sim, cmds: &[Command]| -> Vec<bool> {
        sim.step(cmds)
            .events
            .iter()
            .filter_map(|e| match e {
                SimEvent::PointDefenceChanged { id, on } if *id == st => Some(*on),
                _ => None,
            })
            .collect()
    };
    sim.step(&[]);
    assert_eq!(changes(&mut sim, &[toggle_pd(st)]), [true]);
    assert_eq!(changes(&mut sim, &[toggle_pd(st)]), [false]);
    // Switched on again, it announces its own auto-off at 0 energy.
    assert_eq!(changes(&mut sim, &[toggle_pd(st)]), [true]);
    let mut off = vec![];
    for _ in 0..700 {
        off.extend(changes(&mut sim, &[]));
    }
    assert_eq!(off, [false]);
    // Without pd_energy the PD weapon is no special and has no slot.
    let plain = sta_content::fake::db();
    let def = plain.unit(plain.key("fx_station").unwrap()).unwrap();
    assert_eq!(crate::specials::point_defence_slot(&plain, def), None);
}
