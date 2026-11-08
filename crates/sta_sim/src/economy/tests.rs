use super::*;
use crate::command::{Command, CommandKind, Order};
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::view::PlayerView;
use crate::world::NEUTRAL;
use crate::{Sim, WorldSource};
use sta_content::MapDef;
use sta_core::TeamId;
use std::sync::Arc;

const TICKS_PER_MINUTE: u32 = 30 * 60;

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 {
        x: Fixed::from_int(x),
        y: Fixed::ZERO,
        z: Fixed::from_int(z),
    }
}

fn game() -> Sim {
    let seat = |id, control| PlayerSetup {
        id: PlayerId(id),
        team: TeamId(id),
        race: "fx".into(),
        control,
    };
    Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(MapDef::empty()),
        content: Arc::new(sta_content::fake::db()),
        players: vec![seat(0, Control::Human), seat(1, Control::Idle)],
        options: GameOptions::default(),
    })
}

fn cmd(player: PlayerId, seq: u32, kind: CommandKind) -> Command {
    Command { player, seq, kind }
}

/// Spawn `odf` through the bench edit; the id it got.
fn spawn(sim: &mut Sim, odf: &str, player: PlayerId, at: Vec3) -> EntityId {
    let key = sim.content().unwrap().key(odf).unwrap();
    let e = crate::command::Edit::Spawn {
        odf: key,
        player,
        pos: at,
        heading: (Fixed::ZERO, Fixed::ONE),
    };
    let id = sim.units.next_id();
    sim.step(&[cmd(PlayerId::BENCH, 0, CommandKind::Edit(e))]);
    assert!(sim.unit(id).is_some(), "spawn {odf}");
    id
}

fn mine(sim: &mut Sim, player: u8, miner: EntityId, node: EntityId) -> bool {
    let o = Order::Mine {
        units: vec![miner],
        node,
    };
    let out = sim.step(&[cmd(PlayerId(player), 1, CommandKind::Order(o))]);
    !out.events
        .iter()
        .any(|e| matches!(e, SimEvent::Rejected { .. }))
}

fn stock(sim: &Sim, p: u8, res: usize) -> Fixed {
    let view = sim.view();
    let pv: &PlayerView = view.players.iter().find(|x| x.id == PlayerId(p)).unwrap();
    pv.resources.get(res).copied().unwrap_or(Fixed::ZERO)
}

/// Ticks (counted from the order) at which a delivery finished, and the stock
/// at the end of `ticks`.
fn run(sim: &mut Sim, ticks: u32) -> Vec<u32> {
    let mut at = Vec::new();
    for t in 1..=ticks {
        let out = sim.step(&[]);
        if out
            .events
            .iter()
            .any(|e| matches!(e, SimEvent::Delivered { .. }))
        {
            at.push(t);
        }
    }
    at
}

/// Node and station on the same spot: no travel, so a trip is exactly
/// 150 / 0.5 mining ticks plus 150 / 0.5 unloading ticks = 600 ticks, three
/// trips and 450 dilithium a minute.
#[test]
fn colocated_freighter_earns_450_a_minute() {
    let mut sim = game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    spawn(&mut sim, "fx_station", PlayerId(0), v(0, 0));
    let m = spawn(&mut sim, "fx_miner", PlayerId(0), v(0, 0));
    assert!(mine(&mut sim, 0, m, moon));
    // The order tick itself already mined once (the order is applied before
    // the step), so a minute from there is one tick short of 3 trips.
    let done = run(&mut sim, TICKS_PER_MINUTE - 1);
    assert_eq!(done, [599, 1199, 1799]);
    assert_eq!(stock(&sim, 0, 0), Fixed::from_int(450));
    assert_eq!(stock(&sim, 1, 0), Fixed::ZERO);
}

#[test]
fn delivered_events_carry_the_trip() {
    let mut sim = game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    spawn(&mut sim, "fx_station", PlayerId(0), v(0, 0));
    let m = spawn(&mut sim, "fx_miner", PlayerId(0), v(0, 0));
    assert!(mine(&mut sim, 0, m, moon));
    let mut got = Vec::new();
    for _ in 0..700 {
        got.extend(
            sim.step(&[])
                .events
                .into_iter()
                .filter(|e| matches!(e, SimEvent::Delivered { .. })),
        );
    }
    assert_eq!(
        got,
        [SimEvent::Delivered {
            player: PlayerId(0),
            resource: 0,
            amount: Fixed::from_int(150)
        }]
    );
}

/// Moon and station 1500 apart: the freighter must cover 1500 - 150 - 150
/// each way at no more than 110 units a second, so a trip takes at least
/// 600 + 2 * ceil(1200 / 110 * 30) ticks and every trip takes the same.
#[test]
fn freighter_loops_moon_to_station_with_steady_income() {
    let mut sim = game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    spawn(&mut sim, "fx_station", PlayerId(0), v(1500, 0));
    let m = spawn(&mut sim, "fx_miner", PlayerId(0), v(0, 0));
    assert!(mine(&mut sim, 0, m, moon));
    let minutes = 5;
    let done = run(&mut sim, TICKS_PER_MINUTE * minutes - 1);
    let min_trip = 600 + 2 * (1200 * 30 / 110);
    let gaps: Vec<u32> = done.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(done.len() >= 3, "{done:?}");
    // The first trip starts from rest at the moon; later ones turn around at speed.
    assert!(
        gaps[1..].iter().all(|&g| g == gaps[1]),
        "steady cycle: {gaps:?}"
    );
    assert!(
        gaps[1] >= min_trip && gaps[1] <= 2 * min_trip,
        "trip {} vs minimum {min_trip}",
        gaps[1]
    );
    // Whole trips are credited whole; a trip in progress adds part of one.
    let total = stock(&sim, 0, 0);
    let n = done.len() as i32;
    assert!(
        total >= Fixed::from_int(150 * n) && total < Fixed::from_int(150 * (n + 1)),
        "{total:?} after {n} trips"
    );
    // Pinned: the whole run, so any change to movement or economy shows.
    assert_eq!(
        sim.digest(),
        0xc49f_a634_eb30_8438,
        "digest {:#x}",
        sim.digest()
    );
}

#[test]
fn nearest_station_is_the_destination_and_other_players_do_not_count() {
    let mut sim = game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    spawn(&mut sim, "fx_station", PlayerId(1), v(0, 0));
    spawn(&mut sim, "fx_station", PlayerId(0), v(1500, 0));
    let m = spawn(&mut sim, "fx_miner", PlayerId(0), v(0, 0));
    assert!(mine(&mut sim, 0, m, moon));
    run(&mut sim, 500);
    assert_eq!(sim.unit(m).unwrap().target, Some(v(1500, 0)));
    assert_eq!(stock(&sim, 1, 0), Fixed::ZERO);
}

#[test]
fn no_station_waits_with_a_full_hold() {
    let mut sim = game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    let m = spawn(&mut sim, "fx_miner", PlayerId(0), v(0, 0));
    assert!(mine(&mut sim, 0, m, moon));
    run(&mut sim, 1000);
    assert_eq!(sim.economy.cargo(m), Some(Fixed::from_int(150)));
    assert_eq!(stock(&sim, 0, 0), Fixed::ZERO);
}

#[test]
fn bad_mine_orders_are_rejected() {
    let mut sim = game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    let st = spawn(&mut sim, "fx_station", PlayerId(0), v(0, 0));
    let cruiser = spawn(&mut sim, "fx_cruiser", PlayerId(0), v(0, 0));
    let m = spawn(&mut sim, "fx_miner", PlayerId(0), v(0, 0));
    assert!(!mine(&mut sim, 0, cruiser, moon), "not a freighter");
    assert!(!mine(&mut sim, 0, m, st), "not a resource");
    assert!(!mine(&mut sim, 1, m, moon), "not the player's");
    assert!(!mine(&mut sim, 0, m, EntityId(9999)), "no such node");
    assert!(!sim.economy.is_mining(m));
}

#[test]
fn move_and_stop_end_mining() {
    for stop in [true, false] {
        let mut sim = game();
        let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
        spawn(&mut sim, "fx_station", PlayerId(0), v(0, 0));
        let m = spawn(&mut sim, "fx_miner", PlayerId(0), v(0, 0));
        assert!(mine(&mut sim, 0, m, moon));
        run(&mut sim, 10);
        assert!(sim.economy.is_mining(m));
        let o = if stop {
            Order::Stop { units: vec![m] }
        } else {
            Order::Move {
                units: vec![m],
                to: v(500, 0),
                queue: false,
            }
        };
        sim.step(&[cmd(PlayerId(0), 2, CommandKind::Order(o))]);
        assert!(!sim.economy.is_mining(m), "stop={stop}");
    }
}

#[test]
fn cargo_edit_sets_stock_for_the_bench_only() {
    use crate::command::Edit;
    let mut sim = game();
    let set = |from: PlayerId, res: u8| {
        cmd(
            from,
            0,
            CommandKind::Edit(Edit::Cargo(PlayerId(1), res, Fixed::from_int(500))),
        )
    };
    let out = sim.step(&[set(PlayerId(1), 2)]);
    assert!(out
        .events
        .iter()
        .any(|e| matches!(e, SimEvent::Rejected { .. })));
    assert!(stock(&sim, 1, 2) == Fixed::ZERO);
    let out = sim.step(&[set(PlayerId::BENCH, 9)]);
    assert!(out
        .events
        .iter()
        .any(|e| matches!(e, SimEvent::Rejected { .. })));
    let out = sim.step(&[set(PlayerId::BENCH, 2)]);
    assert!(out.events.is_empty());
    assert_eq!(stock(&sim, 1, 2), Fixed::from_int(500));
    assert_eq!(sim.view().players[1].resources.len(), RESOURCES);
    assert!(sim.economy.spend(PlayerId(1), 2, Fixed::from_int(200)));
    assert!(!sim.economy.spend(PlayerId(1), 2, Fixed::from_int(400)));
    assert_eq!(stock(&sim, 1, 2), Fixed::from_int(300));
}

#[test]
fn no_economy_state_leaves_the_digest_alone() {
    let a = game();
    let mut b = game();
    assert!(b.economy.is_empty());
    assert_eq!(a.digest(), b.digest());
    b.step(&[]);
    assert!(b.economy.is_empty());
}

#[test]
fn rec_round_trips_mid_trip() {
    let mut sim = game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    spawn(&mut sim, "fx_station", PlayerId(0), v(0, 0));
    let m = spawn(&mut sim, "fx_miner", PlayerId(0), v(0, 0));
    assert!(mine(&mut sim, 0, m, moon));
    run(&mut sim, 1000);
    let rec = sim.economy.to_rec();
    let back = Economy::from_rec(&rec);
    assert_eq!(back, sim.economy);
    let mut h1 = Fnv::new();
    let mut h2 = Fnv::new();
    sim.economy.hash_into(&mut h1);
    back.hash_into(&mut h2);
    assert_eq!(h1.0, h2.0);
    let bytes = postcard::to_allocvec(&rec).unwrap();
    assert_eq!(postcard::from_bytes::<EconomyRec>(&bytes).unwrap(), rec);
}

#[test]
fn dead_node_or_miner_drops_the_order() {
    let mut sim = game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    let m = spawn(&mut sim, "fx_miner", PlayerId(0), v(0, 0));
    assert!(mine(&mut sim, 0, m, moon));
    sim.step(&[cmd(
        PlayerId::BENCH,
        5,
        CommandKind::Edit(crate::command::Edit::Kill(moon)),
    )]);
    run(&mut sim, 2);
    assert!(!sim.economy.is_mining(m));
}

/// A freighter 1000 ticks into its first trip, loaded and heading for the station, for the snapshot test.
pub(crate) fn mid_mining() -> (Sim, Arc<sta_content::ContentDb>) {
    let mut sim = game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    spawn(&mut sim, "fx_station", PlayerId(0), v(1500, 0));
    let m = spawn(&mut sim, "fx_miner", PlayerId(0), v(0, 0));
    assert!(mine(&mut sim, 0, m, moon));
    run(&mut sim, 1000);
    let db = Arc::clone(sim.content().unwrap());
    (sim, db)
}
