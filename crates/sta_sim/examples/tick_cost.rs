//! What a sim tick costs under load (#1017): an authored scene, timed tick by tick.
//!
//! ```sh
//! cargo run --release -p sta_sim --example tick_cost -- [craft per side] [ticks]
//! ```
//!
//! The scene is `sta_content::fake` only (its two craft, its map with a moon, a nebula and an
//! asteroid field): no game file is read. Two sides of immortal craft are sent back and forth
//! across the map and at each other, so the unit count stays what it was and the planner, the
//! steering, targeting and the weapons all have work for the whole run. It is a load, not a
//! game: the numbers say what the sim's own code costs on this machine and how a change moves
//! that, nothing about any original game.
//!
//! Printed: the time per tick (mean, median, 99th percentile, worst), the same for the ticks of
//! the last third (a long game's end), the time of the fixed-point division and square root on
//! a fixed set of values, the time of one whole-state digest, and the final digest. The digest is the check that a
//! change of cost changed nothing else: it must be the same before and after.

// The crate forbids clocks because simulation time is the tick count. This program is not the
// simulation: it stands outside and times it. No clock value reaches the sim, its commands or
// the digest (the orders go out at fixed ticks).
#![allow(clippy::disallowed_methods)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, WorldSource};

fn v(x: i32, z: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::from_int(z) }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Mean, median, 99th percentile and worst of `times`, in milliseconds.
fn summary(times: &[Duration]) -> String {
    let mut sorted = times.to_vec();
    sorted.sort();
    let at = |share: f64| sorted[((sorted.len() - 1) as f64 * share) as usize];
    let mean = sorted.iter().sum::<Duration>() / sorted.len().max(1) as u32;
    format!("mean {:.3} ms, median {:.3} ms, p99 {:.3} ms, worst {:.3} ms", ms(mean), ms(at(0.5)), ms(at(0.99)), ms(*sorted.last().unwrap()))
}

/// The time of `rounds` passes of `op` over a fixed set of value pairs, per call in nanoseconds,
/// and a checksum of the results (so the work is not optimised away and can be compared).
fn per_call(rounds: u32, op: impl Fn(Fixed, Fixed) -> Fixed) -> (f64, i64) {
    // world-sized values with fractional bits, both signs: a fixed sequence, no clock, no seed input
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        Fixed::from_bits((state as i64) >> 18)
    };
    let pairs: Vec<(Fixed, Fixed)> = (0..4096).map(|_| (next(), next())).collect();
    let mut sum = 0i64;
    let start = Instant::now();
    for _ in 0..rounds {
        for &(a, b) in &pairs {
            sum = sum.wrapping_add(std::hint::black_box(op(std::hint::black_box(a), std::hint::black_box(b))).to_bits());
        }
    }
    (start.elapsed().as_secs_f64() * 1e9 / (f64::from(rounds) * pairs.len() as f64), sum)
}

fn main() {
    let mut args = std::env::args().skip(1).map(|a| a.parse::<u32>().expect("a whole number"));
    let per_side = args.next().unwrap_or(60);
    let ticks = args.next().unwrap_or(3000);

    let content = Arc::new(sta_content::fake::db());
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Idle };
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 17,
        map: Arc::new(sta_content::fake::two_bases()),
        content: Arc::clone(&content),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    });
    let edit = |seq: u32, e: Edit| Command { player: PlayerId::BENCH, seq, kind: CommandKind::Edit(e) };
    let order = |player: u8, o: Order| Command { player: PlayerId(player), seq: 0, kind: CommandKind::Order(o) };

    // tick 0: two blocks of craft, one per side, either side of the map's middle
    let mut spawns = Vec::new();
    for side in 0..2u8 {
        for i in 0..per_side {
            let odf = if i % 3 == 0 { "fx_cruiser" } else { "fx_frigate" };
            let (col, row) = ((i % 10) as i32, (i / 10) as i32);
            let x = if side == 0 { -3500 + col * 90 } else { 3500 - col * 90 };
            let heading = if side == 0 { (Fixed::ONE, Fixed::ZERO) } else { (-Fixed::ONE, Fixed::ZERO) };
            spawns.push(edit(spawns.len() as u32, Edit::Spawn { odf: content.key(odf).unwrap(), player: PlayerId(side), pos: v(x, -400 + row * 90), heading }));
        }
    }
    sim.step(&spawns);
    let fleet = |sim: &Sim, side: u8| -> Vec<EntityId> { sim.view().units.iter().filter(|u| u.owner == PlayerId(side)).map(|u| u.id).collect() };
    let fleets = [fleet(&sim, 0), fleet(&sim, 1)];
    assert_eq!((fleets[0].len(), fleets[1].len()), (per_side as usize, per_side as usize), "every craft was spawned");
    let immortal: Vec<Command> = fleets.iter().flatten().enumerate().map(|(i, id)| edit(i as u32, Edit::Immortal(*id, true))).collect();
    sim.step(&immortal);

    // the same orders at the same ticks every run: across the map through the moon, the nebula
    // and the asteroid field, and at craft of the other side
    let goals = [v(3000, 2500), v(-3000, -2500), v(0, 3200), v(2500, -3200), v(-2500, 800)];
    let mut times = Vec::with_capacity(ticks as usize);
    for t in 0..ticks {
        let mut cmds = Vec::new();
        if t % 240 == 0 {
            let round = (t / 240) as usize;
            for side in 0..2usize {
                let (movers, fighters) = fleets[side].split_at(fleets[side].len() / 2);
                let goal = goals[(round + side * 2) % goals.len()];
                cmds.push(order(side as u8, Order::Move { units: movers.to_vec(), to: goal, queue: false, mode: None }));
                let target = fleets[1 - side][round % fleets[1 - side].len()];
                cmds.push(order(side as u8, Order::Attack { units: fighters.to_vec(), target }));
            }
        }
        let start = Instant::now();
        let out = sim.step(&cmds);
        times.push(start.elapsed());
        std::hint::black_box(out.view.tick);
    }

    println!("{} craft, {ticks} ticks", 2 * per_side);
    println!("per tick:        {}", summary(&times));
    println!("last third:      {}", summary(&times[times.len() * 2 / 3..]));
    let (div, div_sum) = per_call(200, |a, b| a / b);
    let (whole, whole_sum) = per_call(200, |a, b| a / Fixed::from_int(((b.to_bits() >> 40) | 1) as i32));
    let (root, root_sum) = per_call(200, |a, _| a.abs().sqrt());
    println!("Fixed / Fixed:   {div:.1} ns per call (checksum {div_sum:#x})");
    println!("Fixed / whole:   {whole:.1} ns per call (checksum {whole_sum:#x})");
    println!("Fixed::sqrt:     {root:.1} ns per call (checksum {root_sum:#x})");
    // every step computes the whole-state digest once, for the view it returns: its own cost here
    let start = Instant::now();
    for _ in 0..200 {
        std::hint::black_box(sim.digest());
    }
    println!("Sim::digest:     {:.3} ms per call (one per tick, through the view)", ms(start.elapsed()) / 200.0);
    println!("final digest:    {:#018x}", sim.digest());
}
