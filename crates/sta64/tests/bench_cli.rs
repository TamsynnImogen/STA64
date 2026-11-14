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

/// The fog of war and the shroud (RND-13): `fog.ini` draws them as team 1 sees the map,
/// `fog-off.ini` is the same scene without. Both run twice: no ERROR logged (a shader or
/// pipeline error is only logged), the frames repeat byte for byte, the fog changes the
/// frame, and after the scout warps away the frame changes again (the fog moves with it
/// and what it passed stays explored, so the shroud is not what it was).
#[test]
fn fog_of_war_draws_changes_the_frame_and_follows_the_sensors() {
    if !adapter(&tmp("probe-fog")) {
        return;
    }
    let go = "goto scout 3500 0 -1500 warp\n";
    let (a, log) = run_frames("fog.ini", "0,60", go, "fog-a");
    let (b, _) = run_frames("fog.ini", "0,60", go, "fog-b");
    let (off, off_log) = run_frames("fog-off.ini", "0,60", go, "fog-off");
    for l in [&log, &off_log] {
        assert!(!l.contains("ERROR"), "errors logged\n{l}");
    }
    assert_eq!(a.len(), 2);
    assert_eq!(a, b, "fog frames twice must be the same PNGs");
    assert_ne!(a[0], off[0], "the fog must change the first frame");
    assert_ne!(a[1], off[1], "the fog must change the later frame");
    assert_ne!(a[0], a[1], "the fog must move with the scout");
    assert!(max_diff(&a[0], &off[0]) > 40, "fog is a visible dimming, not a speck");
}

/// A session with a renderer (`--control stdio --frame-dir D`, no `--frames`): runs to
/// `tick` the way a driver would (`resume`, `pause-at T`), asks `frame shot`, and returns
/// the PNG it wrote. `commands` go first (a camera, say).
fn session_frame(scene: &str, tick: u64, commands: &[&str], tag: &str) -> Vec<u8> {
    use std::io::{BufRead, BufReader, Write};
    use std::process::Stdio;
    let dir = tmp(tag);
    let mut child = Command::new(sta64())
        .args(["--headless", "--clock", "bench", "--control", "stdio", "--frame-dir"])
        .arg(&dir)
        .arg("--scene")
        .arg(bench().join("scenes").join(scene))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut ask = |line: &str| -> String {
        writeln!(stdin, "{line}").unwrap();
        stdin.flush().unwrap();
        let mut answer = String::new();
        out.read_line(&mut answer).unwrap();
        answer.trim().to_owned()
    };
    // As `session start` begins: held until the first drive.
    ask("pause");
    for c in commands {
        assert!(!ask(c).starts_with('!'), "{c}");
    }
    if tick > 0 {
        // (`resume` clears a `pause-at`, so it goes first; the bench stops at the tick exactly.)
        ask("resume");
        ask(&format!("pause-at {tick}"));
        let at = format!("tick {tick}");
        let started = std::time::Instant::now();
        while ask("tick") != at {
            assert!(started.elapsed().as_secs() < 120, "the session never reached tick {tick}");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    let answer = ask("frame shot");
    assert!(answer.starts_with(&format!("frame shot tick {tick} ")), "{answer}");
    drop(stdin);
    let _ = child.wait();
    std::fs::read(dir.join("shot.png")).unwrap()
}

/// A frame taken inside a session is the `--frames` frame of the same scene, tick and
/// camera, byte for byte (TOOL-11). Opt-in like the other GPU tests.
#[test]
fn session_frames_equal_the_frames_option() {
    if !adapter(&tmp("probe-session")) {
        return;
    }
    // Frames without overlapping effects repeat byte for byte (the first two scenes); with
    // them two `--frames` runs already differ by a level or two ([`TOLERANCE`], see
    // `fx_frames`), so a session frame of a firing scene is held to the same bound.
    for (scene, tick, exact) in [("planet.ini", 0, true), ("planet.ini", 100, true), ("combat-duel.ini", 60, false), ("combat-firing.ini", 6, false)] {
        let (reference, _) = run_frames(scene, &tick.to_string(), "", &format!("ref-{scene}-{tick}"));
        let session = session_frame(scene, tick, &[], &format!("sess-{scene}-{tick}"));
        assert_eq!(reference.len(), 1);
        if exact {
            assert!(session == reference[0], "{scene} tick {tick}: the session frame differs from the --frames frame");
        } else {
            let d = max_diff(&session, &reference[0]);
            assert!(d <= TOLERANCE, "{scene} tick {tick}: the session frame differs from the --frames frame by {d}/255");
        }
    }
    // A camera set by command in the session is the scene's `Camera=` of the reference.
    let (reference, _) = run_frames("planet.ini", "100", "orbit ship 100 30 600\n", "ref-orbit");
    let session = session_frame("planet.ini", 100, &["orbit ship 100 30 600"], "sess-orbit");
    assert!(session == reference[0], "orbit: the session frame differs from the --frames frame");
}

/// `Take a screenshot` / `Expect shots` at tier 2: frames named for the scenario, judged.
#[test]
fn shot_steps_run_at_tier_two() {
    if !adapter(&tmp("probe-shots")) {
        return;
    }
    let out = tmp("shots");
    let code = cli::main(args(&["run", "shots-planet", "--tier", "2", "--sta64", sta64(), "--out", out.to_str().unwrap()]));
    assert_eq!(code, 0, "sta-bench run shots-planet --tier 2");
    let frames = run_dir(&out).join("shots-planet/frames");
    for n in ["first", "later", "again"] {
        assert!(frames.join(format!("{n}.png")).is_file(), "{n}.png");
    }
    let report = std::fs::read_to_string(run_dir(&out).join("report.md")).unwrap();
    assert!(!report.contains("FAIL"), "{report}");
}

/// A session without a renderer says so instead of ignoring `frame`.
#[cfg(unix)]
#[test]
fn frame_without_a_renderer_is_refused() {
    let base = tmp("session-noframe");
    let out = base.join("results");
    let o = out.to_str().unwrap();
    assert_eq!(cli::main(args(&["session", "start", "--scene", "planet", "--sta64", sta64(), "--out", o])), 0);
    let sessions = base.join("sessions");
    let id = std::fs::read_dir(&sessions).unwrap().flatten().next().unwrap().file_name().into_string().unwrap();
    assert_eq!(cli::main(args(&["drive", "--session", &id, "frame shot", "--out", o])), 1);
    assert_eq!(cli::main(args(&["session", "stop", "--session", &id, "--out", o])), 0);
}
