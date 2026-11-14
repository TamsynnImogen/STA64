//! Install-gated (`STA64_TEST_A1_INSTALL`, an Armada I root; skipped when
//! unset): on stock Armada I a scout given a plain move across a map goes to
//! warp and arrives in about the time `warpSpeed` predicts, while a short
//! move stays at impulse speed (SIM-64). The physics keys `impulseSpeed`,
//! `warpSpeed`, `tooCloseForWarp` and `headingCloseEnoughForWarp` are read
//! from the install; the rule is read from the executable (static), the
//! arrival times are not measured on a running Armada I.

use std::path::Path;
use std::sync::Arc;

use sta_content::{ContentDb, OdfIndex, Profile};
use sta_core::{Fixed, PlayerId, TeamId, Vec3, TICK_HZ};
use sta_sim::command::{Command, CommandKind, Order};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, WorldSource};
use sta_vfs::Vfs;

fn n(v: i32) -> Fixed {
    Fixed::from_int(v)
}

#[test]
fn a_scout_crossing_a_map_warps_and_a_short_move_stays_at_impulse() {
    let Ok(root) = std::env::var("STA64_TEST_A1_INSTALL") else { return };
    let vfs = Vfs::mount_install(Path::new(&root), None).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    assert_eq!(db.profile(), Profile::Sta1Classic, "STA64_TEST_A1_INSTALL must name an Armada I install");
    let db = Arc::new(db);
    let key = db.key("fscout").expect("Federation scout");
    let m = db.unit(key).unwrap().movement;
    assert_eq!((m.max_speed, m.warp_speed, m.too_close_for_warp, m.heading_close_for_warp), (n(20), n(100), n(10), n(30)));
    let seat = |id: u8| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fed".into(), control: Control::Idle };
    let go = |dist: i32, secs: u32| {
        let mut sim = Sim::from_setup(&GameSetup {
            seed: 7,
            map: Arc::new(sta_content::MapDef::empty()),
            content: db.clone(),
            players: vec![seat(0), seat(1)],
            options: GameOptions::default(),
        });
        let s = sim.spawn_def(key, PlayerId(0), Vec3::ZERO, (Fixed::ZERO, Fixed::ONE)).unwrap();
        let to = Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: n(dist) };
        let (mut top, mut arrived) = (Fixed::ZERO, None);
        for i in 0..secs * TICK_HZ {
            let cmds = if i == 0 { vec![Command { player: PlayerId(0), seq: 0, kind: CommandKind::Order(Order::Move { units: vec![s], to, queue: false }) }] } else { vec![] };
            WorldSource::step(&mut sim, &cmds);
            let u = sim.unit(s).unwrap();
            top = top.max(u.speed);
            if arrived.is_none() && (to.z - u.pos.z).abs() < n(60) {
                arrived = Some(i / TICK_HZ);
            }
        }
        (top, arrived)
    };
    // Across a map: 4000 units are 200 s at impulse, about 40 s at warp 100 (plus a short ramp).
    let (top, arrived) = go(4000, 90);
    assert!(top >= n(95), "never warped: {top:?}");
    let secs = arrived.expect("did not arrive");
    assert!((35..=60).contains(&secs), "arrived after {secs} s");
    // A short move: inside the drop-out lead the scout never leaves impulse speed.
    let (top, _) = go(40, 20);
    assert!(top <= n(21), "short move warped: {top:?}");
}
