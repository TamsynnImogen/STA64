//! Install-gated (`STA64_TEST_A1_INSTALL`, an Armada I root; skipped when
//! unset): SIM-66's Armada I movement model on the shipped data. Every craft
//! that moves and has the model's parameters flies a long straight leg and a
//! right-angle order, and the general rules of `kinematics/a1.rs` hold for each:
//! it cruises at its own `warpSpeed`, its speed rises without a jump (the lag),
//! it moves while it turns (no pivot), and it brakes to rest short of the goal
//! without passing it. Key values are the install's own; the rules are read
//! from the executable (static) and measured on the original for the stock
//! scout; no number of the original is written here.

use std::path::Path;
use std::sync::Arc;

use sta_content::{ContentDb, OdfIndex, Profile};
use sta_core::{Fixed, PlayerId, TeamId, Vec3, TICK_HZ};
use sta_sim::command::{Command, CommandKind, Order};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, WorldSource};
use sta_vfs::Vfs;

fn f(v: Fixed) -> f64 {
    v.to_bits() as f64 / 4294967296.0
}

struct Track {
    /// Per tick: (x, z, speed).
    at: Vec<(f64, f64, f64)>,
    /// The unit spawned as a craft that can move.
    craft: bool,
}

fn fly(db: &Arc<ContentDb>, key: sta_core::ContentKey, to: (f64, f64), secs: u32) -> Track {
    let seat = |id: u8| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fed".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 7,
        map: Arc::new(sta_content::MapDef::empty()),
        content: db.clone(),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    });
    let s = sim.spawn_def(key, PlayerId(0), Vec3::ZERO, (Fixed::ZERO, Fixed::ONE)).unwrap();
    let craft = sim.unit(s).is_some_and(|u| u.kind == sta_sim::world::EntityKind::Craft && u.mobile());
    let to = Vec3 { x: Fixed::from_f64(to.0), y: Fixed::ZERO, z: Fixed::from_f64(to.1) };
    let mut at = Vec::new();
    for i in 0..secs * TICK_HZ {
        let cmds = if i == 0 { vec![Command { player: PlayerId(0), seq: 0, kind: CommandKind::Order(Order::Move { units: vec![s], to, queue: false, mode: None }) }] } else { vec![] };
        WorldSource::step(&mut sim, &cmds);
        let u = sim.unit(s).unwrap();
        at.push((f(u.pos.x), f(u.pos.z), f(u.speed)));
    }
    Track { at, craft }
}

#[test]
fn every_moving_craft_follows_the_general_rules_of_the_model() {
    let Ok(root) = std::env::var("STA64_TEST_A1_INSTALL") else {
        eprintln!("SKIP: set STA64_TEST_A1_INSTALL to the stock Armada I folder");
        return;
    };
    let vfs = Vfs::mount_install(Path::new(&root), None).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    assert_eq!(db.profile(), Profile::Sta1Classic, "STA64_TEST_A1_INSTALL must name an Armada I install");
    let db = Arc::new(db);
    let mut checked = 0;
    let mut failures = Vec::new();
    for u in db.units() {
        let m = u.movement;
        let (impulse, warp) = (f(m.max_speed), f(m.warp_speed));
        if impulse <= 0.0 || warp <= 0.0 || sta_sim::kinematics::a1::params(&db, u.key).is_none() {
            continue;
        }
        let cruise = warp.max(impulse);
        // A leg long enough to reach cruise and brake: 60 s at cruise.
        let dist = cruise * 60.0;
        let secs = 150;
        let t = fly(&db, u.key, (0.0, dist), secs);
        if !t.craft {
            continue;
        }
        checked += 1;
        let top = t.at.iter().map(|p| p.2).fold(0.0, f64::max);
        let jump = t.at.windows(2).map(|w| (w[1].2 - w[0].2).abs()).fold(0.0, f64::max);
        let (lx, lz, ls) = *t.at.last().unwrap();
        let passed = t.at.iter().map(|p| p.1).fold(f64::MIN, f64::max);
        let odf = &u.odf;
        if (top - cruise).abs() > cruise * 0.05 {
            failures.push(format!("{odf}: cruise {top:.1}, expected {cruise:.1}"));
        }
        if jump > cruise * 0.5 {
            failures.push(format!("{odf}: speed jumped {jump:.1} in one tick (cruise {cruise:.1})"));
        }
        if passed > dist + 1.0 {
            failures.push(format!("{odf}: overshot the goal by {:.1}", passed - dist));
        }
        let near = cruise.max(50.0);
        if ls > impulse * 0.5 || lx * lx + (lz - dist) * (lz - dist) > near * near {
            failures.push(format!("{odf}: did not come to rest at the goal: at ({lx:.0}, {lz:.0}) speed {ls:.1}"));
        }
        // A right-angle order from rest: it is moving while it turns (no pivot on the spot).
        let side = fly(&db, u.key, (cruise * 20.0, 0.0), 6);
        // Squared distance from the start: more than half a unit within 2 s.
        let moved = side.at.iter().take((2 * TICK_HZ) as usize).map(|p| p.0 * p.0 + p.1 * p.1).fold(0.0, f64::max);
        if moved <= 0.25 {
            failures.push(format!("{odf}: pivoted on the spot (squared distance {moved:.3} in 2 s)"));
        }
    }
    eprintln!("{checked} craft checked");
    assert!(checked > 0, "no craft has the model's parameters");
    assert!(failures.is_empty(), "{} of {checked}:\n{}", failures.len(), failures.join("\n"));
}
