//! Tier 2: the scenario's scene in a `sta64 --headless --clock bench` process,
//! with frames (docs/TESTBENCH.md, "Two tiers").
//!
//! The in-process run has already happened, so its tick count and final digest
//! are known: the process runs the same scene for the same ticks and must end
//! on the same digest, and writes frames at fixed ticks. `--repeat N` runs it
//! N times and requires the frames to be byte-identical.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::{Opts, step};
use crate::parse::Scenario;
use crate::run::{CaseResult, RunConfig, Status};

/// `sta64`'s exit code for "no GPU adapter" (`sta64/src/bench/frames.rs`).
const NO_ADAPTER: i32 = 77;

/// The scene file text a scenario names, its `Key=Value` words applied to `[Scene]`.
pub(crate) fn materialize_scene(scene_dir: &Path, name: &str, overrides: &[(String, String)]) -> Result<String, String> {
    let path = scene_dir.join(format!("{name}.ini"));
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let header = lines.iter().position(|l| l.trim().eq_ignore_ascii_case("[scene]"));
    let header = match header {
        Some(h) => h,
        None => {
            lines.insert(0, "[Scene]".into());
            0
        }
    };
    for (k, v) in overrides {
        let end = lines[header + 1..].iter().position(|l| l.trim_start().starts_with('[')).map_or(lines.len(), |p| header + 1 + p);
        let at = (header + 1..end).find(|&i| lines[i].split('=').next().is_some_and(|key| key.trim().eq_ignore_ascii_case(k)));
        match at {
            Some(i) => lines[i] = format!("{k}={v}"),
            None => lines.insert(header + 1, format!("{k}={v}")),
        }
    }
    Ok(lines.join("\n") + "\n")
}

/// Where `sta64` is: `--sta64`, `$STA64_BIN`, beside this executable, `PATH`.
pub(crate) fn sta64_path(o: &Opts) -> PathBuf {
    if let Some(p) = &o.sta64 {
        return p.clone();
    }
    if let Some(p) = std::env::var_os("STA64_BIN") {
        return p.into();
    }
    let exe = if cfg!(windows) { "sta64.exe" } else { "sta64" };
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_owned)) {
        for d in [Some(dir.clone()), dir.parent().map(Path::to_owned)].into_iter().flatten() {
            if d.join(exe).is_file() {
                return d.join(exe);
            }
        }
    }
    exe.into()
}

struct Ran {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn spawn(bin: &Path, args: &[String], secs: u64) -> Result<Ran, String> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", bin.display()))?;
    // `--control stdio` with nothing to say: close it, the run ends at its tick budget.
    drop(child.stdin.take());
    let (mut out, mut err) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
    let t_out = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let t_err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        if start.elapsed() > Duration::from_secs(secs) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("timed out after {secs} s"));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    Ok(Ran { code: status.code(), stdout: t_out.join().unwrap_or_default(), stderr: t_err.join().unwrap_or_default() })
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    v.sort();
    v
}

/// Append the tier-2 half of `case`: the process run, its digest against the
/// in-process one, and the frames.
pub(crate) fn run_case(cfg: &RunConfig, sc: &Scenario, case: &mut CaseResult, o: &Opts, staged: &Path) {
    let Some(r) = &sc.headers.scene else {
        case.passed = false;
        case.steps.push(step(Status::Fail, "tier 2", "the scenario names no Scene"));
        return;
    };
    let case_dir = staged.join(&case.name);
    let _ = fs::create_dir_all(&case_dir);
    let scene = match materialize_scene(&cfg.scene_dir, &r.name, &r.overrides) {
        Ok(t) => t,
        Err(e) => {
            case.passed = false;
            case.steps.push(step(Status::Fail, "tier 2", e));
            return;
        }
    };
    let scene_path = case_dir.join("scene.ini");
    let _ = fs::write(&scene_path, scene);

    let ticks = case.ticks;
    let frames = o.frames.clone().unwrap_or_else(|| if ticks == 0 { vec![0] } else { vec![0, ticks] });
    let list = frames.iter().map(u64::to_string).collect::<Vec<_>>().join(",");
    let bin = sta64_path(o);
    let want = case.digests.last().map(|(_, d)| format!("digest {d:016x}"));
    let size = sc.headers.size.map(|(w, h)| format!("{w}x{h}")).or(o.frame_size.clone());
    let mut hashes: Vec<Vec<String>> = Vec::new();

    for run in 0..o.repeat.max(1) {
        let fdir = case_dir.join(if run == 0 { "frames".to_owned() } else { format!("frames.r{run}") });
        let _ = fs::remove_dir_all(&fdir);
        let mut args: Vec<String> = ["--headless", "--clock", "bench", "--control", "stdio", "--ticks"].map(String::from).into();
        args.extend([ticks.to_string(), "--scene".into(), scene_path.display().to_string()]);
        args.extend(["--frames".into(), list.clone(), "--frame-dir".into(), fdir.display().to_string()]);
        if let Some(s) = &size {
            args.extend(["--frame-size".into(), s.clone()]);
        }
        let label = format!("tier 2: sta64 headless run {}", run + 1);
        let ran = match spawn(&bin, &args, o.timeout) {
            Ok(r) => r,
            Err(e) => {
                case.passed = false;
                case.steps.push(step(Status::Fail, &label, e));
                return;
            }
        };
        case.log.extend(ran.stdout.lines().map(|l| format!("sta64: {l}")));
        if ran.code == Some(NO_ADAPTER) {
            case.steps.push(step(Status::Skip, &label, format!("no GPU adapter, frames skipped ({})", ran.stderr.trim())));
            println!("note: {}: no GPU adapter here, frames skipped", case.name);
            return;
        }
        if ran.code != Some(0) {
            case.passed = false;
            let tail: Vec<&str> = ran.stderr.lines().rev().take(5).collect();
            case.steps.push(step(Status::Fail, &label, format!("exit {:?}: {}", ran.code, tail.into_iter().rev().collect::<Vec<_>>().join(" | "))));
            return;
        }
        let last = ran.stdout.lines().last().unwrap_or("");
        match &want {
            Some(w) if !last.contains(w.as_str()) => {
                case.passed = false;
                case.steps.push(step(Status::Fail, &label, format!("process ended `{last}`, in-process run `{w}`")));
                return;
            }
            _ => case.steps.push(step(Status::Pass, &label, last.to_owned())),
        }
        let pngs: Vec<PathBuf> = files(&fdir).into_iter().filter(|p| p.extension().is_some_and(|e| e == "png")).collect();
        if pngs.len() != frames.len() {
            case.passed = false;
            case.steps.push(step(Status::Fail, &label, format!("{} frames asked, {} written", frames.len(), pngs.len())));
            return;
        }
        hashes.push(ran.stdout.lines().filter(|l| l.starts_with("frame tick ")).map(str::to_owned).collect());
        if run > 0 {
            let same = files(&case_dir.join("frames")).iter().zip(&pngs).all(|(a, b)| fs::read(a).ok() == fs::read(b).ok());
            let text = format!("frames of run 1 and run {}", run + 1);
            if same {
                case.steps.push(step(Status::Pass, &text, "byte-identical"));
                let _ = fs::remove_dir_all(&fdir);
            } else {
                case.passed = false;
                case.steps.push(step(Status::Fail, &text, format!("differ: {:?} vs {:?}", hashes[0], hashes[run as usize])));
            }
        }
    }
    if let Some(h) = hashes.first() {
        case.artifacts.insert("frames.txt".into(), h.join("\n") + "\n");
    }
}

/// Move the staged frames of each tier-2 case into the report directory.
pub(crate) fn adopt_frames(staged: &Path, report: &Path, cases: &[CaseResult]) {
    for c in cases {
        let from = staged.join(&c.name);
        for d in files(&from) {
            if d.is_dir() && d.file_name().is_some_and(|n| n.to_string_lossy().starts_with("frames")) {
                let _ = fs::rename(&d, report.join(&c.name).join(d.file_name().unwrap()));
            }
        }
        let _ = fs::copy(from.join("scene.ini"), report.join(&c.name).join("scene.ini"));
    }
}
