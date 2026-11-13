//! `perf <map|scene.ini> --install <root> [--mod <name>]`: performance runs of the install's
//! map view, one headless `sta64 --perf` process per profile and size, and one table
//! (docs/TESTBENCH.md, "Performance runs"; the measuring is `sta64`'s `bench/perf.rs`).
//!
//! The scene is the named `.ini`, or generated from a map name (`Map=`, fog on, no HUD) with
//! `--camera "orbit x y z yaw pitch distance"`. Results go to
//! `$XDG_DATA_HOME/sta64/bench/perf/<label>/` (never into a checkout): one `<profile>-<WxH>.json`
//! per run, `results.json` and `results.md` over all of them.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::tier2::sta64_path;
use super::{Fail, Opts, Outcome};
use crate::pair;

/// `sta64`'s exit code for "no GPU adapter".
const NO_ADAPTER: i32 = 77;

/// The variants run when `--variants` is not given: the attribution set.
pub const DEFAULT_VARIANTS: &str = "base,no-rocks,no-planets,no-nebula,no-sky,no-shadows,no-fog,std-hulls,msaa-off,msaa-2,no-prepass,freeze-rocks,freeze-hulls";

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct Stat {
    pub mean: f64,
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
    pub min: f64,
}

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct Counts {
    pub entities: u64,
    pub visible_meshes: u64,
    pub rocks_visible: u64,
    pub shadow_pass_active: bool,
}

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct SysCost {
    pub ms_per_frame: f64,
    pub calls_per_frame: f64,
}

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct Rep {
    pub frame_ms: Stat,
    pub main_cpu_ms: Stat,
    pub gpu_wait_ms: Stat,
    pub gpu_pass_ms: BTreeMap<String, f64>,
    pub gpu_top_level_total_ms: f64,
    pub systems: BTreeMap<String, SysCost>,
    pub counts: Counts,
}

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct Variant {
    pub name: String,
    pub median_of_means_ms: f64,
    pub min_of_means_ms: f64,
    pub max_of_means_ms: f64,
    pub fps_median: f64,
    pub reps: Vec<Rep>,
}

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct Report {
    pub adapter: String,
    pub elapsed_s: f64,
    pub frames_per_round: u64,
    pub reps: u64,
    pub camera: String,
    pub gpu_synced_each_frame: bool,
    pub timestamp_queries_available: bool,
    pub load_avg_start: String,
    pub load_avg_end: String,
    pub competing_processes_start: BTreeMap<String, u64>,
    pub competing_processes_end: BTreeMap<String, u64>,
    pub scenery: Vec<String>,
    pub variants: Vec<Variant>,
}

/// One run of one process: profile, size, and what it wrote.
pub struct Run {
    pub profile: String,
    pub size: String,
    pub report: Report,
}

fn out_root() -> PathBuf {
    pair::default_root().with_file_name("perf")
}

fn scene_text(map: &str, camera: Option<&str>) -> String {
    let mut s = format!("[Scene]\nMap={map}\nMapObjects=keep\nFog=1\nHud=0\nGrid=0\nCursor=0\nSeed=7\n");
    if let Some(c) = camera {
        let _ = writeln!(s, "Camera={c}");
    }
    s
}

pub(crate) fn perf(o: &Opts) -> Outcome {
    let usage = "perf <map|scene.ini> --install ROOT [--mod NAME] [--camera \"orbit x y z yaw pitch dist\"] [--profile refit,stock] [--sizes 1920x1080,3440x1440] [--measure N] [--warmup N] [--reps N] [--variants a,b+c] [--perf-camera still|sweep] [--systems] [--label NAME] [--out DIR]";
    let [what] = o.rest.as_slice() else { return Err(Fail::Usage(usage.into())) };
    let install = o.install.clone().ok_or_else(|| Fail::Usage(format!("--install ({usage})")))?;
    let label = o.label.clone().unwrap_or_else(|| {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        format!("{}-{stamp}", Path::new(what).file_stem().map_or_else(|| "run".into(), |s| s.to_string_lossy().into_owned()))
    });
    let out = o.out.clone().unwrap_or_else(|| out_root().join(&label));
    if pair::inside_checkout(&out) {
        return Err(Fail::Usage(format!("{}: inside a checkout; performance results stay outside it (default: $XDG_DATA_HOME/sta64/bench/perf/)", out.display())));
    }
    fs::create_dir_all(&out).map_err(|e| Fail::Error(format!("{}: {e}", out.display())))?;
    let scene = if what.ends_with(".ini") {
        PathBuf::from(what)
    } else {
        let p = out.join("Scene.ini");
        fs::write(&p, scene_text(what, o.camera.as_deref())).map_err(|e| Fail::Error(e.to_string()))?;
        p
    };
    let bin = sta64_path(o);
    let profiles: Vec<String> = o.profile.as_deref().unwrap_or("refit,stock").split(',').map(|s| s.trim().to_owned()).collect();
    let sizes: Vec<String> = o.sizes.as_deref().unwrap_or("1920x1080,3440x1440").split(',').map(|s| s.trim().to_owned()).collect();
    for s in &sizes {
        if s.split_once('x').is_none_or(|(w, h)| w.parse::<u32>().is_err() || h.parse::<u32>().is_err()) {
            return Err(Fail::Usage(format!("--sizes: WxH, not `{s}`")));
        }
    }
    let mut runs = Vec::new();
    for profile in &profiles {
        for size in &sizes {
            let json = out.join(format!("{profile}-{size}.json"));
            let _ = fs::remove_file(&json);
            let mut args: Vec<String> = vec!["--headless".into(), "--clock".into(), "bench".into(), "--scene".into(), scene.display().to_string()];
            args.extend(["--install-root".into(), install.display().to_string()]);
            if let Some(m) = &o.mod_name {
                args.extend(["--mod".into(), m.clone()]);
            }
            args.extend(["--install-content".into(), "--frame-size".into(), size.clone(), "--perf".into(), json.display().to_string()]);
            args.extend(["--perf-frames".into(), o.measure.unwrap_or(300).to_string(), "--perf-warmup".into(), o.warmup.unwrap_or(90).to_string()]);
            args.extend(["--perf-reps".into(), o.reps.unwrap_or(3).to_string(), "--perf-variants".into(), o.variants.clone().unwrap_or_else(|| DEFAULT_VARIANTS.into())]);
            args.extend(["--perf-camera".into(), o.perf_camera.clone().unwrap_or_else(|| "still".into())]);
            if o.systems {
                args.push("--perf-systems".into());
            }
            println!("perf: {profile} {size} ...");
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "renice -n 10 -p $$ >/dev/null 2>&1; exec \"$0\" \"$@\""]).arg(&bin).args(&args);
            // Headless means headless: no display server of the user's session, no sound device.
            cmd.env_remove("WAYLAND_DISPLAY").env_remove("DISPLAY").env("STA64_RENDER_PROFILE", profile);
            let (code, stdout, stderr) = run_child(&mut cmd, o.timeout.max(1800)).map_err(Fail::Error)?;
            let _ = fs::write(out.join(format!("{profile}-{size}.log")), format!("{stdout}\n--- stderr\n{stderr}"));
            match code {
                Some(0) => {}
                Some(NO_ADAPTER) => {
                    eprintln!("sta-bench: no GPU adapter: {}", stderr.trim());
                    return Ok(77);
                }
                other => {
                    let tail: Vec<&str> = stderr.lines().rev().take(6).collect();
                    return Err(Fail::Error(format!("sta64 exit {other:?} ({profile} {size}): {}", tail.into_iter().rev().collect::<Vec<_>>().join(" | "))));
                }
            }
            let text = fs::read_to_string(&json).map_err(|e| Fail::Error(format!("{}: {e}", json.display())))?;
            let report: Report = json5::from_str(&text).map_err(|e| Fail::Error(format!("{}: {e}", json.display())))?;
            runs.push(Run { profile: profile.clone(), size: size.clone(), report });
        }
    }
    let md = markdown(what, &install, o.mod_name.as_deref(), &runs);
    let _ = fs::write(out.join("results.md"), &md);
    print!("{md}");
    println!("perf: results in {}", out.display());
    Ok(0)
}

fn run_child(cmd: &mut Command, secs: u64) -> Result<(Option<i32>, String, String), String> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
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

fn procs(m: &BTreeMap<String, u64>) -> String {
    if m.is_empty() {
        "none".into()
    } else {
        m.iter().map(|(k, v)| format!("{k} x{v}")).collect::<Vec<_>>().join(", ")
    }
}

/// The Markdown over all runs.
pub fn markdown(what: &str, install: &Path, mod_name: Option<&str>, runs: &[Run]) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# Performance run: {what}\n");
    let _ = writeln!(s, "Install `{}`{}. Frame time is the wall time of one frame with the GPU waited for; `spread` is the min-max of the rounds' means (the noise to read the differences against).\n", install.display(), mod_name.map(|m| format!(", mod `{m}`")).unwrap_or_default());
    for r in runs {
        let rep = &r.report;
        let _ = writeln!(s, "## {} profile, {}\n", r.profile, r.size);
        let _ = writeln!(
            s,
            "Adapter {}; GPU timestamps {}; {} rounds of {} frames per variant; camera {}; load average {} at the start, {} at the end; competing processes: {} / {}.\n",
            rep.adapter,
            if rep.timestamp_queries_available { "available" } else { "NOT available (GPU per pass is empty)" },
            rep.reps,
            rep.frames_per_round,
            rep.camera,
            rep.load_avg_start,
            rep.load_avg_end,
            procs(&rep.competing_processes_start),
            procs(&rep.competing_processes_end),
        );
        if !rep.scenery.is_empty() {
            let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
            for l in &rep.scenery {
                *kinds.entry(l.split(" at ").next().unwrap_or(l)).or_default() += 1;
            }
            let _ = writeln!(s, "Map: {}.\n", kinds.iter().map(|(k, v)| format!("{v} {k}")).collect::<Vec<_>>().join(", "));
        }
        let base = rep.variants.first();
        let base_ms = base.map_or(0.0, |b| b.median_of_means_ms);
        let _ = writeln!(s, "| variant | frame ms (median) | fps | spread ms | vs base ms | main CPU ms | GPU wait ms | GPU passes ms | visible meshes |");
        let _ = writeln!(s, "|---|---:|---:|---:|---:|---:|---:|---:|---:|");
        for v in &rep.variants {
            let mid = v.reps.get(v.reps.len() / 2);
            let _ = writeln!(
                s,
                "| {} | {:.2} | {:.1} | {:.2}-{:.2} | {:+.2} | {:.2} | {:.2} | {:.2} | {} |",
                v.name,
                v.median_of_means_ms,
                v.fps_median,
                v.min_of_means_ms,
                v.max_of_means_ms,
                v.median_of_means_ms - base_ms,
                mid.map_or(0.0, |m| m.main_cpu_ms.mean),
                mid.map_or(0.0, |m| m.gpu_wait_ms.mean),
                mid.map_or(0.0, |m| m.gpu_top_level_total_ms),
                mid.map_or(0, |m| m.counts.visible_meshes),
            );
        }
        if let Some(b) = base.and_then(|b| b.reps.first()) {
            let mut passes: Vec<_> = b.gpu_pass_ms.iter().collect();
            passes.sort_by(|a, b| b.1.total_cmp(a.1));
            if !passes.is_empty() {
                let _ = writeln!(s, "\nGPU time per pass, base (ms): {}\n", passes.iter().take(14).map(|(k, v)| format!("`{k}` {v:.2}")).collect::<Vec<_>>().join(", "));
            }
            let mut sys: Vec<_> = b.systems.iter().collect();
            sys.sort_by(|a, b| b.1.ms_per_frame.total_cmp(&a.1.ms_per_frame));
            if !sys.is_empty() {
                let _ = writeln!(s, "\nCPU per system, base (ms per frame, calls per frame):\n");
                for (k, v) in sys.iter().take(15) {
                    let _ = writeln!(s, "- `{k}`: {:.3} ms, {:.1} calls", v.ms_per_frame, v.calls_per_frame);
                }
            }
            let _ = writeln!(s, "\nCounts (base): {} entities, {} visible meshes ({} rocks), shadow pass {}.\n", b.counts.entities, b.counts.visible_meshes, b.counts.rocks_visible, if b.counts.shadow_pass_active { "on" } else { "off" });
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_reads_back_and_the_table_names_each_variant() {
        let json = r#"{"adapter":"x (Vulkan)","reps":2,"frames_per_round":10,"timestamp_queries_available":true,
          "load_avg_start":"1 2 3","competing_processes_start":{"sta64":1},"scenery":["asteroid field at 0 0 0"],
          "variants":[{"name":"base","median_of_means_ms":20.0,"min_of_means_ms":19.0,"max_of_means_ms":21.0,"fps_median":50.0,
            "reps":[{"frame_ms":{"mean":20.0},"main_cpu_ms":{"mean":5.0},"gpu_pass_ms":{"main_opaque_pass_3d":3.0},"gpu_top_level_total_ms":3.0,
              "systems":{"system: foo":{"ms_per_frame":1.5,"calls_per_frame":1.0}},"counts":{"entities":9,"visible_meshes":4}}]},
            {"name":"no-rocks","median_of_means_ms":15.0,"min_of_means_ms":14.0,"max_of_means_ms":16.0,"fps_median":66.0,"reps":[]}]}"#;
        let report: Report = json5::from_str(json).expect("parses");
        let md = markdown("m", Path::new("/g"), None, &[Run { profile: "refit".into(), size: "3440x1440".into(), report }]);
        assert!(md.contains("## refit profile, 3440x1440"));
        assert!(md.contains("| no-rocks | 15.00 | 66.0 | 14.00-16.00 | -5.00 |"), "{md}");
        assert!(md.contains("`main_opaque_pass_3d` 3.00"));
        assert!(md.contains("1 asteroid field"));
    }

    #[test]
    fn the_generated_scene_names_the_map_and_camera() {
        let t = scene_text("4alliance", Some("orbit 0 0 0 30 40 900"));
        assert!(t.contains("Map=4alliance") && t.contains("Fog=1") && t.contains("Camera=orbit 0 0 0 30 40 900"));
        assert!(crate::parse::SceneFile::parse(&t).is_ok());
    }
}
