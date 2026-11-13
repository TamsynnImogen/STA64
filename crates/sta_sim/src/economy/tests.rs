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
    game_with_content(sta_content::fake::db())
}

fn game_with_content(content: sta_content::ContentDb) -> Sim {
    let seat = |id, control| PlayerSetup {
        id: PlayerId(id),
        team: TeamId(id),
        race: "fx".into(),
        control,
    };
    Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(MapDef::empty()),
        content: Arc::new(content),
        players: vec![seat(0, Control::Human), seat(1, Control::Idle)],
        options: GameOptions::default(),
    })
}

fn game_with_second_resource() -> Sim {
    let base = sta_content::fake::db();
    let mut builder = sta_content::ContentDb::builder()
        .profile(base.profile())
        .features(base.features().clone());
    for unit in base.units() {
        builder = builder.unit(unit.clone());
    }
    for weapon in base.weapons() {
        builder = builder.weapon(weapon.clone());
    }
    let mut latinum = sta_content::UnitDef::new("fx_latinum_moon", "planet");
    latinum.resource = Some(1);
    game_with_content(builder.unit(latinum).build().unwrap())
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

/// The renderer animates the mining cycle from these (RND-29): each phase change is
/// announced once, in order, with the resource the miner works.
#[test]
fn a_miner_announces_each_phase_of_its_cycle() {
    use crate::event::MinerPhase;
    let mut sim = game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    spawn(&mut sim, "fx_station", PlayerId(0), v(0, 0));
    let m = spawn(&mut sim, "fx_miner", PlayerId(0), v(0, 0));
    assert!(mine(&mut sim, 0, m, moon));
    let mut seen = Vec::new();
    for _ in 0..1250 {
        for e in sim.step(&[]).events {
            if let SimEvent::MinerPhase { id, phase, node, dock } = e {
                assert_eq!(dock.is_some(), phase == MinerPhase::Unloading, "{phase:?} {dock:?}");
                assert_eq!((id, node), (m, moon));
                seen.push(phase);
            }
        }
    }
    use MinerPhase::*;
    // The order tick itself already mined (see above), so the first phase heard of is the next one.
    assert_eq!(seen[..7], [ToDock, Unloading, ToNode, Mining, ToDock, Unloading, ToNode], "{seen:?}");
    // Never twice in a row: a phase is announced when it changes.
    assert!(seen.windows(2).all(|w| w[0] != w[1]));
}

/// A station shows `DOCKED` in the view exactly while a freighter unloads at it (RND-21),
/// and a world with no mining never raises it.
#[test]
fn a_station_is_docked_while_a_freighter_unloads_at_it() {
    use crate::view::UnitFlags;
    let mut sim = game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    let near = spawn(&mut sim, "fx_station", PlayerId(0), v(0, 0));
    let far = spawn(&mut sim, "fx_station", PlayerId(0), v(900, 900));
    let m = spawn(&mut sim, "fx_miner", PlayerId(0), v(0, 0));
    let docked = |sim: &Sim, id| sim.view().units.iter().find(|u| u.id == id).unwrap().flags.contains(UnitFlags::DOCKED);
    assert!(!docked(&sim, near) && !docked(&sim, m));
    assert!(mine(&mut sim, 0, m, moon));
    let mut seen = Vec::new();
    for _ in 0..1250 {
        let out = sim.step(&[]);
        let phase = out.events.iter().rev().find_map(|e| match e {
            SimEvent::MinerPhase { phase, .. } => Some(*phase),
            _ => None,
        });
        seen.push((phase, docked(&sim, near)));
        // Only the station the freighter unloads at; never the freighter or a far station.
        assert!(!docked(&sim, far) && !docked(&sim, m));
    }
    let on: Vec<bool> = seen.iter().map(|s| s.1).collect();
    assert!(on.contains(&true) && on.contains(&false), "docked for part of the cycle only");
    // Docked from the tick it unloads to the tick it leaves, in one run each time.
    let runs = on.windows(2).filter(|w| w[0] != w[1]).count();
    assert!(runs >= 3, "{runs} flag changes over two cycles");
    // The flag follows the unloading phase.
    for (i, (phase, d)) in seen.iter().enumerate() {
        if *phase == Some(crate::event::MinerPhase::Unloading) {
            assert!(d, "unloading at tick {i}");
        }
        if matches!(phase, Some(crate::event::MinerPhase::Mining | crate::event::MinerPhase::ToNode)) {
            assert!(!d, "{phase:?} at tick {i}");
        }
    }
}

/// A mined-out node with nothing carried ends the cycle: the miner is announced idle.
#[test]
fn a_miner_whose_node_is_gone_is_announced_idle() {
    use crate::event::MinerPhase;
    let mut sim = game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    spawn(&mut sim, "fx_station", PlayerId(0), v(0, 0));
    let m = spawn(&mut sim, "fx_miner", PlayerId(0), v(0, 5000));
    assert!(mine(&mut sim, 0, m, moon));
    sim.step(&[]);
    sim.despawn(moon, None);
    let mut idle = false;
    for _ in 0..5 {
        idle |= sim.step(&[]).events.iter().any(|e| matches!(e, SimEvent::MinerPhase { phase: MinerPhase::Idle, .. }));
    }
    assert!(idle);
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
    assert_eq!(crate::unit_status_tests::digest_before_crew(&sim), 0xc49f_a634_eb30_8438);
    assert_eq!(
        sim.digest(),
        0xa66a_04cf_f2c4_75b9,
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
        let carried = sim.economy.cargo(m).unwrap();
        assert!(carried > Fixed::ZERO);
        sim.step(&[cmd(PlayerId(0), 2, CommandKind::Order(o))]);
        assert!(!sim.economy.is_mining(m), "stop={stop}");
        assert_eq!(sim.economy.cargo(m), Some(carried));
        assert_eq!(sim.economy.miners.get(m).unwrap().phase, Phase::Idle);
        if !stop {
            assert_eq!(sim.unit(m).unwrap().target, Some(v(500, 0)));
        }
        sim.step(&[cmd(PlayerId::BENCH, 3, CommandKind::Edit(Edit::Kill(moon)))]);
        run(&mut sim, 20);
        assert_eq!(stock(&sim, 0, 0), Fixed::ZERO);
        assert_eq!(sim.economy.cargo(m), Some(carried));
        if !stop {
            assert_eq!(sim.unit(m).unwrap().target, Some(v(500, 0)));
        }
    }
}

#[test]
fn empty_miner_stops_approaching_a_removed_node() {
    let mut sim = game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(2000, 0));
    let miner = spawn(&mut sim, "fx_miner", PlayerId(0), v(0, 0));
    assert!(mine(&mut sim, 0, miner, moon));
    assert_eq!(sim.economy.cargo(miner), Some(Fixed::ZERO));
    assert_eq!(sim.unit(miner).unwrap().target, Some(v(2000, 0)));
    sim.step(&[cmd(PlayerId::BENCH, 5, CommandKind::Edit(Edit::Kill(moon)))]);
    assert!(!sim.economy.is_mining(miner));
    assert_eq!(sim.economy.cargo(miner), None);
    assert_eq!(sim.unit(miner).unwrap().target, None);
}

#[test]
fn loaded_freighter_delivers_after_its_old_node_is_removed() {
    let mut sim = game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    spawn(&mut sim, "fx_station", PlayerId(0), v(0, 0));
    let m = spawn(&mut sim, "fx_miner", PlayerId(0), v(0, 0));
    assert!(mine(&mut sim, 0, m, moon));
    run(&mut sim, 10);
    let carried = sim.economy.cargo(m).unwrap();
    let mut events = sim.step(&[cmd(PlayerId::BENCH, 5, CommandKind::Edit(Edit::Kill(moon)))]).events;
    for _ in 0..30 {
        events.extend(sim.step(&[]).events);
    }
    assert_eq!(stock(&sim, 0, 0), carried);
    assert!(events.contains(&SimEvent::Delivered {
        player: PlayerId(0), resource: 0, amount: carried
    }));
    assert_eq!(sim.economy.cargo(m), None);
    assert!(!sim.economy.is_mining(m));
}

#[test]
fn changing_nodes_delivers_old_resource_before_mining_new_resource() {
    let mut sim = game_with_second_resource();
    let old = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    let new = spawn(&mut sim, "fx_latinum_moon", NEUTRAL, v(0, 0));
    spawn(&mut sim, "fx_station", PlayerId(0), v(0, 0));
    let m = spawn(&mut sim, "fx_miner", PlayerId(0), v(0, 0));
    assert!(mine(&mut sim, 0, m, old));
    run(&mut sim, 10);
    let carried = sim.economy.cargo(m).unwrap();
    assert!(mine(&mut sim, 0, m, new));
    let mut missing_new = sim.clone();
    let mut missing_events = missing_new.step(&[cmd(
        PlayerId::BENCH, 5, CommandKind::Edit(Edit::Kill(new))
    )]).events;
    for _ in 0..30 {
        missing_events.extend(missing_new.step(&[]).events);
    }
    assert_eq!(stock(&missing_new, 0, 0), carried);
    assert!(missing_events.contains(&SimEvent::Delivered {
        player: PlayerId(0), resource: 0, amount: carried
    }));
    assert!(!missing_new.economy.is_mining(m));
    assert_eq!(missing_new.economy.cargo(m), None);
    let mut events = Vec::new();
    for _ in 0..30 {
        events.extend(sim.step(&[]).events);
    }
    assert_eq!(stock(&sim, 0, 0), carried);
    assert_eq!(stock(&sim, 0, 1), Fixed::ZERO);
    assert!(events.contains(&SimEvent::Delivered {
        player: PlayerId(0), resource: 0, amount: carried
    }));
    assert_eq!(sim.economy.miners.get(m).unwrap().res, 1);
    assert_eq!(sim.economy.miners.get(m).unwrap().node, new);
    assert!(sim.economy.cargo(m).unwrap() > Fixed::ZERO);
    run(&mut sim, 600);
    assert_eq!(stock(&sim, 0, 0), carried);
    assert!(stock(&sim, 0, 1) >= Fixed::from_int(150));
}

#[test]
fn interrupted_partial_unload_keeps_credited_stock_and_reports_only_remainder() {
    let mut sim = game_with_second_resource();
    let old = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    let new = spawn(&mut sim, "fx_latinum_moon", NEUTRAL, v(0, 0));
    spawn(&mut sim, "fx_station", PlayerId(0), v(0, 0));
    let m = spawn(&mut sim, "fx_miner", PlayerId(0), v(0, 0));
    assert!(mine(&mut sim, 0, m, old));
    run(&mut sim, 310);
    let credited = stock(&sim, 0, 0);
    let carried = sim.economy.cargo(m).unwrap();
    assert!(credited > Fixed::ZERO && carried > Fixed::ZERO);
    assert_eq!(credited + carried, Fixed::from_int(150));
    sim.step(&[cmd(PlayerId(0), 2, CommandKind::Order(Order::Stop { units: vec![m] }))]);
    assert_eq!(sim.economy.miners.get(m).unwrap().unloaded, Fixed::ZERO);
    assert_eq!(stock(&sim, 0, 0), credited);
    assert!(mine(&mut sim, 0, m, new));
    let mut events = Vec::new();
    for _ in 0..310 {
        events.extend(sim.step(&[]).events);
    }
    assert_eq!(stock(&sim, 0, 0), Fixed::from_int(150));
    assert!(events.contains(&SimEvent::Delivered {
        player: PlayerId(0), resource: 0, amount: carried
    }));
    assert!(!events.iter().any(|e| matches!(e, SimEvent::Delivered { resource: 0, amount, .. } if *amount == Fixed::from_int(150))));
    assert_eq!(sim.economy.miners.get(m).unwrap().res, 1);
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
fn idle_cargo_record_round_trips_with_phase_four_and_same_digest() {
    let mut sim = game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    let m = spawn(&mut sim, "fx_miner", PlayerId(0), v(0, 0));
    assert!(mine(&mut sim, 0, m, moon));
    run(&mut sim, 10);
    sim.step(&[cmd(PlayerId(0), 2, CommandKind::Order(Order::Stop { units: vec![m] }))]);
    let rec = sim.economy.to_rec();
    assert_eq!(rec.miners.len(), 1);
    assert_eq!(rec.miners[0].5, 4);
    let back = Economy::from_rec(&rec);
    assert_eq!(back, sim.economy);
    let mut a = Fnv::new();
    let mut b = Fnv::new();
    sim.economy.hash_into(&mut a);
    back.hash_into(&mut b);
    assert_eq!(a.0, b.0);
}

#[test]
fn dead_miner_drops_the_record() {
    let mut sim = game();
    let moon = spawn(&mut sim, "fx_moon", NEUTRAL, v(0, 0));
    let m = spawn(&mut sim, "fx_miner", PlayerId(0), v(0, 0));
    assert!(mine(&mut sim, 0, m, moon));
    sim.step(&[cmd(
        PlayerId::BENCH,
        5,
        CommandKind::Edit(crate::command::Edit::Kill(m)),
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
