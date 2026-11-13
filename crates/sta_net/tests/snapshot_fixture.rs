//! NET_DECISIONS N4.1: before any snapshot crosses the network, a snapshot
//! must continue to the same recorded digests on every platform CI runs.
//!
//! The scenario is scripted (fake test content, no game files): spawn forces,
//! give orders, run 300 ticks, snapshot, load, run 300 more. The digests of
//! the continuation are in `fixtures/snapshot_continuation.txt` and must be
//! the same on ubuntu, macOS and Windows (`full-ci` runs them all), and equal
//! to those of the run that never stopped. The recorded values are the
//! roster, movement and rng section digests (`sta_sim::digest`), which change
//! only when those parts of the state or their rules change; combat and
//! economy sections are checked live against the uninterrupted run only, so
//! work in those systems does not touch the fixture.
//!
//! Re-record with `STA64_BLESS_NET_FIXTURE=1 cargo test -p sta_net --test
//! snapshot_fixture` after a deliberate change to movement or the roster, and
//! say so in the PR.

use std::fmt::Write as _;
use std::sync::Arc;

use sta_core::{Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, WorldSource};

const SNAPSHOT_AT: u64 = 300;
const END: u64 = 600;
const EVERY: u64 = 30;

fn setup() -> GameSetup {
    GameSetup {
        seed: 0x5eed,
        map: Arc::new(sta_content::fake::two_bases()),
        content: Arc::new(sta_content::fake::db()),
        players: (0..3)
            .map(|p| PlayerSetup {
                id: PlayerId(p),
                team: TeamId(p),
                race: "fx".into(),
                control: Control::Human,
            })
            .collect(),
        options: GameOptions::default(),
    }
}

/// The commands applied at `tick`: spawns at 0, then a move order for each
/// seat's first unit every 45 ticks, to places that depend on the tick.
fn commands(sim: &Sim, setup: &GameSetup, tick: u64) -> Vec<Command> {
    let mut v = Vec::new();
    if tick == 0 {
        let cruiser = setup.content.key("fx_cruiser").unwrap();
        let frigate = setup.content.key("fx_frigate").unwrap();
        for p in 0..3i32 {
            for k in 0..3i32 {
                v.push(Command {
                    player: PlayerId::SCRIPT,
                    seq: (p * 3 + k) as u32,
                    kind: CommandKind::Edit(Edit::Spawn {
                        odf: if k % 2 == 0 { cruiser } else { frigate },
                        player: PlayerId(p as u8),
                        pos: Vec3::from_f32([(p * 220 - 220) as f32, 0.0, (k * 80 - 80) as f32]),
                        heading: (Fixed::ZERO, Fixed::ONE),
                    }),
                });
            }
        }
    } else if tick.is_multiple_of(45) {
        let view = sim.view();
        for p in 0..3u8 {
            let mine: Vec<_> = view.units.iter().filter(|u| u.owner == PlayerId(p)).map(|u| u.id).collect();
            if mine.is_empty() {
                continue;
            }
            let k = (tick / 45) as usize;
            v.push(Command {
                player: PlayerId(p),
                seq: (tick / 45) as u32,
                kind: CommandKind::Order(Order::Move {
                    units: vec![mine[k % mine.len()]],
                    to: Vec3::from_f32([
                        ((k * 53 + usize::from(p) * 17) % 600) as f32 - 300.0,
                        0.0,
                        ((k * 71 + usize::from(p) * 29) % 600) as f32 - 300.0,
                    ]),
                    queue: k.is_multiple_of(3),
                }),
            });
        }
    }
    v
}

fn run(sim: &mut Sim, setup: &GameSetup, from: u64, to: u64, record: &mut Vec<(u64, [u64; 5], u64)>) {
    for tick in from..to {
        let cmds = commands(sim, setup, tick);
        sim.step(&cmds);
        let t = tick + 1;
        if t > SNAPSHOT_AT && t.is_multiple_of(EVERY) {
            record.push((t, sim.section_digests().values(), sim.digest()));
        }
    }
}

#[test]
fn a_snapshot_continues_to_the_recorded_digests() {
    let setup = setup();
    // The run that never stops.
    let mut live = Sim::from_setup(&setup);
    let mut live_digests = Vec::new();
    run(&mut live, &setup, 0, SNAPSHOT_AT, &mut Vec::new());
    let bytes = live.snapshot();
    run(&mut live, &setup, SNAPSHOT_AT, END, &mut live_digests);

    // The run that stopped, was saved, and was loaded as another peer would.
    let mut first = Sim::from_setup(&setup);
    run(&mut first, &setup, 0, SNAPSHOT_AT, &mut Vec::new());
    assert_eq!(first.snapshot(), bytes, "two runs of the scenario must snapshot identically");
    let mut loaded = Sim::from_snapshot(&bytes, Some(setup.content.clone())).expect("snapshot loads");
    assert_eq!(loaded.digest(), first.digest());
    let mut loaded_digests = Vec::new();
    run(&mut loaded, &setup, SNAPSHOT_AT, END, &mut loaded_digests);

    assert_eq!(loaded_digests, live_digests, "a loaded snapshot must continue exactly as the live game");

    // Recorded values: the movement-independent-of-combat sections.
    let mut text = String::from("# tick roster movement rng (N4.1; re-record: see the test's docs)\n");
    for (t, s, _) in &loaded_digests {
        let _ = writeln!(text, "{t} {:#018x} {:#018x} {:#018x}", s[0], s[1], s[4]);
    }
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/snapshot_continuation.txt");
    if std::env::var_os("STA64_BLESS_NET_FIXTURE").is_some() {
        std::fs::write(path, &text).expect("write the fixture");
        return;
    }
    let recorded = std::fs::read_to_string(path).expect("fixtures/snapshot_continuation.txt");
    assert_eq!(
        recorded.replace("\r\n", "\n"),
        text,
        "the continuation differs from the recorded digests (a platform difference, or a deliberate change: re-record with STA64_BLESS_NET_FIXTURE=1)"
    );
}
