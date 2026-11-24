//! Frame identity on real content (ADR 0009, S6 and implementation step 6): the install's own map
//! view, drawn headless and offscreen, with sprite nodes on the shared look materials (the game's
//! path) against the same run with sprite nodes on their own `StandardMaterial`s (the reference path,
//! `STA64_SPRITE_REFERENCE=1`, which turns adoption off as the GPU test's reference does), under
//! `STA64_RENDER_PROFILE=stock` and `refit`. Frames are compared byte for byte, no tolerance.
//!
//! The control is the reference path against itself. On real content it does not always repeat: a
//! few things outside the sprite nodes still vary run to run (some views of the Refit volumetric
//! nebula, a few pixels at one level), and a frame then comes out as one of a small number of
//! different frames. So the test makes `STA64_TEST_SPRITE_RUNS` (default 3) runs of each path,
//! interleaved, and a shared-material frame passes when it is byte-equal to the frame of the same
//! tick from one of the reference runs. It prints, per tick, how many different frames the
//! reference gave and where they differ. A tick whose reference frame never repeats proves nothing
//! and fails.
//!
//! The scenes are the player's: `STA64_TEST_SPRITE_SCENES` names a directory outside the repository
//! with bench scene files (`<name>.ini`, read against the install), each with optional
//! `<name>.cmd` (command lines sent on the session's stdin before it runs, such as
//! `at 30 cloak <object> on` or timed `camera` lines) and `<name>.ticks` (the frame ticks, comma
//! separated). Cover a cloaking unit and a nebula seen from inside (the issue's *done when*).
//!
//! Opt-in: prints `SKIP` and passes unless `STA64_GPU_TESTS=1`, `STA64_TEST_INSTALL` (and optionally
//! `STA64_TEST_MOD`) and `STA64_TEST_SPRITE_SCENES` are set; `sta64` exits 77 without a hardware
//! adapter and the test skips. `STA64_TEST_SPRITE_SIZE` (`WxH`, default 960x540) sets the frame
//! size; `STA64_TEST_SPRITE_PROFILES` (default `stock,refit`) the render profiles. Frames stay in a temporary directory (kept with `STA64_KEEP_FRAMES=1`, printed), never in
//! the repository.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Frames start late: in the first seconds a frame can differ run to run, by when the map's surface
/// bakes (planets, moons) finish on their threads, which the control run shows as a difference of
/// the harness, not of the path.
const DEFAULT_TICKS: &str = "200,220,240,260,280,300,330,360,400";
/// What the bench prints when the reference path is on (`bench::pair_view`).
const REFERENCE_LINE: &str = "sprite nodes: per-node materials";

struct Scene {
    name: String,
    ini: PathBuf,
    commands: String,
    ticks: Vec<u32>,
}

fn scenes(dir: &Path) -> Vec<Scene> {
    let mut inis: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "ini"))
        .collect();
    inis.sort();
    inis.into_iter()
        .map(|ini| {
            let side = |ext: &str| std::fs::read_to_string(ini.with_extension(ext)).ok();
            let ticks = side("ticks").unwrap_or_else(|| DEFAULT_TICKS.into());
            let ticks = ticks.split(',').map(|t| t.trim().parse::<u32>().unwrap_or_else(|_| panic!("{}: tick {t:?}", ini.display()))).collect();
            Scene { name: ini.file_stem().unwrap().to_string_lossy().into_owned(), commands: side("cmd").unwrap_or_default(), ticks, ini }
        })
        .collect()
}

/// One run's frames (RGBA, by tick), or `None` without an adapter.
fn frames(scene: &Scene, install: &str, active_mod: Option<&str>, profile: &str, reference: bool, size: (u32, u32), out: &Path) -> Option<Vec<Vec<u8>>> {
    let _ = std::fs::remove_dir_all(out);
    std::fs::create_dir_all(out).unwrap();
    let ticks = scene.ticks.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    let mut c = Command::new(env!("CARGO_BIN_EXE_sta64"));
    c.args(["--headless", "--clock", "bench", "--control", "stdio", "--install-content", "--frames", &ticks])
        .args(["--frame-size", &format!("{}x{}", size.0, size.1), "--frame-dir"])
        .arg(out)
        .arg("--scene")
        .arg(&scene.ini)
        .arg("--install-root")
        .arg(install)
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .env("STA64_RENDER_PROFILE", profile)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if reference {
        c.env("STA64_SPRITE_REFERENCE", "1");
    } else {
        c.env_remove("STA64_SPRITE_REFERENCE");
    }
    if let Some(m) = active_mod {
        c.args(["--mod", m]);
    }
    let mut child = c.spawn().unwrap();
    // All commands go in before the first tick runs (the bench holds tick 0 for its warm-up
    // frames); closing stdin lets the run end after its last frame.
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(scene.commands.as_bytes()).unwrap();
    drop(stdin);
    let run = child.wait_with_output().unwrap();
    let log = format!("{}{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr));
    match run.status.code() {
        Some(0) => {}
        Some(77) => {
            eprintln!("SKIP: no GPU adapter for headless frames");
            return None;
        }
        code => panic!("{} {profile}: headless frame run failed ({code:?}):\n{log}", scene.name),
    }
    assert!(!log.contains("ERROR"), "{} {profile}: shader or pipeline error:\n{log}", scene.name);
    // A refused command line (`! ...`) is worth seeing, but the scene's own switches can answer
    // `! unsupported` too, so it is printed, not failed.
    for l in log.lines().filter(|l| l.starts_with('!')) {
        eprintln!("{} {profile}: {l}", scene.name);
    }
    // The switch really took (a mistyped variable would compare the shared path with itself).
    assert_eq!(log.contains(REFERENCE_LINE), reference, "{} {profile}: reference path {}", scene.name, if reference { "not on" } else { "on" });
    Some(scene.ticks.iter().map(|t| rgba(&std::fs::read(out.join(format!("tick-{t:06}.png"))).expect("a frame"), size)).collect())
}

fn rgba(png: &[u8], size: (u32, u32)) -> Vec<u8> {
    let mut r = png::Decoder::new(std::io::Cursor::new(png)).read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    assert_eq!((info.width, info.height), size);
    buf.truncate(info.buffer_size());
    buf
}

/// How two frames differ: pixels, their box, the largest channel difference.
fn describe(a: &[u8], b: &[u8], width: u32) -> String {
    let w = width as usize;
    let n = a.len() / (w * 4).max(1);
    let px = n.max(1) * w;
    let c = a.len() / px;
    let (mut count, mut most, mut x0, mut x1, mut y0, mut y1) = (0, 0u8, usize::MAX, 0, usize::MAX, 0);
    for (i, (p, q)) in a.chunks(c).zip(b.chunks(c)).enumerate() {
        if p != q {
            count += 1;
            most = most.max(p.iter().zip(q).map(|(u, v)| u.abs_diff(*v)).max().unwrap_or(0));
            let (x, y) = (i % w, i / w);
            (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
        }
    }
    format!("{count} pixels in x {x0}..={x1}, y {y0}..={y1}, largest channel difference {most}")
}

#[test]
fn real_content_frames_are_byte_equal_through_the_shared_sprite_materials() {
    if std::env::var("STA64_GPU_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set STA64_GPU_TESTS=1 to run the GPU frame tests");
        return;
    }
    let Ok(install) = std::env::var("STA64_TEST_INSTALL") else {
        eprintln!("SKIP: STA64_TEST_INSTALL names no install");
        return;
    };
    let Ok(dir) = std::env::var("STA64_TEST_SPRITE_SCENES") else {
        eprintln!("SKIP: STA64_TEST_SPRITE_SCENES names no directory of scenes");
        return;
    };
    let active_mod = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
    let size = std::env::var("STA64_TEST_SPRITE_SIZE")
        .ok()
        .and_then(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))))
        .unwrap_or((960, 540));
    let profiles = std::env::var("STA64_TEST_SPRITE_PROFILES").unwrap_or_else(|_| "stock,refit".into());
    let profiles: Vec<&str> = profiles.split(',').map(str::trim).filter(|p| !p.is_empty()).collect();
    let list = scenes(Path::new(&dir));
    assert!(!list.is_empty(), "{dir}: no scene (*.ini)");
    let runs = std::env::var("STA64_TEST_SPRITE_RUNS").ok().and_then(|n| n.parse::<usize>().ok()).unwrap_or(3).max(2);
    let root = std::env::temp_dir().join(format!("sta64-sprite-identity-{}", std::process::id()));
    let mut failures = Vec::new();
    let mut summary = Vec::new();
    for scene in &list {
        for &profile in &profiles {
            let run = |tag: String, reference: bool| frames(scene, &install, active_mod.as_deref(), profile, reference, size, &root.join(format!("{}-{profile}-{tag}", scene.name)));
            // Interleaved, so a load that changes while the test runs touches both paths alike.
            let (mut refs, mut shared) = (Vec::new(), Vec::new());
            for i in 0..runs {
                let Some(r) = run(format!("reference-{i}"), true) else { return };
                let Some(n) = run(format!("shared-{i}"), false) else { return };
                refs.push(r);
                shared.push(n);
            }
            let lit = |f: &Vec<u8>| f.chunks(4).filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8).count();
            for (i, f) in refs[0].iter().enumerate() {
                assert!(lit(f) > 1000, "{} {profile} tick {}: the frame shows almost nothing ({} lit pixels)", scene.name, scene.ticks[i], lit(f));
            }
            // Per tick: the different frames the reference path gives on its own (the control), and
            // whether each shared run's frame is byte-equal to one of them.
            let (mut noisy, mut unrepeated, mut unmatched) = (0, 0, 0);
            for (i, &tick) in scene.ticks.iter().enumerate() {
                let mut outcomes: Vec<&Vec<u8>> = Vec::new();
                for r in &refs {
                    if !outcomes.contains(&&r[i]) {
                        outcomes.push(&r[i]);
                    }
                }
                if outcomes.len() > 1 {
                    noisy += 1;
                    eprintln!("{} {profile} tick {tick}: {} different reference frames in {runs} runs (the harness, not the path): {}", scene.name, outcomes.len(), describe(outcomes[0], outcomes[1], size.0));
                }
                if outcomes.len() == runs {
                    unrepeated += 1;
                    failures.push(format!("{} {profile} tick {tick}: the reference path never repeated this frame in {runs} runs, so nothing is proven here", scene.name));
                }
                for (k, n) in shared.iter().enumerate() {
                    if outcomes.contains(&&n[i]) {
                        continue;
                    }
                    unmatched += 1;
                    let near = outcomes.iter().min_by_key(|a| a.iter().zip(&n[i]).filter(|(p, q)| p != q).count()).unwrap();
                    failures.push(format!("{} {profile} shared run {k} tick {tick}: equal to no reference frame; nearest: {}", scene.name, describe(near, &n[i], size.0)));
                }
            }
            let n = scene.ticks.len();
            let line = format!(
                "{} {profile}: control: {noisy} of {n} ticks gave more than one reference frame in {runs} runs ({unrepeated} never repeated); shared path {} ({unmatched} of {} shared frames equal to no reference frame)",
                scene.name,
                if unmatched == 0 && unrepeated == 0 { "pass" } else { "FAIL" },
                n * runs,
            );
            eprintln!("{line}");
            summary.push(line);
        }
    }
    if std::env::var("STA64_KEEP_FRAMES").as_deref() == Ok("1") {
        eprintln!("frames kept in {}", root.display());
    } else {
        let _ = std::fs::remove_dir_all(&root);
    }
    assert!(failures.is_empty(), "{}\n\n{}", summary.join("\n"), failures.join("\n"));
}
