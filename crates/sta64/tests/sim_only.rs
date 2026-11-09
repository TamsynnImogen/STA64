//! `sta64 --sim-only` end to end: the binary's digest equals the in-process
//! tier-1 runner's for the same scene (docs/TESTBENCH.md).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;

use sta_bench::parse::{BenchCommand, SceneFile};
use sta_bench::run::{Bench, RunConfig};

fn scene() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../bench/scenes/synthetic.ini")
}

fn in_process(ticks: u64) -> String {
    let scene = SceneFile::parse(&std::fs::read_to_string(scene()).unwrap()).unwrap();
    let mut b = Bench::build(Arc::new(RunConfig::fake()), &scene).unwrap();
    while b.tick() < ticks {
        assert!(b.advance());
    }
    b.exec(&BenchCommand::Query)
}

fn sta64() -> Command {
    Command::new(env!("CARGO_BIN_EXE_sta64"))
}

#[test]
fn sim_only_exits_zero_with_the_runners_digest() {
    let out = sta64()
        .args(["--sim-only", "--clock", "bench", "--ticks", "300", "--scene"])
        .arg(scene())
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    let want = in_process(300);
    assert!(want.contains("digest "), "{want}");
    assert_eq!(text.trim(), want);
}

#[test]
fn control_channel_answers_commands() {
    let mut child = sta64()
        .args(["--sim-only", "--control", "stdio", "--ticks", "30", "--scene"])
        .arg(scene())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"tick\n; comment\nnonsense words\nquery\n").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let lines: Vec<_> = String::from_utf8(out.stdout).unwrap().lines().map(str::to_owned).collect();
    assert_eq!(lines[0], "tick 0");
    assert!(lines[1].starts_with('!'), "{lines:?}");
    assert!(lines[2].starts_with("tick 0 objects 3 digest "), "{lines:?}");
    assert_eq!(lines[3], in_process(30));
}

#[test]
fn missing_scene_fails() {
    let out = sta64().args(["--sim-only", "--scene", "no/such.ini"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn headless_bevy_run_matches_too() {
    let out = sta64()
        .args(["--headless", "--clock", "bench", "--ticks", "120", "--scene"])
        .arg(scene())
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), in_process(120));
}
