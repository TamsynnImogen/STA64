//! LUA-31: the scripted Armada I warp speed (`Edit::WarpSpeed`), on an Armada I
//! scout (impulse 20, warp 100, `tooCloseForWarp` 10, `headingCloseEnoughForWarp` 30;
//! the SIM-64 stand-in of `orders/tests.rs`).

use std::sync::Arc;

use sta_content::{ContentDb, MoveStats};

use crate::command::{Command, CommandKind, Edit, Order};
use crate::movement::approach_tests;
use crate::present::Presentation;
use crate::{EntityId, Fixed, PlayerId, Sim, SimEvent, Vec3, WorldSource};

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}
fn at(x: i32, z: i32) -> Vec3 {
    Vec3 { x: n(x), y: Fixed::ZERO, z: n(z) }
}
const AHEAD: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);
const HZ: u32 = sta_core::TICK_HZ;

fn scout_db(keys: bool) -> Arc<ContentDb> {
    let base = sta_content::fake::db();
    let mut b = ContentDb::builder().profile(base.profile()).features(base.features().clone());
    for w in base.weapons().cloned() {
        b = b.weapon(w);
    }
    for mut u in base.units().cloned() {
        if u.odf == "fx_frigate" {
            u.movement = MoveStats::from_f32(20.0, 1.0, 1.0, 1.0, 1.0);
            u.movement.warp_speed = n(100);
            if keys {
                u.movement.too_close_for_warp = n(10);
                u.movement.heading_close_for_warp = n(30);
            }
        }
        b = b.unit(u);
    }
    Arc::new(b.build().unwrap())
}

pub(crate) fn game() -> (Sim, EntityId) {
    let db = scout_db(true);
    let mut sim = approach_tests::sim_with(Arc::clone(&db));
    let s = sim.spawn_def(db.key("fx_frigate").unwrap(), PlayerId(1), at(0, 0), AHEAD).unwrap();
    (sim, s)
}

pub(crate) fn script(sim: &mut Sim, e: Edit) -> bool {
    let c = Command { player: PlayerId::SCRIPT, seq: 0, kind: CommandKind::Edit(e) };
    !sim.step(&[c]).events.iter().any(|e| matches!(e, SimEvent::Rejected { .. }))
}

fn order(sim: &mut Sim, o: Order) {
    sim.step(&[Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(o) }]);
}

fn warp(sim: &Sim, id: EntityId) -> Fixed {
    sim.unit(id).unwrap().stats.warp_speed
}

fn speed(sim: &Sim, id: EntityId) -> Fixed {
    sim.unit(id).unwrap().speed
}

/// Top speed while the craft's bow is off the goal (turning legs) and while it is
/// on it (the aligned leg, after a settling second), over `secs` seconds.
fn legs(sim: &mut Sim, id: EntityId, goal: Vec3, secs: u32) -> (Fixed, Fixed) {
    let (mut turning, mut aligned) = (Fixed::ZERO, Fixed::ZERO);
    let mut settled = 0;
    for _ in 0..secs * HZ {
        sim.step(&[]);
        let u = sim.unit(id).unwrap();
        let (gx, gz) = (goal.x - u.pos.x, goal.z - u.pos.z);
        let d = (gx * gx + gz * gz).sqrt();
        if d < n(200) {
            break;
        }
        let cos = (u.heading.0 * gx + u.heading.1 * gz) / d;
        if cos > Fixed::from_f64(0.99) {
            settled += 1;
            if settled > 2 * HZ {
                aligned = aligned.max(u.speed);
            }
        } else if cos < Fixed::from_f64(0.9) {
            // Clearly off the bow (the engine's alignment gate is 0.99).
            settled = 0;
            turning = turning.max(u.speed);
        }
    }
    (turning, aligned)
}

#[test]
fn warp_30_holds_30_on_the_aligned_leg_and_impulse_while_turning() {
    // The goal is behind the bow: the craft turns at impulse, then flies at 30 (above impulse 20: a real warp).
    let (mut sim, s) = game();
    assert!(script(&mut sim, Edit::WarpSpeed(s, Some(n(30)))));
    assert_eq!(warp(&sim, s), n(30));
    let goal = at(0, -6000);
    order(&mut sim, Order::Move { units: vec![s], to: goal, queue: false });
    let (turning, aligned) = legs(&mut sim, s, goal, 60);
    assert!(turning <= n(21), "turning leg at {turning:?}");
    assert!(aligned >= n(29) && aligned <= n(31), "aligned leg at {aligned:?}");
}

#[test]
fn reset_restores_the_class_warp_and_the_override_persists_across_orders() {
    let (mut sim, s) = game();
    assert!(script(&mut sim, Edit::WarpSpeed(s, Some(n(30)))));
    order(&mut sim, Order::Move { units: vec![s], to: at(0, 3000), queue: false });
    for _ in 0..5 * HZ {
        sim.step(&[]);
    }
    order(&mut sim, Order::Stop { units: vec![s] });
    order(&mut sim, Order::Move { units: vec![s], to: at(0, 9000), queue: false });
    assert_eq!(warp(&sim, s), n(30));
    assert!(sim.view().attrs.warp == vec![(s, n(30))]);
    assert!(script(&mut sim, Edit::WarpSpeed(s, None)));
    assert_eq!(warp(&sim, s), n(100));
    assert!(sim.view().attrs.warp.is_empty());
    let mut top = Fixed::ZERO;
    for _ in 0..20 * HZ {
        sim.step(&[]);
        top = top.max(speed(&sim, s));
    }
    assert!(top > n(90), "after reset the warp leg is back: {top:?}");
}

#[test]
fn impulse_85_with_warp_50_flies_50_aligned_and_85_turning() {
    let (mut sim, s) = game();
    assert!(script(&mut sim, Edit::WarpSpeed(s, Some(n(50)))));
    assert!(script(&mut sim, Edit::MaxSpeed(s, Some(n(85)))));
    // Setting impulse keeps the absolute warp speed.
    assert_eq!(warp(&sim, s), n(50));
    let goal = at(0, -9000);
    order(&mut sim, Order::Move { units: vec![s], to: goal, queue: false });
    let (turning, aligned) = legs(&mut sim, s, goal, 120);
    assert!(turning > n(60) && turning <= n(86), "turning leg at {turning:?}");
    assert!(aligned >= n(49) && aligned <= n(51), "aligned leg at {aligned:?}");
}

#[test]
fn a_warp_speed_below_impulse_slows_the_far_leg_without_a_warp_effect() {
    let (mut sim, s) = game();
    assert!(script(&mut sim, Edit::WarpSpeed(s, Some(n(10)))));
    order(&mut sim, Order::Move { units: vec![s], to: at(0, 6000), queue: false });
    let mut top = Fixed::ZERO;
    for _ in 0..30 * HZ {
        sim.step(&[]);
        top = top.max(speed(&sim, s));
    }
    assert!(top <= n(11) && top > n(8), "{top:?}");
    // No warp phase was ever entered.
    assert!(!sim.approach_warp.contains(s));
}

#[test]
fn warp_zero_or_equal_to_impulse_means_no_warp_leg_and_the_move_arrives() {
    for w in [n(0), n(20)] {
        let (mut sim, s) = game();
        assert!(script(&mut sim, Edit::WarpSpeed(s, Some(w))));
        assert_eq!(warp(&sim, s), w);
        let goal = at(0, 400);
        order(&mut sim, Order::Move { units: vec![s], to: goal, queue: false });
        let mut top = Fixed::ZERO;
        for _ in 0..40 * HZ {
            sim.step(&[]);
            top = top.max(speed(&sim, s));
        }
        assert!(top <= n(21) && top > n(15), "{w:?}: {top:?}");
        let u = sim.unit(s).unwrap();
        assert!(u.target.is_none(), "{w:?}: the move ended");
        assert!((u.pos.z - goal.z).abs() < n(40), "{w:?}: arrived at {:?}", u.pos);
    }
}

#[test]
fn values_are_kept_between_zero_and_the_ceiling() {
    let (mut sim, s) = game();
    assert!(script(&mut sim, Edit::WarpSpeed(s, Some(n(-5)))));
    assert_eq!(warp(&sim, s), Fixed::ZERO);
    assert!(script(&mut sim, Edit::WarpSpeed(s, Some(n(1_000_000)))));
    assert_eq!(warp(&sim, s), crate::warp_override::MAX_WARP);
    order(&mut sim, Order::Move { units: vec![s], to: at(0, 60000), queue: false });
    for _ in 0..60 * HZ {
        sim.step(&[]);
        let u = sim.unit(s).unwrap();
        assert!(u.pos.z >= Fixed::ZERO && u.speed <= crate::warp_override::MAX_WARP + n(1));
    }
}

#[test]
fn the_end_of_a_cineractive_resets_warp_and_impulse_of_every_craft() {
    let (mut sim, s) = game();
    let t = sim.spawn_def(sim.content().unwrap().key("fx_cruiser").unwrap(), PlayerId(0), at(500, 0), AHEAD).unwrap();
    let (class_warp_t, class_imp_s) = (warp(&sim, t), sim.unit(s).unwrap().stats.max_speed);
    for id in [s, t] {
        assert!(script(&mut sim, Edit::WarpSpeed(id, Some(n(33)))));
    }
    assert!(script(&mut sim, Edit::MaxSpeed(s, Some(n(70)))));
    assert!(script(&mut sim, Edit::Present(Presentation::Cineractive { on: true, skippable: false })));
    assert_eq!(warp(&sim, s), n(33));
    assert!(script(&mut sim, Edit::Present(Presentation::Cineractive { on: false, skippable: false })));
    assert_eq!((warp(&sim, s), warp(&sim, t)), (n(100), class_warp_t));
    assert_eq!(sim.unit(s).unwrap().stats.max_speed, class_imp_s);
    assert!(sim.view().attrs.warp.is_empty());
}

#[test]
fn only_live_craft_are_accepted_and_only_the_script() {
    let (mut sim, s) = game();
    let station = sim.spawn_def(sim.content().unwrap().key("fx_station").unwrap(), PlayerId(0), at(900, 0), AHEAD).unwrap();
    let before = warp(&sim, station);
    assert!(!script(&mut sim, Edit::WarpSpeed(station, Some(n(30)))));
    assert!(!script(&mut sim, Edit::WarpSpeed(EntityId(9999), None)));
    assert_eq!(warp(&sim, station), before);
    sim.units.get_mut(s).unwrap().hull[0] = Fixed::ZERO;
    assert!(!script(&mut sim, Edit::WarpSpeed(s, Some(n(30)))));
    sim.units.get_mut(s).unwrap().hull[0] = Fixed::ONE;
    sim.units.get_mut(s).unwrap().flags = crate::view::UnitFlags::DYING;
    assert!(!script(&mut sim, Edit::WarpSpeed(s, None)));
    sim.units.get_mut(s).unwrap().flags = crate::view::UnitFlags::default();
    let c = Command { player: PlayerId(1), seq: 0, kind: CommandKind::Edit(Edit::WarpSpeed(s, Some(n(30)))) };
    assert!(sim.step(&[c]).events.iter().any(|e| matches!(e, SimEvent::Rejected { .. })));
    assert_eq!(warp(&sim, s), n(100));
}

#[test]
fn a_run_that_never_overrides_keeps_its_digests() {
    let (plain, s) = game();
    let (mut same, _) = game();
    // Writing the class value is no override: nothing is listed, saved or hashed.
    assert!(script(&mut same, Edit::WarpSpeed(s, Some(n(100)))));
    let mut plain = plain;
    plain.step(&[]);
    assert!(same.view().attrs.warp.is_empty());
    assert_eq!(same.digest(), plain.digest());
    assert_eq!(same.section_digests(), plain.section_digests());
    // An override moves the full digest and the movement section only.
    let (mut over, _) = game();
    assert!(script(&mut over, Edit::WarpSpeed(s, Some(n(30)))));
    assert_ne!(over.digest(), plain.digest());
    assert_eq!(over.section_digests().differing(&plain.section_digests()), ["movement"]);
}

#[test]
fn a_snapshot_keeps_the_override_and_a_restored_sim_steps_like_the_live_one() {
    let (mut sim, s) = game();
    assert!(script(&mut sim, Edit::WarpSpeed(s, Some(n(30)))));
    order(&mut sim, Order::Move { units: vec![s], to: at(0, -5000), queue: false });
    for _ in 0..10 * HZ {
        sim.step(&[]);
    }
    let mut loaded = Sim::from_snapshot(&sim.snapshot(), sim.content().cloned()).unwrap();
    assert_eq!(loaded.digest(), sim.digest());
    assert_eq!(warp(&loaded, s), n(30));
    for _ in 0..20 * HZ {
        sim.step(&[]);
        loaded.step(&[]);
    }
    assert_eq!(loaded.digest(), sim.digest());
    // A sim without an override saves an empty record and loads the definition's value.
    let (fresh, id) = game();
    let back = Sim::from_snapshot(&fresh.snapshot(), fresh.content().cloned()).unwrap();
    assert_eq!(warp(&back, id), n(100));
}

/// The pinned whole-state digest of a run holding an overridden warp speed; it
/// moves only when the override's effect on the sim or the digest changes.
#[test]
fn the_pinned_digest_of_a_run_with_an_overridden_warp_speed() {
    let (mut sim, s) = game();
    assert!(script(&mut sim, Edit::WarpSpeed(s, Some(n(30)))));
    order(&mut sim, Order::Move { units: vec![s], to: at(0, -4000), queue: false });
    for _ in 0..12 * HZ {
        sim.step(&[]);
    }
    assert_eq!(sim.digest(), PINNED_DIGEST, "{:#x}", sim.digest());
}

const PINNED_DIGEST: u64 = 0x109e_fd91_d4ee_ddbc;
