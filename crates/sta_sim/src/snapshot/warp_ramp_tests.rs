//! #1102 part 2: the warp ramp record of snapshot format 56, and the formats before it.

use super::*;
use super::chain;
use crate::WorldSource;
use crate::command::{Command, CommandKind, Order};
use crate::movement::approach::{ENTERING, IN_WARP, Ramp, WARP_OUT};
use crate::movement::approach_tests;

/// The record's place: the last but one of the chain (the warp option's byte follows it).
const RAMPS: usize = chain::LEN - 2;

fn v(z: i32) -> Vec3 {
    Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: Fixed::from_int(z) }
}

/// A craft on a warp move under `warp_shape`, stepped until its phase is `want`.
fn warping(want: u8) -> (Sim, EntityId) {
    let mut sim = approach_tests::sim(approach_tests::db(sta_content::Profile::Sta2Classic, Some(1100)));
    let db = Arc::clone(sim.content.as_ref().unwrap());
    let s = sim.spawn_def(db.key("st_seer").unwrap(), PlayerId(1), v(0), (Fixed::ZERO, Fixed::ONE)).unwrap();
    sim.step(&[Command { player: PlayerId(1), seq: 0, kind: CommandKind::Order(Order::MoveWarp { units: vec![s], to: v(7000), queue: false }) }]);
    for _ in 0..60 * sta_core::TICK_HZ {
        if sim.approach_warp.phase(s) == Some(want) {
            return (sim, s);
        }
        sim.step(&[]);
    }
    panic!("phase {want} not reached");
}

/// The same snapshot as the format before the record wrote it.
fn as_previous_format(bytes: &[u8]) -> Vec<u8> {
    // (that format has neither the record nor the warp option after it)
    let mut old = chain::before(bytes, RAMPS);
    old[4..6].copy_from_slice(&PRE_WARP_RAMP_FORMAT.to_le_bytes());
    old
}

#[test]
fn the_ramps_are_saved_and_a_malformed_record_is_refused_with_its_reason() {
    let (sim, s) = warping(ENTERING);
    let ramp = sim.approach_warp.ramp(s).expect("an entering craft has its ramp");
    let bytes = sim.snapshot();
    let content = sim.content.clone();
    let back = Sim::from_snapshot(&bytes, content.clone()).unwrap();
    assert_eq!((back.approach_warp.phase(s), back.approach_warp.ramp(s)), (Some(ENTERING), Some(ramp)));
    assert_eq!(back.snapshot(), bytes, "save, load, save");
    assert_eq!(back.digest(), sim.digest());

    let rec = |version: u8, rows: Vec<(EntityId, Ramp)>| chain::with(&bytes, RAMPS, chain::enc(&WarpRampRec { version, rows }));
    let why = |version: u8, rows: Vec<(EntityId, Ramp)>| chain::corrupt(&rec(version, rows), content.clone());
    let bad = "warp ramp rows out of order, without a length or of a craft with no warp phase";
    assert_eq!(why(2, vec![(s, ramp)]), "unknown warp ramp record version");
    assert_eq!(why(1, vec![(s, Ramp { ticks: 0, ..ramp })]), bad);
    assert_eq!(why(1, vec![(EntityId(999), ramp)]), bad);
    assert_eq!(why(1, vec![(s, ramp), (s, ramp)]), bad);
    // a ramp the rules cannot have made, one refusal each: longer than the longest ramp (so
    // an entry that would never end, or a length that does not fit a signed number), a shift
    // below zero, a shift beyond what the craft's hull allows, a start after this snapshot
    let unreal = "warp ramp longer than a ramp can be, with a shift out of range or from the future";
    let longest = crate::movement::approach::longest_ramp();
    assert!(ramp.ticks <= longest && Sim::from_snapshot(&rec(1, vec![(s, Ramp { ticks: longest, ..ramp })]), content.clone()).is_ok(), "the longest ramp loads");
    assert_eq!(why(1, vec![(s, Ramp { ticks: longest + 1, ..ramp })]), unreal);
    assert_eq!(why(1, vec![(s, Ramp { ticks: u32::MAX, ..ramp })]), unreal);
    assert_eq!(why(1, vec![(s, Ramp { shift: Fixed::from_bits(-1), ..ramp })]), unreal);
    let hull = sim.content.as_ref().unwrap().unit(sim.unit(s).unwrap().def).unwrap().radius;
    let most = (crate::movement::approach::stretch(Fixed::from_int(30_000), Fixed::ONE) - Fixed::ONE) * hull;
    assert!(ramp.shift < most && Sim::from_snapshot(&rec(1, vec![(s, Ramp { shift: most, ..ramp })]), content.clone()).is_ok(), "the largest shift of this hull loads");
    assert_eq!(why(1, vec![(s, Ramp { shift: most + Fixed::from_bits(1), ..ramp })]), unreal);
    assert_eq!(why(1, vec![(s, Ramp { shift: Fixed::from_int(1_000_000), ..ramp })]), unreal);
    assert!(Sim::from_snapshot(&rec(1, vec![(s, Ramp { start: sim.tick, ..ramp })]), content.clone()).is_ok(), "a start on the snapshot's tick loads");
    assert_eq!(why(1, vec![(s, Ramp { start: sim.tick + 1, ..ramp })]), unreal);
    assert_eq!(why(1, vec![(s, Ramp { start: u64::MAX, ..ramp })]), unreal);
    // a craft may be without its ramp (it had not been stepped yet): that loads
    assert!(Sim::from_snapshot(&rec(1, Vec::new()), content.clone()).is_ok());
    // the record is mandatory in its format, and nothing may follow the snapshot
    assert!(Sim::from_snapshot(&chain::before(&bytes, RAMPS), content.clone()).is_err());
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(chain::corrupt(&trailing, content).contains("trailing"));
}

/// A snapshot of the format before the record has phases only: a craft at warp loads at warp
/// and flies on at warp speed, one leaving loads as leaving with no shift left, and both are
/// written back in the current format.
#[test]
fn the_format_before_the_record_loads_without_ramps() {
    let (sim, s) = warping(IN_WARP);
    let content = sim.content.clone();
    // what that format could hold: the phase, no ramp
    let mut stored = sim.clone();
    stored.approach_warp = crate::movement::approach::Warp::from_rec(vec![(s, IN_WARP)], Vec::new()).unwrap();
    let old = as_previous_format(&stored.snapshot());
    let mut back = Sim::from_snapshot(&old, content.clone()).expect("the previous format loads");
    assert_eq!((back.approach_warp.phase(s), back.approach_warp.ramp(s)), (Some(IN_WARP), None));
    assert_eq!(back.snapshot(), stored.snapshot(), "written back in the current format");
    let warp = back.unit(s).unwrap().stats.warp_speed;
    for tick in 0..sta_core::TICK_HZ {
        back.step(&[]);
        assert_eq!((back.approach_warp.phase(s), back.unit(s).unwrap().speed), (Some(IN_WARP), warp), "tick {tick}: it flies on at warp");
    }

    // a craft that was slowing down out of warp in that format: it finishes leaving by the shape
    let mut stored = sim.clone();
    stored.approach_warp = crate::movement::approach::Warp::from_rec(vec![(s, WARP_OUT)], Vec::new()).unwrap();
    let mut back = Sim::from_snapshot(&as_previous_format(&stored.snapshot()), content.clone()).unwrap();
    assert_eq!((back.approach_warp.phase(s), back.approach_warp.ramp(s)), (Some(WARP_OUT), None));
    back.step(&[]);
    assert!(back.unit(s).unwrap().speed <= back.unit(s).unwrap().stats.max_speed, "at impulse at once");

    // with a ramp stored, the bytes are not a snapshot of the previous format
    let (entering, _) = warping(ENTERING);
    let mut wrong = entering.snapshot();
    wrong[4..6].copy_from_slice(&PRE_WARP_RAMP_FORMAT.to_le_bytes());
    assert!(chain::corrupt(&wrong, content).contains("trailing"));
}
