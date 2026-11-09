//! `sta-bench` against the real `sta64` binary: frame determinism and sessions.
//!
//! The frame tests need a real GPU, so they are opt-in: they print `SKIP` and
//! pass unless `STA64_GPU_TESTS=1`. Even then, `sta64` exits 77 when there is
//! no hardware adapter (software ones count as none) and the test skips; with
//! the variable set a timeout is a failure.

use std::path::{Path, PathBuf};
use std::process::Command;

use sta_bench::cli;

fn sta64() -> &'static str {
    env!("CARGO_BIN_EXE_sta64")
}

fn bench() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench")
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sta64-bench-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn args(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

/// Does a frame render here at all?
fn adapter(dir: &Path) -> bool {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 to run the GPU frame tests");
        return false;
    }
    let out = Command::new(sta64())
        .args(["--headless", "--clock", "bench", "--frames", "0", "--frame-dir"])
        .arg(dir)
        .arg("--scene")
        .arg(bench().join("scenes/planet.ini"))
        .output()
        .unwrap();
    match out.status.code() {
        Some(0) => true,
        Some(77) => {
            eprintln!("SKIP: no GPU adapter for headless frames: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or(""));
            false
        }
        c => panic!("headless frame run failed ({c:?}): {}", String::from_utf8_lossy(&out.stderr)),
    }
}

fn run_dir(root: &Path) -> PathBuf {
    std::fs::read_dir(root).unwrap().flatten().map(|e| e.path()).find(|p| p.join("results.json").is_file()).unwrap()
}

#[test]
fn planet_frames_repeat_byte_for_byte() {
    if !adapter(&tmp("probe")) {
        return;
    }
    let mut pngs = Vec::new();
    for run in 0..2 {
        let out = tmp(&format!("planet{run}"));
        let code = cli::main(args(&[
            "run", "planet", "--tier", "2", "--frames", "0,100", "--sta64", sta64(), "--out", out.to_str().unwrap(),
        ]));
        assert_eq!(code, 0, "sta-bench run planet --tier 2");
        let frames = run_dir(&out).join("planet/frames");
        let mut files: Vec<_> = std::fs::read_dir(&frames).unwrap().flatten().map(|e| e.path()).collect();
        files.sort();
        assert_eq!(files.len(), 2, "{files:?}");
        pngs.push((run_dir(&out), files.iter().map(|f| std::fs::read(f).unwrap()).collect::<Vec<_>>()));
    }
    assert!(pngs[0].1.iter().all(|p| p.starts_with(b"\x89PNG")));
    assert_eq!(pngs[0].1, pngs[1].1, "the same frame twice must be the same PNG");
    let diff = cli::main(args(&["diff", pngs[0].0.to_str().unwrap(), pngs[1].0.to_str().unwrap()]));
    assert_eq!(diff, 0, "sta-bench diff of the two runs");
}

#[test]
fn repeat_flag_checks_frames_in_one_call() {
    if !adapter(&tmp("probe2")) {
        return;
    }
    let out = tmp("repeat");
    let code = cli::main(args(&["run", "planet", "--tier", "2", "--repeat", "2", "--sta64", sta64(), "--out", out.to_str().unwrap()]));
    assert_eq!(code, 0);
}

#[cfg(unix)]
#[test]
fn session_start_drive_stop() {
    let base = tmp("session");
    let out = base.join("results");
    let o = out.to_str().unwrap();
    let list = |o: &str| cli::main(args(&["session", "list", "--out", o]));
    // `session start` prints its ID; find it as the directory it made.
    let code = cli::main(args(&["session", "start", "--scene", "planet", "Seed=7", "--sta64", sta64(), "--out", o]));
    assert_eq!(code, 0);
    let sessions = base.join("sessions");
    let id = std::fs::read_dir(&sessions).unwrap().flatten().next().unwrap().file_name().into_string().unwrap();
    assert_eq!(list(o), 0);
    let ok = cli::main(args(&["drive", "--session", &id, "tick", "query", "--out", o]));
    assert_eq!(ok, 0);
    let bad = cli::main(args(&["drive", "--session", &id, "frobnicate", "--out", o]));
    assert_eq!(bad, 1);
    assert_eq!(cli::main(args(&["session", "stop", "--session", &id, "--out", o])), 0);
    // An ended ID is an error.
    assert_eq!(cli::main(args(&["drive", "--session", &id, "query", "--out", o])), 1);
    let _ = std::fs::remove_dir_all(&sessions);
}
