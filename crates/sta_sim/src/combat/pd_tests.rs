use crate::command::{Command, CommandKind, Order};
use crate::event::SimEvent;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::{Sim, WorldSource};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use std::sync::Arc;

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::ZERO,
        z: Fixed::from_int(z),
    }
}

const AHEAD: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);

/// `boats` torpedo cruisers of player 1 at (2000, 2000), and a station of
/// player 2 (one beam, one PD, 3000 hull) 800 away: out of every beam's
/// reach, inside the torpedoes' 900.
fn scene(boats: usize) -> (Sim, Vec<EntityId>, EntityId) {
    scene_with(sta_content::fake::db(), boats)
}

fn scene_with(db: sta_content::ContentDb, boats: usize) -> (Sim, Vec<EntityId>, EntityId) {
    let seat = |id| PlayerSetup {
        id: PlayerId(id),
        team: TeamId(id),
        race: "fx".into(),
        control: Control::Idle,
    };
    let (cruiser, station) = (db.key("fx_cruiser").unwrap(), db.key("fx_station").unwrap());
    let setup = GameSetup {
        seed: 7,
        map: Arc::new(sta_content::MapDef::empty()),
        content: Arc::new(db),
        players: vec![seat(1), seat(2)],
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let ids = (0..boats)
        .map(|_| {
            sim.spawn_def(cruiser, PlayerId(1), v(2000, 2000), AHEAD)
                .unwrap()
        })
        .collect();
    let st = sim
        .spawn_def(station, PlayerId(2), v(2000, 2800), AHEAD)
        .unwrap();
    (sim, ids, st)
}

/// Ticks of the torpedo `Fired`s, the PD `Fired`s (`target` is a point) and
/// the `Hit`s on the station, over `ticks` ticks.
fn run(sim: &mut Sim, units: Vec<EntityId>, target: EntityId, ticks: u64) -> [Vec<u64>; 3] {
    let order = Command {
        player: PlayerId(1),
        seq: 0,
        kind: CommandKind::Order(Order::Attack { units, target }),
    };
    let mut out = [Vec::new(), Vec::new(), Vec::new()];
    for t in 0..ticks {
        let cmds = if t == 0 { vec![order.clone()] } else { vec![] };
        for e in sim.step(&cmds).events {
            match e {
                SimEvent::Fired {
                    shooter,
                    hardpoint: 1,
                    ..
                } if shooter != target => out[0].push(t),
                SimEvent::Fired { shooter, .. } if shooter == target => out[1].push(t),
                SimEvent::Hit { target: x, .. } if x == target => out[2].push(t),
                _ => {}
            }
        }
    }
    out
}

/// One torpedo per 90 ticks (13.33 u/tick over 800 u): it is inside the PD's
/// 250 range after 42 moves (at tick 41 it has made 41 of them, 253 u out,
/// tick 42 puts it at 240 u) and the PD takes it on the next tick, 43.
#[test]
fn a_lone_torpedo_is_intercepted_every_time() {
    let (mut sim, boats, st) = scene(1);
    let [fired, pd, hits] = run(&mut sim, boats, st, 200);
    assert_eq!(fired, [0, 90, 180]);
    assert_eq!(pd, [43, 133]);
    assert!(hits.is_empty());
    assert_eq!(
        sim.weapons.shots_in_flight(),
        1,
        "the third is still flying"
    );
}

/// Three torpedoes at once reach the range together. The PD cycles every 10
/// ticks and the shots need 18-19 ticks to cross 250 u: two are taken, at
/// 43 and 53, the third hits at 61.
#[test]
fn a_saturated_pd_lets_shots_through() {
    let (mut sim, boats, st) = scene(3);
    let [fired, pd, hits] = run(&mut sim, boats, st, 80);
    assert_eq!(fired, [0, 0, 0]);
    assert_eq!(pd, [43, 53]);
    assert_eq!(hits, [61]);
    assert_eq!(sim.weapons.shots_in_flight(), 0);
    // Pinned: the digest of the run, PD timers included.
    assert_eq!(crate::unit_status_tests::digest_before_crew(&sim), 0x7ff0_e3dd_fafd_ce3a);
    // CON-35's keyed content hash (the line above folds in the old one).
    assert_eq!(sim.digest(), 0x0e39_00ab_aced_3ef6);
}

#[test]
fn same_run_same_digest() {
    let d = || {
        let (mut sim, boats, st) = scene(3);
        run(&mut sim, boats, st, 100);
        sim.digest()
    };
    assert_eq!(d(), d());
}

/// The fake content with `point_defense_cycles` off: PD does nothing and all
/// three torpedoes land together.
#[test]
fn pd_is_idle_without_the_feature() {
    let fake = sta_content::fake::db();
    let mut b = sta_content::ContentDb::builder().features(Default::default());
    for u in fake.units() {
        b = b.unit(u.clone());
    }
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    let db = b.build().unwrap();
    assert!(!db.features().point_defense_cycles);
    let (mut sim, boats, st) = scene_with(db, 3);
    let [fired, pd, hits] = run(&mut sim, boats, st, 80);
    assert_eq!(fired, [0, 0, 0]);
    assert!(pd.is_empty());
    assert_eq!(hits, [61, 61, 61]);
}

/// Three cruisers against a PD station, 45 ticks in: the PD is mid-cooldown
/// with torpedoes in flight; for the snapshot test.
pub(crate) fn mid_saturation() -> (Sim, Arc<sta_content::ContentDb>) {
    let (mut sim, boats, st) = scene(3);
    run(&mut sim, boats, st, 45);
    let db = Arc::clone(sim.content().unwrap());
    (sim, db)
}
