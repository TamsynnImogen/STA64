//! AI-vs-AI soak (AI-03): the skirmish AI on both seats of the fake
//! `two_bases` map, ten seeded games, the first few played twice.
//!
//! The seed reaches the games through the AI's seeded draws (wave sizes
//! between the AIP's `min_attack_force` and the growing ceiling up to
//! `max_attack_force`), so seeds play different games. The soak asserts that:
//! different decision ticks, and both seats win some of the ten (a winner
//! that never changes is an ordering bias, as the seat-staggered planning
//! once was). Note that `Sim::digest()` also hashes the RNG state, so digests
//! differ per seed regardless; the outcome fields are what show the games
//! differ.
//!
//! Done when: ten seeded games end without panic, and a seed gives the same
//! digest on every run. "End" means the game plays to a winner inside the
//! tick bound; a game that does not is reported and fails the soak, so a
//! stalemate is visible instead of hidden. The harness is the same game-mode
//! stand-in as AI-02's tests: it gives each seat a station, a yard and 1000
//! dilithium by bench edits, since `Sim::from_setup` spawns nothing at start
//! positions yet.

use std::sync::Arc;

use sta_ai::{Registry, SKIRMISH};
use sta_core::{Fixed, PlayerId, TeamId, Vec3, TICK_HZ};
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

/// The ten soak seeds.
const SEEDS: [u64; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
/// Bound of the default soak: sim minutes per game.
const MINUTES: u64 = 60;
/// Bound of the long soak.
const LONG_MINUTES: u64 = 90;
/// Seeds that are played a second time to compare in the default soak.
const REPLAYED: usize = 3;
/// A digest is folded into the trace every this many ticks.
const TRACE_EVERY: u64 = 5 * TICK_HZ as u64;

fn setup(seed: u64) -> GameSetup {
    let seat = |id: u8| PlayerSetup {
        id: PlayerId(id),
        team: TeamId(id),
        race: "fx".into(),
        control: Control::Ai(SKIRMISH.into()),
    };
    GameSetup {
        seed,
        map: Arc::new(sta_content::fake::two_bases()),
        content: Arc::new(sta_content::fake::db()),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    }
}

/// A station, a yard and 1000 dilithium for each seat.
fn start(sim: &mut Sim, setup: &GameSetup) {
    let key = |odf: &str| setup.content.key(odf).unwrap();
    let mut seq = 0;
    let mut cmd = |e: Edit| {
        seq += 1;
        Command {
            player: PlayerId::BENCH,
            seq,
            kind: CommandKind::Edit(e),
        }
    };
    let mut batch = Vec::new();
    for (i, s) in setup.map.starts.iter().enumerate() {
        let p = PlayerId(i as u8);
        for (odf, dz) in [("fx_station", 0), ("fx_yard", 300)] {
            let pos = Vec3 {
                x: s.pos.x,
                y: s.pos.y,
                z: s.pos.z + Fixed::from_int(dz),
            };
            batch.push(cmd(Edit::Spawn {
                odf: key(odf),
                player: p,
                pos,
                heading: s.heading,
            }));
        }
        batch.push(cmd(Edit::Cargo(p, 0, Fixed::from_int(1000))));
    }
    sim.step(&batch);
}

#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    /// Tick the game was decided at, if it was inside the bound.
    won_at: Option<u64>,
    winners: Option<Vec<PlayerId>>,
    /// Ticks played.
    ticks: u64,
    /// Final state digest.
    digest: u64,
    /// FNV fold of the digest every [`TRACE_EVERY`] ticks: a divergence
    /// anywhere in the game shows even if the end state converges.
    trace: u64,
    rejected: usize,
    /// Units per seat at the end, and the mean x of each seat's units.
    units: [(usize, i64); 2],
}

fn play(seed: u64, minutes: u64) -> Outcome {
    let setup = setup(seed);
    let reg = Registry::builtin();
    let mut sim = Sim::from_setup(&setup);
    start(&mut sim, &setup);
    let mut ctl: Vec<_> = setup
        .players
        .iter()
        .map(|p| match &p.control {
            Control::Ai(n) => reg.create_for_seat(n, &setup, p.id).unwrap(),
            _ => unreachable!(),
        })
        .collect();
    let (mut events, mut rejected, mut trace, mut won_at) =
        (Vec::<SimEvent>::new(), 0, 0xcbf2_9ce4_8422_2325_u64, None);
    let bound = minutes * 60 * u64::from(TICK_HZ);
    let mut ticks = 0;
    while ticks < bound {
        let view = sim.view();
        let mut out = Vec::new();
        for c in &mut ctl {
            c.think(&view, &events, &mut out);
        }
        let o = sim.step(&out);
        ticks += 1;
        rejected += o
            .events
            .iter()
            .filter(|e| matches!(e, SimEvent::Rejected { .. }))
            .count();
        events = o.events;
        if ticks % TRACE_EVERY == 0 {
            trace = (trace ^ sim.digest()).wrapping_mul(0x0100_0000_01b3);
        }
        if sim.winners().is_some() {
            won_at = Some(o.view.tick);
            break;
        }
    }
    let view = sim.view();
    let seat = |p: u8| {
        let mine: Vec<_> = view
            .units
            .iter()
            .filter(|u| u.owner == PlayerId(p))
            .collect();
        let x =
            mine.iter().map(|u| u.pos.x.to_f32() as i64).sum::<i64>() / mine.len().max(1) as i64;
        (mine.len(), x)
    };
    Outcome {
        won_at,
        winners: sim.winners(),
        ticks,
        digest: sim.digest(),
        trace,
        rejected,
        units: [seat(0), seat(1)],
    }
}

fn soak(minutes: u64, replayed: usize) {
    let mut undecided = Vec::new();
    let (mut won_at, mut winners) = (
        std::collections::BTreeSet::new(),
        std::collections::BTreeSet::new(),
    );
    for (i, seed) in SEEDS.into_iter().enumerate() {
        let a = play(seed, minutes);
        eprintln!(
            "AI-03: seed {seed:>2}: ticks {:>6} won_at {:?} winners {:?} rejected {} units(count, mean x) {:?} digest {:016x} trace {:016x}",
            a.ticks, a.won_at, a.winners, a.rejected, a.units, a.digest, a.trace
        );
        if i < replayed {
            assert_eq!(
                a,
                play(seed, minutes),
                "seed {seed}: two runs of the same game differ"
            );
        }
        assert_eq!(
            a.rejected, 0,
            "seed {seed}: the AI sent orders the sim refused"
        );
        if a.won_at.is_none() {
            undecided.push(seed);
        }
        won_at.insert(a.won_at);
        winners.extend(a.winners.iter().flatten().copied());
    }
    assert!(
        undecided.is_empty(),
        "games with no winner inside {minutes} sim minutes: seeds {undecided:?}"
    );
    assert!(
        won_at.len() >= SEEDS.len() / 2,
        "seeds play the same game: only {} distinct decisive ticks in {}",
        won_at.len(),
        SEEDS.len()
    );
    assert_eq!(
        winners.len(),
        2,
        "one seat wins every game ({winners:?}): an ordering bias"
    );
}

#[test]
fn ten_seeded_ai_games_end_without_panic_and_replay_identically() {
    soak(MINUTES, REPLAYED);
}

/// A digest pinned for the 4-OS matrix on `main`: any platform or build that
/// plays the first three sim minutes differently shows up here. A change to
/// the AI or the sim that alters those minutes updates the numbers.
#[test]
fn three_minute_digest_is_pinned() {
    let o = play(1, 3);
    assert_eq!(
        (o.digest, o.trace),
        (7447273215544161350, 6333087146984546231),
        "pinned digest and trace of seed 1 at 3 sim minutes"
    );
}

#[test]
#[ignore = "long soak: 90 sim minutes per game, twice per seed"]
fn long_soak() {
    soak(LONG_MINUTES, SEEDS.len());
}
