//! `sta-bench desync` (NET-06) on two replays of the bench's content: where
//! the games first part, what exit codes mean, and that damaged inputs are
//! errors rather than panics. The router-side tests are in `sta_net`.

use std::path::{Path, PathBuf};
use std::process::{Command as Proc, Output};
use std::sync::Arc;

use sta_content::ContentDb;
use sta_core::{Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::replay::{DEFAULT_DIGEST_EVERY, Replay};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, WorldSource};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("desync-tests").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(args: &[&str]) -> Output {
    Proc::new(env!("CARGO_BIN_EXE_sta-bench")).args(args).output().unwrap()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn setup(content: Arc<ContentDb>) -> GameSetup {
    let seat = |i: u8| PlayerSetup { id: PlayerId(i), team: TeamId(i), race: "fx".into(), control: Control::Human };
    GameSetup { seed: 5, map: Arc::new(sta_content::fake::two_bases()), content, players: vec![seat(0), seat(1)], options: GameOptions::default() }
}

/// A game of 120 ticks: two ships spawned at tick 0, a move order at tick 30 and
/// whatever `extra` adds, which is applied at tick 60. Returns the replay.
fn game(extra: Vec<Command>) -> Replay {
    let content = Arc::new(sta_content::fake::db());
    let s = setup(content.clone());
    let mut sim = Sim::from_setup(&s);
    let mut replay = Replay::new(&s);
    replay.digests.push((0, sim.digest()));
    let cruiser = content.key("fx_cruiser").unwrap();
    for tick in 0..120u64 {
        let mut cmds = Vec::new();
        if tick == 0 {
            for (seq, (p, x)) in [(0u8, -100.0f32), (1, 100.0)].into_iter().enumerate() {
                cmds.push(Command {
                    player: PlayerId::SCRIPT,
                    seq: seq as u32,
                    kind: CommandKind::Edit(Edit::Spawn { odf: cruiser, player: PlayerId(p), pos: Vec3::from_f32([x, 0.0, 0.0]), heading: (Fixed::ZERO, Fixed::ONE) }),
                });
            }
        }
        if tick == 30 {
            let unit = sim.view().units.iter().find(|u| u.owner == PlayerId(0)).map(|u| u.id);
            let to = Vec3::from_f32([0.0, 0.0, 300.0]);
            cmds.push(Command { player: PlayerId(0), seq: 0, kind: CommandKind::Order(Order::Move { units: vec![unit.unwrap()], to, queue: false }) });
        }
        if tick == 60 {
            cmds.extend(extra.iter().cloned());
        }
        let out = sim.step(&cmds);
        replay.log_step(tick, &cmds, out.view.tick, out.view.digest, DEFAULT_DIGEST_EVERY);
    }
    replay
}

fn write(dir: &Path, name: &str, r: &Replay) -> String {
    let p = dir.join(name);
    std::fs::write(&p, r.encode()).unwrap();
    p.to_string_lossy().into_owned()
}

fn order(units: Vec<sta_core::EntityId>) -> Command {
    Command { player: PlayerId(0), seq: 1, kind: CommandKind::Order(Order::Stop { units }) }
}

#[test]
fn a_command_only_one_peer_ran_is_named_with_its_tick_section_and_entity() {
    let dir = scratch("diverged");
    let victim_unit = {
        let r = game(vec![]);
        let sim = r.play(Arc::new(sta_content::fake::db())).unwrap();
        sim.view().units.iter().find(|u| u.owner == PlayerId(0)).unwrap().id
    };
    let a = write(&dir, "seat0.star", &game(vec![]));
    let b = write(&dir, "seat1.star", &game(vec![order(vec![victim_unit])]));
    let json = dir.join("report.json");
    let o = run(&["desync", &a, &b, "--fake", "--json", json.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1), "{}{}", out(&o), err(&o));
    let md = out(&o);
    assert!(md.contains("Diverged at tick 61"), "{md}");
    assert!(md.contains("Only b applied") && md.contains("Stop"), "{md}");
    let j = std::fs::read_to_string(&json).unwrap();
    assert!(j.contains("\"verdict\":\"diverged\"") && j.contains("\"first_differing_tick\":61"), "{j}");
    assert!(j.contains(&format!("\"entity\":{}", victim_unit.0)), "{j}");
    assert!(j.contains("\"only_in_b\":[{\"tick\":60,\"player\":0,\"seq\":1"), "{j}");
    // `--json -` prints only the JSON, and the report does not change between runs.
    let again = run(&["desync", &a, &b, "--fake", "--json", "-"]);
    assert_eq!(again.status.code(), Some(1));
    assert_eq!(out(&again).trim_end(), j.trim_end());
}

#[test]
fn identical_replays_exit_zero() {
    let dir = scratch("same");
    let a = write(&dir, "a.star", &game(vec![]));
    let b = write(&dir, "b.star", &game(vec![]));
    let o = run(&["desync", &a, &b, "--fake"]);
    assert_eq!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    assert!(out(&o).contains("No divergence"), "{}", out(&o));
}

#[test]
fn unreadable_inputs_are_a_clear_error_and_not_a_verdict() {
    let dir = scratch("damaged");
    let good = game(vec![]);
    let a = write(&dir, "a.star", &good);
    let bytes = good.encode();
    let truncated = dir.join("trunc.star");
    std::fs::write(&truncated, &bytes[..bytes.len() / 2]).unwrap();
    let garbage = dir.join("junk.star");
    std::fs::write(&garbage, b"this is not a replay").unwrap();
    let missing = dir.join("missing.star");
    for bad in [&truncated, &garbage, &missing] {
        let o = run(&["desync", &a, bad.to_str().unwrap(), "--fake"]);
        assert_eq!(o.status.code(), Some(3), "{}{}", out(&o), err(&o));
        assert!(err(&o).contains("not a readable replay"), "{}", err(&o));
        assert!(!err(&o).contains("panicked"), "{}", err(&o));
    }
    // Usage and missing content are not panics either.
    assert_eq!(run(&["desync", &a]).status.code(), Some(2));
    let o = run(&["desync", &a, &a]);
    assert_eq!(o.status.code(), Some(3));
    assert!(err(&o).contains("pass --install"), "{}", err(&o));
}
