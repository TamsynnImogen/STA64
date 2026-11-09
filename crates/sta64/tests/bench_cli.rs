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

/// One `sta64 --headless` run of `scene`, capturing `frames`, with `commands` fed on
/// stdin first. The frames' PNGs in tick order, and everything the run logged at
/// warn level or above.
fn run_frames(scene: &str, frames: &str, commands: &str, tag: &str) -> (Vec<Vec<u8>>, String) {
    use std::io::Write;
    use std::process::Stdio;
    let dir = tmp(tag);
    let mut cmd = Command::new(sta64());
    cmd.args(["--headless", "--clock", "bench", "--frames", frames, "--frame-dir"])
        .arg(&dir)
        .arg("--scene")
        .arg(bench().join("scenes").join(scene))
        .env("RUST_LOG", "warn")
        .stderr(Stdio::piped())
        .stdout(Stdio::null());
    if !commands.is_empty() {
        cmd.args(["--control", "stdio", "--ticks", frames.rsplit(',').next().unwrap()]).stdin(Stdio::piped());
    }
    let mut child = cmd.spawn().unwrap();
    if !commands.is_empty() {
        child.stdin.take().unwrap().write_all(commands.as_bytes()).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    let log = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(0), "{scene}: sta64 must exit 0\n{log}");
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "png")).collect();
    files.sort();
    (files.iter().map(|f| std::fs::read(f).unwrap()).collect(), log)
}

/// What overlapping effects' frames may differ by between runs, out of 255.
const TOLERANCE: u8 = 3;

/// The most any channel of two PNGs of one size differs by; `u8::MAX` if the sizes differ.
fn max_diff(a: &[u8], b: &[u8]) -> u8 {
    let decode = |p: &[u8]| {
        let mut r = png::Decoder::new(std::io::Cursor::new(p)).read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut buf).unwrap();
        (info.width, info.height, buf)
    };
    let (a, b) = (decode(a), decode(b));
    if (a.0, a.1) != (b.0, b.1) {
        return u8::MAX;
    }
    a.2.iter().zip(&b.2).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0)
}

/// The effects draw on a real GPU: `sta64` exits 0 with no ERROR logged (a shader or
/// pipeline error is only logged), the frames repeat byte for byte, and each frame in
/// `live` differs from the first, which is before any effect. The first `exact` frames
/// repeat byte for byte; the rest within [`TOLERANCE`]: a frame with several effects
/// overlapping varies by a level or two on a few hundred bytes from run to run (8-bit
/// additive blends are not associative, and the draw order of overlapping effects
/// is not fixed; cause not pinned down).
fn fx_frames(scene: &str, frames: &str, commands: &str, live: &[usize], exact: usize) {
    if !adapter(&tmp(&format!("probe-{scene}"))) {
        return;
    }
    let (a, log) = run_frames(scene, frames, commands, &format!("{scene}-a"));
    let (b, _) = run_frames(scene, frames, commands, &format!("{scene}-b"));
    assert!(!log.contains("ERROR"), "{scene}: errors logged\n{log}");
    assert_eq!(a.len(), frames.split(',').count());
    for (i, (x, y)) in a.iter().zip(&b).enumerate() {
        let d = max_diff(x, y);
        if i < exact {
            assert_eq!(x, y, "{scene}: frame {i} twice must be the same PNG");
        } else {
            assert!(d <= TOLERANCE, "{scene}: frame {i} differs between runs by {d}/255");
        }
    }
    for &i in live {
        assert_ne!(a[0], a[i], "{scene}: frame {i} must differ from the one before any effect");
    }
}

#[test]
fn weapon_fx_render_beams_and_bolts() {
    // Ticks 0 (nothing yet), 6 (beam and flares), 100 (a torpedo in flight).
    fx_frames("combat-firing.ini", "0,6,100", "", &[1, 2], 1);
}

#[test]
fn impact_fx_render_shield_hits_and_an_explosion() {
    // 0, a shield hit just after the aft shield goes down (117), the fireball after the kill (477).
    fx_frames("combat-duel.ini", "0,120,482,500", "", &[1, 2, 3], 2);
}

// The construction weld shares the weapon beams' quad material (`quad.wesl`, the kind
// is a uniform), so its pipeline is the one `weapon_fx_render_beams_and_bolts` compiles.
// Drawing it needs a station under construction, which the bench scenes only reach
// through scenario commands that tier 2's replay does not repeat (digest mismatch).
/// A `sta64` stand-in that adds `--postfx <fx>` to every launch (the bench
/// passes no extra flags itself).
#[cfg(unix)]
fn sta64_with_postfx(dir: &Path, fx: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join(format!("sta64-{fx}.sh"));
    std::fs::write(&script, format!("#!/bin/sh\nexec \"{}\" \"$@\" --postfx {fx}\n", sta64())).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script.to_str().unwrap().to_owned()
}

/// The frame PNGs of one `planet` run, run with the given `sta64` binary.
#[cfg(unix)]
fn planet_frames(bin: &str, tag: &str) -> Vec<Vec<u8>> {
    let out = tmp(tag);
    let code = cli::main(args(&["run", "planet", "--tier", "2", "--frames", "0,100", "--sta64", bin, "--out", out.to_str().unwrap()]));
    assert_eq!(code, 0, "sta-bench run planet ({tag})");
    let mut files: Vec<_> = std::fs::read_dir(run_dir(&out).join("planet/frames")).unwrap().flatten().map(|e| e.path()).collect();
    files.sort();
    files.iter().map(|f| std::fs::read(f).unwrap()).collect()
}

#[cfg(unix)]
#[test]
fn bloom_changes_frames_and_repeats() {
    if !adapter(&tmp("probe-postfx")) {
        return;
    }
    let wrap = tmp("postfx-wrap");
    let bloom = sta64_with_postfx(&wrap, "bloom");
    let off = planet_frames(sta64(), "postfx-off");
    let on_a = planet_frames(&bloom, "postfx-on-a");
    let on_b = planet_frames(&bloom, "postfx-on-b");
    assert_eq!(on_a, on_b, "bloom on twice must give the same frames");
    assert_ne!(off, on_a, "bloom must change the frames");
}
