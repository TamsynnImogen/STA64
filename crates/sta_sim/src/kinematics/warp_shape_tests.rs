//! `warp_shape` (#1102 part 2): how a craft enters, flies and leaves warp. On the approach
//! scene's ship (`approach_tests::db`) and one authored craft with a large hull and a wide gap
//! between its speeds, whose ramp is long enough to look inside; nothing here is game data.

use std::sync::Arc;

use sta_content::{ContentDb, Profile, Region, RegionKind, RegionShape};
use sta_core::{EntityId, Fixed, PlayerId, TICK_HZ, Vec3};

use super::DRIFT_DECAY;
use crate::command::{Command, CommandKind, Edit, MissionSwitch, Order};
use crate::movement::approach::{self, ENTERING, IN_WARP, Ramp, WARP_OUT, ramp_ticks, stretch, warp_lead};
use crate::movement::approach_tests;
use crate::view::{UnitFlags, WarpPhase};
use crate::{Sim, Unit, WorldSource};

const HZ: u32 = TICK_HZ;
const AHEAD: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);
/// The small ship of the approach scene and the authored large one.
const SMALL: &str = "st_seer";
const LARGE: &str = "st_long";

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}
fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: n(x), y: Fixed::ZERO, z: n(z) }
}
fn hz() -> Fixed {
    n(HZ as i32)
}
/// Rounding of the fixed-point steps: far below anything a test tells apart.
fn eps() -> Fixed {
    Fixed::from_bits(1 << 18)
}
fn near(a: Fixed, b: Fixed) -> bool {
    (a - b).abs() <= eps()
}

/// The approach scene's content for `profile` plus [`LARGE`]: the same ship with a big hull, a
/// slow impulse and a fast warp.
fn db(profile: Profile) -> Arc<ContentDb> {
    let base = approach_tests::db(profile, Some(1100));
    let mut b = ContentDb::builder().profile(base.profile()).features(base.features().clone());
    for w in base.weapons() {
        b = b.weapon(w.clone());
    }
    for u in base.units() {
        b = b.unit(u.clone());
    }
    let mut large = base.units().find(|u| u.odf == SMALL).unwrap().clone();
    large.odf = LARGE.into();
    large.radius = n(90);
    large.movement.max_speed = n(60);
    large.movement.combat_speed = n(30);
    large.movement.warp_speed = n(900);
    Arc::new(b.unit(large).build().unwrap().with_rules(base.rules().clone()))
}

fn game() -> (Sim, Arc<ContentDb>) {
    let db = db(Profile::Sta2Classic);
    assert!(db.features().warp_shape);
    (approach_tests::sim_with(Arc::clone(&db)), db)
}

fn spawn(sim: &mut Sim, db: &ContentDb, odf: &str, player: u8, at: Vec3) -> EntityId {
    sim.spawn_def(db.key(odf).unwrap(), PlayerId(player), at, AHEAD).unwrap()
}
fn warp_move(id: EntityId, to: Vec3) -> Command {
    Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::MoveWarp { units: vec![id], to, queue: false }) }
}
fn script(e: Edit) -> Command {
    Command { player: PlayerId::SCRIPT, seq: 0, kind: CommandKind::Edit(e) }
}
fn phase(sim: &Sim, id: EntityId) -> Option<u8> {
    sim.approach_warp.phase(id)
}
fn unit(sim: &Sim, id: EntityId) -> Unit {
    sim.unit(id).unwrap().clone()
}
fn radius(db: &ContentDb, odf: &str) -> Fixed {
    db.unit(db.key(odf).unwrap()).unwrap().radius
}

/// Steps until the craft's phase is `want` (at most `ticks`); the craft as it was before the
/// step that got there.
fn until(sim: &mut Sim, id: EntityId, want: Option<u8>, ticks: u32) -> Unit {
    for _ in 0..ticks {
        let before = unit(sim, id);
        sim.step(&[]);
        if phase(sim, id) == want {
            return before;
        }
    }
    panic!("phase {want:?} not reached in {ticks} ticks (it is {:?})", phase(sim, id));
}

/// A craft of `odf` on a warp move straight ahead, stepped until it starts to enter warp: the
/// sim, the craft, and the craft as it was before its ramp's first tick.
fn entering(odf: &str) -> (Sim, Arc<ContentDb>, EntityId, Unit) {
    let (mut sim, db) = game();
    let s = spawn(&mut sim, &db, odf, 1, v(0, 0));
    sim.step(&[warp_move(s, v(0, 7500))]);
    let before = until(&mut sim, s, Some(ENTERING), 30 * HZ);
    (sim, db, s, before)
}

#[test]
fn only_stock_armada_ii_has_the_switch_on() {
    let on: Vec<Profile> = [Profile::Native, Profile::Fo4, Profile::Fo3, Profile::Sta2Classic, Profile::Sta1Classic].into_iter().filter(|p| p.features().warp_shape).collect();
    assert_eq!(on, [Profile::Sta2Classic]);
}

#[test]
fn the_shift_of_a_ramp_comes_over_one_half_and_adds_up_to_the_whole() {
    for ticks in [1, 2, 3, 7, 33, 90] {
        let ramp = Ramp { start: 100, ticks, shift: n(40) };
        for entering in [true, false] {
            let per_tick: Vec<Fixed> = (0..u64::from(ticks)).map(|k| ramp.shift_at(100 + k, entering)).collect();
            let total = per_tick.iter().fold(Fixed::ZERO, |a, b| a + *b);
            assert!(near(total, ramp.shift), "{ticks} ticks, entering {entering}: {total:?}");
            for (k, s) in per_tick.iter().enumerate() {
                let k = k as u32;
                // a tick wholly in the half without shift gets none
                let idle = if entering { 2 * (k + 1) <= ticks } else { 2 * k >= ticks };
                assert!(*s >= Fixed::ZERO && (!idle || *s == Fixed::ZERO), "{ticks} ticks, entering {entering}, tick {k}: {s:?}");
            }
            assert_eq!(ramp.shift_at(100 + u64::from(ticks), entering), Fixed::ZERO, "nothing after the ramp");
            assert!(ramp.ends_at(99 + u64::from(ticks)) && (ticks == 1 || !ramp.ends_at(98 + u64::from(ticks))));
        }
    }
}

#[test]
fn the_stretch_factor_and_the_ramp_length_stay_within_their_bounds() {
    let db = db(Profile::Sta2Classic);
    let stats = |odf: &str| db.unit(db.key(odf).unwrap()).unwrap().movement;
    let (small, large) = (stats(SMALL), stats(LARGE));
    // the large craft's speeds put its factor at the upper bound: more warp speed adds nothing
    let top = stretch(large.warp_speed, large.max_speed);
    assert_eq!(stretch(large.warp_speed * n(2), large.max_speed), top);
    // the small one's lie between the bounds, and nothing goes below the lower one
    let mid = stretch(small.warp_speed, small.max_speed);
    let low = stretch(small.max_speed, small.max_speed);
    assert!(low < mid && mid < top && mid < stretch(small.warp_speed * n(2), small.max_speed));
    assert_eq!(stretch(small.max_speed + n(1), small.max_speed), low, "warp barely above impulse: the lower bound");
    assert_eq!(stretch(small.max_speed / n(2), small.max_speed), low, "warp below impulse: the lower bound");

    // the small craft's entry ramp is at the lower bound of the length: a smaller hull changes nothing
    let (r, dv) = (radius(&db, SMALL), small.warp_speed - small.max_speed);
    let shortest = ramp_ticks(r, mid, dv);
    assert_eq!(ramp_ticks(r / n(2), mid, dv), shortest);
    assert_eq!(ramp_ticks(Fixed::ZERO, mid, dv), shortest);
    // the large one's is between the bounds
    let (r, dv) = (radius(&db, LARGE), large.warp_speed - large.max_speed);
    let long = ramp_ticks(r, top, dv);
    assert!(long > shortest && ramp_ticks(r / n(2), top, dv) < long && ramp_ticks(r * n(2), top, dv) > long);
    // and a huge hull or a tiny speed change stops at the upper bound
    let longest = ramp_ticks(r * n(1000), top, dv);
    assert_eq!(ramp_ticks(r * n(2000), top, dv), longest);
    assert_eq!(ramp_ticks(r, top, Fixed::from_bits(1)), longest);
    assert_eq!(ramp_ticks(r, top, Fixed::ZERO), longest);
    assert!(longest > long);

    // in the sim: the small craft's ramp is the shortest there is
    let (sim, _, s, _) = entering(SMALL);
    assert_eq!(sim.approach_warp.ramp(s).unwrap().ticks, shortest);
    // and the large one's is the rule's length for the speed it held
    let (sim, _, s, before) = entering(LARGE);
    assert_eq!(sim.approach_warp.ramp(s).unwrap().ticks, ramp_ticks(r, top, large.warp_speed - before.speed));
}

#[test]
fn entering_holds_the_speed_then_shifts_forward_then_is_at_warp_speed() {
    let (mut sim, db, s, before) = entering(LARGE);
    let (vi, vw) = (before.stats.max_speed, before.stats.warp_speed);
    let ramp = sim.approach_warp.ramp(s).unwrap();
    let held = before.speed;
    assert!(held <= vi && held > vi * n(9) / n(10), "it starts from about impulse: {held:?}");
    assert_eq!(ramp.shift, (stretch(vw, vi) - Fixed::ONE) * radius(&db, LARGE));
    assert_eq!(ramp.ticks, ramp_ticks(radius(&db, LARGE), stretch(vw, vi), vw - held));
    assert!(ramp.ticks >= 20, "a ramp long enough to look inside: {}", ramp.ticks);

    // the step that began the ramp is its first tick
    let (mut last, mut extra_total) = (before.clone(), Fixed::ZERO);
    for k in 1..=ramp.ticks {
        let u = unit(&sim, s);
        assert_eq!(u.pos.x, before.pos.x, "tick {k}: straight on");
        let extra = u.pos.z - last.pos.z - held / hz();
        if 2 * k <= ramp.ticks {
            assert!(near(extra, Fixed::ZERO), "tick {k}: the first half is travel at the held speed only ({extra:?})");
        } else {
            assert!(extra > -eps(), "tick {k}");
            if 2 * (k - 1) >= ramp.ticks {
                assert!(near(extra, ramp.shift * n(2) / n(ramp.ticks as i32)), "tick {k}: an even share of the shift ({extra:?})");
            }
        }
        extra_total = extra_total + extra;
        if k < ramp.ticks {
            assert_eq!((phase(&sim, s), u.speed), (Some(ENTERING), held), "tick {k}: no speed ramp");
            last = u;
            sim.step(&[]);
        } else {
            assert_eq!((phase(&sim, s), u.speed), (Some(IN_WARP), vw), "with the last tick it is at warp speed");
            last = u;
        }
    }
    assert!((extra_total - ramp.shift).abs() < n(1) / n(100), "the whole shift: {extra_total:?} of {:?}", ramp.shift);
    // the way over the ramp is the held speed for its time plus the shift, well within a tick of travel
    let way = last.pos.z - before.pos.z;
    assert!((way - (held * n(ramp.ticks as i32) / hz() + ramp.shift)).abs() < held / hz());
    // from here on: warp speed, a tick's worth of it each tick
    for tick in 0..HZ {
        sim.step(&[]);
        let u = unit(&sim, s);
        assert_eq!((phase(&sim, s), u.speed), (Some(IN_WARP), vw), "tick {tick}");
        assert!(near(u.pos.z - last.pos.z, vw / hz()), "tick {tick}");
        last = u;
    }
}

#[test]
fn leaving_drops_to_impulse_at_once_and_shifts_forward_over_the_first_half() {
    let (mut sim, db, s, _) = entering(LARGE);
    until(&mut sim, s, Some(IN_WARP), 10 * HZ);
    let before = until(&mut sim, s, Some(WARP_OUT), 60 * HZ);
    let (vi, vw) = (before.stats.max_speed, before.stats.warp_speed);
    assert_eq!(before.speed, vw);
    let goal = before.target.expect("it is still on its way");
    let left = goal.z - before.pos.z;
    assert!(left < warp_lead(vw, vi, radius(&db, LARGE)) && left > warp_lead(vw, vi, radius(&db, LARGE)) - vw / hz() * n(2), "it leaves at the lead: {left:?}");

    let ramp = sim.approach_warp.ramp(s).expect("the exit has a ramp");
    assert_eq!(ramp.start + 1, sim.tick, "made on the tick it left");
    assert_eq!(ramp.shift, (stretch(vw, vi) - Fixed::ONE) * radius(&db, LARGE));
    assert_eq!(ramp.ticks, ramp_ticks(radius(&db, LARGE), stretch(vw, vi), vw - vi));
    let mut last = before;
    for k in 1..=ramp.ticks {
        let u = unit(&sim, s);
        assert!(u.speed <= vi, "tick {k}: no speed ramp down, impulse at once ({:?})", u.speed);
        let moved = u.pos.z - last.pos.z;
        let share = ramp.shift * n(2) / n(ramp.ticks as i32);
        if 2 * k <= ramp.ticks {
            // its own travel (at most impulse) and an even share of the shift
            assert!(moved > share - eps() && moved < share + vi / hz() + eps(), "tick {k}: {moved:?}");
        } else if 2 * (k - 1) >= ramp.ticks {
            assert!(moved < vi / hz() + eps(), "tick {k}: the second half is its own travel only ({moved:?})");
        }
        last = u;
        if k < ramp.ticks {
            assert_eq!(phase(&sim, s), Some(WARP_OUT), "tick {k}");
            assert_eq!(sim.approach_warp.ramp(s), Some(ramp), "the ramp is fixed when it starts");
            sim.step(&[]);
        }
    }
    assert_eq!(phase(&sim, s), None, "with the ramp's last tick the warp is over");
    // and the craft goes on to its goal and stops there
    for _ in 0..60 * HZ {
        sim.step(&[]);
        assert_eq!(phase(&sim, s), None, "so near the goal it does not warp again");
    }
    let u = unit(&sim, s);
    assert!(u.target.is_none() && u.speed == Fixed::ZERO && (u.pos.z - goal.z).abs() < n(20), "{:?}", u.pos);
}

#[test]
fn a_new_warp_order_off_the_line_is_followed_at_warp() {
    let (mut sim, db, s, _) = entering(SMALL);
    until(&mut sim, s, Some(IN_WARP), 5 * HZ);
    let start = unit(&sim, s);
    let vw = start.stats.warp_speed;
    // straight to the side of where it is flying
    let to = Vec3 { x: n(4000), y: Fixed::ZERO, z: start.pos.z + n(600) };
    assert!(!approach::warp_leg(&start, to).1, "the new goal is well off its line");
    sim.step(&[warp_move(s, to)]);
    let lead = warp_lead(vw, start.stats.max_speed, radius(&db, SMALL));
    let mut at_warp = 0;
    loop {
        let u = unit(&sim, s);
        if phase(&sim, s) != Some(IN_WARP) {
            let left = ((to.x - u.pos.x) * (to.x - u.pos.x) + (to.z - u.pos.z) * (to.z - u.pos.z)).sqrt();
            assert_eq!(phase(&sim, s), Some(WARP_OUT));
            assert!(left < lead + vw / hz(), "it left warp only at the lead before its goal ({left:?})");
            break;
        }
        assert_eq!(u.speed, vw, "tick {at_warp}: warp speed through the turn, no braking");
        at_warp += 1;
        assert!(at_warp < 60 * HZ, "it never arrives");
        sim.step(&[]);
    }
    let u = unit(&sim, s);
    assert!(at_warp > 2 * HZ, "a long way at warp: {at_warp} ticks");
    assert!(u.heading.0 > n(9) / n(10), "it turned onto the new line: {:?}", u.heading);
}

/// A field in the way makes the navigation plan a way round. A craft at warp keeps warp speed
/// and steers; it is not braked for the turn. On a warp move and on an attack approach.
#[test]
fn a_detour_at_warp_is_flown_at_warp() {
    for attack in [false, true] {
        let (mut sim, db) = game();
        let s = spawn(&mut sim, &db, SMALL, 1, v(0, 0));
        let first = if attack {
            let t = spawn(&mut sim, &db, "st_target", 2, v(0, 7000));
            Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::Attack { units: vec![s], target: t }) }
        } else {
            warp_move(s, v(0, 7000))
        };
        sim.step(&[first]);
        until(&mut sim, s, Some(IN_WARP), 10 * HZ);
        let before = unit(&sim, s);
        sim.regions.push(Region {
            kind: RegionKind::AsteroidField,
            odf: "turn_fixture".into(),
            shape: RegionShape::Sphere { centre: Vec3 { z: before.pos.z + n(600), ..before.pos }, radius: n(280) },
        });
        let mut turned = false;
        for tick in 0..HZ / 2 {
            sim.step(&[]);
            let u = unit(&sim, s);
            assert_eq!((phase(&sim, s), u.speed), (Some(IN_WARP), before.stats.warp_speed), "attack {attack}, tick {tick}");
            turned |= u.heading != before.heading;
        }
        assert!(turned, "attack {attack}: it steers round at warp");
    }
}

#[test]
fn engines_going_off_at_warp_give_the_normal_exit_then_the_drift_and_warp_returns_with_them() {
    let (mut sim, db, s, _) = entering(LARGE);
    until(&mut sim, s, Some(IN_WARP), 10 * HZ);
    for _ in 0..HZ / 2 {
        sim.step(&[]);
    }
    let before = unit(&sim, s);
    let (vi, vw) = (before.stats.max_speed, before.stats.warp_speed);
    assert_eq!(before.speed, vw);
    sim.step(&[script(Edit::Engines(s, false))]);
    assert!(unit(&sim, s).flags.contains(UnitFlags::ENGINES_OFF));
    assert_eq!(phase(&sim, s), Some(WARP_OUT), "the next tick it is leaving");
    let ramp = sim.approach_warp.ramp(s).unwrap();
    assert_eq!(ramp, Ramp::leaving(ramp.start, radius(&db, LARGE), vw, vi, vw), "the normal exit's ramp");

    // exactly the drift from impulse speed plus the exit's shift, tick by tick
    let (mut z, mut speed) = (before.pos.z, vi);
    for k in 0..ramp.ticks + HZ {
        let u = unit(&sim, s);
        z = z + speed / hz() + ramp.shift_at(ramp.start + u64::from(k), false);
        speed = speed - speed * DRIFT_DECAY / hz();
        assert!(near(u.pos.z, z) && near(u.speed, speed), "tick {k}: at {:?} with {:?}, expected {z:?} with {speed:?}", u.pos.z, u.speed);
        assert_eq!(u.heading, before.heading, "tick {k}");
        assert_eq!(phase(&sim, s), (k + 1 < ramp.ticks).then_some(WARP_OUT), "tick {k}: the row lasts as long as the ramp");
        sim.step(&[]);
    }
    // the engines return and the order still asks for warp: it enters again by itself
    sim.step(&[script(Edit::Engines(s, true))]);
    until(&mut sim, s, Some(ENTERING), 20 * HZ);
    until(&mut sim, s, Some(IN_WARP), 5 * HZ);
    assert_eq!(unit(&sim, s).speed, vw);
}

/// The permission for warp falls away while a craft is on its entry ramp: it leaves the same
/// tick, on the ramp it has, and never reaches warp speed.
#[test]
fn an_entry_cut_short_becomes_a_leave_on_the_same_ramp() {
    let (mut sim, _, s, before) = entering(LARGE);
    let ramp = sim.approach_warp.ramp(s).unwrap();
    for _ in 0..4 {
        sim.step(&[]);
    }
    assert_eq!(phase(&sim, s), Some(ENTERING));
    let at_cut = unit(&sim, s);
    sim.step(&[script(Edit::MissionSwitch(MissionSwitch::AllowWarp, false))]);
    assert_eq!(phase(&sim, s), Some(WARP_OUT), "a leave on the tick the gate fails");
    assert_eq!(sim.approach_warp.ramp(s), Some(ramp), "the ramp keeps its start and its length");
    // still in the ramp's first half: as a leave, that half carries the shift
    let u = unit(&sim, s);
    assert!(u.pos.z - at_cut.pos.z > ramp.shift * n(2) / n(ramp.ticks as i32) - eps());
    let vi = before.stats.max_speed;
    while phase(&sim, s).is_some() {
        assert!(unit(&sim, s).speed <= vi, "never above impulse");
        assert!(sim.tick <= ramp.start + u64::from(ramp.ticks), "the row is gone with the ramp's last tick");
        sim.step(&[]);
    }
    for _ in 0..2 * HZ {
        sim.step(&[]);
        assert!(phase(&sim, s).is_none() && unit(&sim, s).speed <= vi, "warp stays off while the switch is off");
    }
}

#[test]
fn a_snapshot_inside_either_ramp_reloads_and_runs_on_identically() {
    let runs_on = |sim: &Sim, what: &str| {
        let mut back = Sim::from_snapshot(&sim.snapshot(), sim.content.clone()).unwrap();
        assert_eq!(back.digest(), sim.digest(), "{what}");
        assert_eq!(back.snapshot(), sim.snapshot(), "{what}: save, load, save");
        let mut sim = sim.clone();
        for tick in 0..3 * HZ {
            sim.step(&[]);
            back.step(&[]);
            assert_eq!(back.digest(), sim.digest(), "{what}, tick {tick}");
        }
    };
    let (mut sim, _, s, _) = entering(LARGE);
    for _ in 0..6 {
        sim.step(&[]);
    }
    assert_eq!(phase(&sim, s), Some(ENTERING));
    runs_on(&sim, "inside the entry");
    // the ramp is part of the state: another start is another digest
    let mut other = sim.clone();
    let ramp = other.approach_warp.ramp(s).unwrap();
    other.approach_warp.set_ramp(s, Ramp { start: ramp.start - 1, ..ramp });
    assert_ne!(other.digest(), sim.digest());

    until(&mut sim, s, Some(IN_WARP), 5 * HZ);
    runs_on(&sim, "at warp");
    until(&mut sim, s, Some(WARP_OUT), 60 * HZ);
    for _ in 0..6 {
        sim.step(&[]);
    }
    assert_eq!(phase(&sim, s), Some(WARP_OUT));
    runs_on(&sim, "inside the exit");
}

#[test]
fn the_view_rows_are_the_stored_ramps() {
    let (mut sim, db) = game();
    let s = spawn(&mut sim, &db, LARGE, 1, v(0, 0));
    let first = warp_move(s, v(0, 7500));
    let mut seen: Vec<WarpPhase> = Vec::new();
    for i in 0..90 * HZ {
        let view = WorldSource::step(&mut sim, if i == 0 { std::slice::from_ref(&first) } else { &[] }).view;
        let Some(row) = view.warp_of(s).copied() else {
            assert_eq!(phase(&sim, s), None);
            continue;
        };
        let ramp = sim.approach_warp.ramp(s).expect("a craft with a warp row has its ramp once it was stepped");
        assert_eq!((row.ramp_start, row.ramp_len), (ramp.start, ramp.ticks));
        let want = match phase(&sim, s) {
            Some(ENTERING) => WarpPhase::Entering,
            Some(IN_WARP) => WarpPhase::AtWarp,
            _ => WarpPhase::Leaving,
        };
        assert_eq!(row.phase, want);
        // how far into its ramp the craft is, as the client reads it
        let into = view.tick - row.ramp_start;
        match row.phase {
            WarpPhase::Entering | WarpPhase::Leaving => assert!(into >= 1 && into < u64::from(row.ramp_len), "{into} of {}", row.ramp_len),
            WarpPhase::AtWarp => assert!(into >= u64::from(row.ramp_len), "the entry ramp is over"),
        }
        if seen.last() != Some(&row.phase) {
            seen.push(row.phase);
        }
    }
    assert_eq!(seen, [WarpPhase::Entering, WarpPhase::AtWarp, WarpPhase::Leaving]);
    assert!(sim.view().warp_rows.is_empty());
}

/// Content without the switch stores nothing of it: no entering phase, no ramp, and so the
/// earlier form of the warp record in the digest.
#[test]
fn without_the_switch_no_ramp_is_stored() {
    for profile in [Profile::Fo4, Profile::Native] {
        let db = db(profile);
        assert!(!db.features().warp_shape);
        let mut sim = approach_tests::sim_with(Arc::clone(&db));
        let s = spawn(&mut sim, &db, SMALL, 1, v(0, 0));
        sim.step(&[warp_move(s, v(0, 7500))]);
        let mut warped = false;
        for _ in 0..30 * HZ {
            sim.step(&[]);
            warped |= phase(&sim, s) == Some(IN_WARP);
            assert_ne!(phase(&sim, s), Some(ENTERING), "{profile:?}");
            assert!(sim.approach_warp.ramps_rec().is_empty(), "{profile:?}");
        }
        assert!(warped, "{profile:?}: the scene warps");
    }
}
