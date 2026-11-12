//! `pair <pair> --game a2|roots`: one scene description on the original game
//! (Refit's bench, through `tools/pair/refit_side.py`) and on STA64, compared
//! in sim seconds (see [`crate::pair`]; docs/TESTBENCH.md, "Paired scenes").
//!
//! Exit 0 when everything is within the pair's tolerances, 1 when something
//! is beyond them or a side failed, 2 on usage, 75 when the original-game
//! bench lock stays busy (BENCH-BUSY: run again). The report is written either way.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::pair::{self, PairDesc, PairTol};
use crate::run::RunConfig;

use super::tier2::sta64_path;
use super::{Fail, Opts, Outcome, config};

/// `sta64`'s exit code for "no GPU adapter".
const NO_ADAPTER: i32 = 77;

fn pair_path(o: &Opts, arg: &str) -> PathBuf {
    let p = Path::new(arg);
    if p.extension().is_some_and(|e| e == "ini") || p.exists() {
        return p.to_owned();
    }
    let cfg = config(o);
    cfg.scene_dir.parent().unwrap_or(Path::new(".")).join("pairs").join(format!("{arg}.ini"))
}

fn repo_root(o: &Opts) -> PathBuf {
    let cfg = config(o);
    cfg.scene_dir.parent().and_then(Path::parent).unwrap_or(Path::new(".")).to_owned()
}

/// Run `cmd`, feeding `stdin`, killed after `secs`. Returns (exit code, stdout, stderr).
fn spawn(cmd: &mut Command, stdin: &str, secs: u64) -> Result<(Option<i32>, String, String), String> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{:?}: {e}", cmd.get_program()))?;
    if let Some(mut i) = child.stdin.take() {
        let _ = i.write_all(stdin.as_bytes());
    }
    let (mut out, mut err) = (child.stdout.take().expect("piped"), child.stderr.take().expect("piped"));
    let to = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let te = std::thread::spawn(move || {
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
        std::thread::sleep(Duration::from_millis(50));
    };
    Ok((status.code(), to.join().unwrap_or_default(), te.join().unwrap_or_default()))
}

/// STA64's frames: a `sta64 --headless` run of the scene on the bench's fake
/// content (stand-in shapes until the renderers run without an install), the
/// same commands at the same ticks. Returns the frame files (relative to
/// `sta_dir`) and a note.
fn sta64_frames(o: &Opts, desc: &PairDesc, sched: &[pair::Timed], times: &[f64], end: u64, sta_dir: &Path) -> (Vec<Option<String>>, String) {
    let none = vec![None; times.len()];
    let ticks: Vec<u64> = times.iter().map(|t| pair::tick_of(*t)).collect();
    let scene = sta_dir.join("Scene.frames.ini");
    if let Err(e) = fs::write(&scene, desc.scene_text(&["fake"])) {
        return (none, e.to_string());
    }
    let fdir = sta_dir.join("frames");
    let _ = fs::remove_dir_all(&fdir);
    let mut script = String::new();
    for c in sched {
        script += &format!("at {} {}\n", pair::tick_of(c.t), c.text);
    }
    let bin = sta64_path(o);
    let list = ticks.iter().map(u64::to_string).collect::<Vec<_>>().join(",");
    let mut cmd = Command::new(&bin);
    cmd.args(["--headless", "--clock", "bench", "--control", "stdio", "--ticks"])
        .arg(end.max(ticks.iter().copied().max().unwrap_or(0)).to_string())
        .arg("--scene")
        .arg(&scene)
        .args(["--frames", &list, "--frame-dir"])
        .arg(&fdir)
        .args(["--frame-size", &format!("{}x{}", desc.size.0, desc.size.1)]);
    match spawn(&mut cmd, &script, o.timeout.max(120)) {
        Err(e) => (none, format!("sta64 ({}): {e}", bin.display())),
        Ok((Some(NO_ADAPTER), _, err)) => (none, format!("no GPU adapter: {}", err.trim())),
        Ok((Some(0), out, _)) => {
            let _ = fs::write(sta_dir.join("frames.log"), &out);
            let files: Vec<Option<String>> = ticks
                .iter()
                .map(|t| {
                    let rel = format!("frames/tick-{t:06}.png");
                    sta_dir.join(&rel).is_file().then_some(rel)
                })
                .collect();
            let missing = files.iter().filter(|f| f.is_none()).count();
            let note = format!(
                "sta64 --headless on the bench's fake content (stand-in shapes){}",
                if missing > 0 { format!("; {missing} frames missing") } else { String::new() }
            );
            (files, note)
        }
        Ok((code, _, err)) => {
            let tail: Vec<&str> = err.lines().rev().take(4).collect();
            (none, format!("sta64 exit {code:?}: {}", tail.into_iter().rev().collect::<Vec<_>>().join(" | ")))
        }
    }
}

pub(crate) fn pair(o: &Opts) -> Outcome {
    let usage = "pair <pair> --game a2|roots [--refit-out DIR] [--refit ROOT] [--install DIR] [--mod NAME] [--fake] [--no-frames] [--tol ...] [--out DIR]";
    let [arg] = o.rest.as_slice() else {
        return Err(Fail::Usage(usage.into()));
    };
    let game = o.game.clone().ok_or_else(|| Fail::Usage(format!("--game a2|roots ({usage})")))?;
    let path = pair_path(o, arg);
    let mut desc = PairDesc::load(&path, &game).map_err(Fail::Usage)?;
    if let Some(t) = &o.tol {
        desc.tol = PairTol::parse(t).map_err(|e| Fail::Usage(format!("--tol: {e}")))?;
    }
    let out = o.out.clone().unwrap_or_else(|| pair::default_root().join(format!("{}-{}", desc.name, desc.game)));
    if pair::inside_checkout(&out) {
        return Err(Fail::Usage(format!(
            "{}: inside a checkout; frames of the original game stay out of the repository (use the default, $XDG_DATA_HOME/sta64/bench/pairs/)",
            out.display()
        )));
    }
    fs::create_dir_all(&out).map_err(|e| Fail::Error(format!("{}: {e}", out.display())))?;

    // The original side.
    let refit_dir = match &o.refit_out {
        Some(d) => d.clone(),
        None => {
            let d = out.join("refit");
            let _ = fs::remove_dir_all(&d);
            let script = repo_root(o).join("tools/pair/refit_side.py");
            let mut cmd = Command::new("python3");
            cmd.arg(&script).arg("--pair").arg(&path).args(["--game", &desc.game]).arg("--out").arg(&d);
            if let Some(r) = &o.refit {
                cmd.arg("--refit").arg(r);
            }
            println!("original side: {} (one Refit bench session; minutes)", script.display());
            let status = cmd.status().map_err(|e| Fail::Error(format!("python3 {}: {e}", script.display())))?;
            if status.code() == Some(75) {
                eprintln!("sta-bench: BENCH-BUSY: another original-game session holds the bench lock; run again");
                return Ok(75);
            }
            if !status.success() {
                return Err(Fail::Error(format!("the original side failed ({status}); its files are in {}", d.display())));
            }
            d
        }
    };
    let meta = pair::read_meta(&refit_dir, "refit.json").map_err(Fail::Error)?;
    if meta.get("game").str().is_some_and(|g| g != desc.game) {
        return Err(Fail::Usage(format!("{}: a run of {}, not {}", refit_dir.display(), meta.get("game").str().unwrap_or("?"), desc.game)));
    }

    // STA64.
    let (cfg, content) = if o.fake {
        (RunConfig::fake(), "fake content (sta_bench RunConfig::fake)".to_owned())
    } else {
        let root = o.install.clone().or_else(|| std::env::var_os("STA64_TEST_INSTALL").map(PathBuf::from)).ok_or_else(|| {
            Fail::Usage("STA64's side needs the game's install: --install DIR (or STA64_TEST_INSTALL), or --fake".into())
        })?;
        let m = o.mod_name.clone().or(desc.mod_name.clone());
        let cfg = pair::install_config(&root, m.as_deref()).map_err(Fail::Error)?;
        let what = format!("install {} mod {} profile {:?}", root.display(), m.as_deref().unwrap_or("-"), cfg.content.profile());
        (cfg, what)
    };
    let sched = pair::schedule(&desc, Some(&meta));
    let end_t = meta.get("end_t").num().unwrap_or(desc.duration);
    let sta_dir = out.join("sta64");
    let _ = fs::remove_dir_all(&sta_dir);
    let scene = desc.scene_text(if o.fake { &["fake"] } else { &[] });
    let run = pair::run_sta64(&scene, Arc::new(cfg), &sched, end_t, &sta_dir).map_err(|e| Fail::Error(format!("STA64 side: {e}")))?;
    let times = pair::frame_times(&desc, Some(&meta));
    let (files, note) = if times.is_empty() {
        (Vec::new(), "no frames asked".to_owned())
    } else if o.no_frames {
        (vec![None; times.len()], "skipped (--no-frames)".to_owned())
    } else {
        sta64_frames(o, &desc, &sched, &times, run.ticks, &sta_dir)
    };
    let frames: Vec<(f64, u64, Option<String>)> = times.iter().zip(files).map(|(t, f)| (*t, pair::tick_of(*t), f)).collect();
    fs::write(sta_dir.join("run.json"), pair::sta64_run_json(&content, &run, &frames, &note)).map_err(|e| Fail::Error(e.to_string()))?;

    let rep = pair::compare(&desc, &refit_dir, &sta_dir, &out).map_err(Fail::Error)?;
    println!("{}", rep.md.lines().last().unwrap_or(""));
    println!("report: {} (and report.json)", out.join("report.md").display());
    Ok(i32::from(rep.out_of_tolerance > 0))
}
