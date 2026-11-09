//! `sta64 --headless --clock bench` writes the same sim marks as the
//! in-process runner, in Refit's names (docs/TESTBENCH.md, "Marks").

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use sta_bench::parse::SceneFile;
use sta_bench::run::{Bench, RunConfig};

const TICKS: u64 = 200;

fn scene() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../bench/scenes/combat-firing.ini")
}

fn in_process() -> String {
    let scene = SceneFile::parse(&std::fs::read_to_string(scene()).unwrap()).unwrap();
    let mut b = Bench::build(Arc::new(RunConfig::fake()), &scene).unwrap();
    while b.tick() < TICKS {
        assert!(b.advance());
    }
    sta_bench::run::marks::render(&b, &b.marks())
}

fn refit_names() -> Vec<String> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sta_bench/tests/fixtures/refit_marks.txt");
    std::fs::read_to_string(p)
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(str::to_owned)
        .collect()
}

#[test]
fn firing_marks_match_runner_and_refit_names() {
    let dir = std::env::temp_dir().join(format!("sta64-marks-it-{}", std::process::id()));
    let out = Command::new(env!("CARGO_BIN_EXE_sta64"))
        .args(["--headless", "--clock", "bench", "--ticks", &TICKS.to_string(), "--scene"])
        .arg(scene())
        .env("STA64_BENCH_OUT", &dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let got = std::fs::read_to_string(dir.join("marks")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let want = in_process();
    assert!(want.contains("fire.start") && want.contains("hit"), "{want}");
    assert_eq!(got, want);
    let names = refit_names();
    for line in got.lines() {
        let kind = line.split_whitespace().nth(1).unwrap();
        assert!(names.iter().any(|n| n == kind), "`{kind}` is not a Refit mark name");
    }
}

#[test]
fn every_derivable_kind_is_a_refit_name() {
    let names = refit_names();
    for k in sta_bench::marks::KINDS {
        assert!(names.iter().any(|n| n == k), "{k}");
    }
}
