//! The `sta-bench` command line (docs/TESTBENCH.md, "Architecture").
//!
//! ```text
//! sta-bench list [scenarios|scenes]
//! sta-bench run <scenario...> [--tier 2] [--frames 0,100] [--repeat 2]
//! sta-bench check
//! sta-bench session start --scene <name> [Key=Value...] | stop | list
//! sta-bench drive --session ID "<command>"...
//! sta-bench diff <report-a> <report-b>
//! sta-bench parity <refit-out> <ours-out> [--tol time=0.1,pos=1,hull=1] | --names
//! sta-bench capture list | check <folder|latest> [--install <root> [--mod <name>]] [--fake]
//! ```
//!
//! The logic is a library so `sta64`'s own tests can drive it with the binary
//! they were built with; `src/bin/sta-bench.rs` is a shell around [`main`].

mod capture;
mod diff;
mod parity;
mod session;
mod tier2;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::run::{
    self, CaseResult, RunConfig, Status, StepOutcome, default_report_dir, discover, load_scenario,
    tier, write_report,
};

pub const USAGE: &str = "\
Usage: sta-bench <command> [options]

  list [scenarios|scenes]          what there is to run
  run <scenario...>                run scenarios (a name in bench/scenarios, or a path)
      --tier 2                     run in a headless sta64 process (frames), after the in-process run
      --frames 0,100               ticks to save frames at (default: 0 and the last tick)
      --frame-size WxH             frame size
      --repeat N                   run the process N times and require identical frames
  check                            run every tier-1 scenario in-process; exit 1 on a failure
  session start --scene <name> [Key=Value...]   start a headless sta64 and print its ID
  session list | session stop --session ID
  drive --session ID \"<command>\"...  send bench commands to a session, print the answers
  capture list                     playtest captures, newest first (default: $XDG_DATA_HOME/sta64/captures)
  capture check <folder|latest>    replay a capture headless to its tick; exit 1 when the digest differs
      --install <root> --mod <name>  mount this install (default: the one the capture recorded)
      --fake                       the capture is of the bench's fake content
  diff <report-a> <report-b>       compare report directories; exit 1 when they differ
  parity <refit-out> <ours-out>    Refit's Scene.marks/Scene.objects.jsonl against our marks/objects.jsonl
                                   (run directories, or their logs/): objects by name, marks in sim seconds
      --tol time=0.1,pos=1,hull=1  tolerances (seconds, world units, points)
      --names                      print the Refit-to-ours mark-name table

Common options:
  --bench-dir <dir>                directory holding scenes/ and scenarios/ (default: the repository's bench/)
  --out <dir>                      report root (default: $XDG_DATA_HOME/sta64/bench/results)
  --no-report                      do not write a report directory
  --sta64 <path>                   sta64 binary (default: $STA64_BIN, next to sta-bench, then PATH)
  --session <id>                   session for drive and stop (default: $STA64_BENCH_SESSION)
  --timeout <secs>                 per command or process (default 60)
";

#[derive(Default)]
pub(crate) struct Opts {
    pub bench_dir: Option<PathBuf>,
    pub out: Option<PathBuf>,
    pub no_report: bool,
    pub sta64: Option<PathBuf>,
    pub session: Option<String>,
    pub scene: Option<String>,
    pub tier: Option<u8>,
    pub frames: Option<Vec<u64>>,
    pub frame_size: Option<String>,
    pub repeat: u32,
    pub tol: Option<String>,
    pub names: bool,
    pub install: Option<PathBuf>,
    pub mod_name: Option<String>,
    pub fake: bool,
    pub timeout: u64,
    pub rest: Vec<String>,
}

fn parse(args: Vec<String>) -> Result<Opts, String> {
    let mut o = Opts { repeat: 1, timeout: 60, ..Opts::default() };
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let mut val = |flag: &str| it.next().ok_or(format!("{flag} needs a value"));
        match a.as_str() {
            "--bench-dir" => o.bench_dir = Some(val(&a)?.into()),
            "--out" => o.out = Some(val(&a)?.into()),
            "--no-report" => o.no_report = true,
            "--sta64" => o.sta64 = Some(val(&a)?.into()),
            "--session" => o.session = Some(val(&a)?),
            "--scene" => o.scene = Some(val(&a)?),
            "--tier" => o.tier = Some(val(&a)?.parse().map_err(|_| "--tier: 1 or 2")?),
            "--frames" => {
                let v = val(&a)?;
                o.frames = Some(
                    v.split(',')
                        .map(|t| t.trim().parse())
                        .collect::<Result<_, _>>()
                        .map_err(|_| "--frames: ticks like 0,100".to_string())?,
                );
            }
            "--tol" => o.tol = Some(val(&a)?),
            "--names" => o.names = true,
            "--install" => o.install = Some(val(&a)?.into()),
            "--mod" => o.mod_name = Some(val(&a)?).filter(|m| !m.is_empty()),
            "--fake" => o.fake = true,
            "--frame-size" => o.frame_size = Some(val(&a)?),
            "--repeat" => o.repeat = val(&a)?.parse().map_err(|_| "--repeat: a number")?,
            "--timeout" => o.timeout = val(&a)?.parse().map_err(|_| "--timeout: seconds")?,
            _ if a.starts_with("--") => return Err(format!("unknown option {a}")),
            _ => o.rest.push(a),
        }
    }
    Ok(o)
}

/// The runner's content and directories, with `--bench-dir` applied.
pub(crate) fn config(o: &Opts) -> Arc<RunConfig> {
    let mut cfg = RunConfig::fake();
    if let Some(d) = &o.bench_dir {
        cfg.scene_dir = d.join("scenes");
        cfg.scenario_dir = d.join("scenarios");
    }
    Arc::new(cfg)
}

fn report_root(o: &Opts) -> PathBuf {
    o.out.clone().unwrap_or_else(default_report_dir)
}

/// Run the command line; returns the process exit code (0 ok, 1 a check or
/// diff failed, 2 usage).
pub fn main(args: Vec<String>) -> i32 {
    let Some((cmd, rest)) = args.split_first() else {
        eprint!("{USAGE}");
        return 2;
    };
    if matches!(cmd.as_str(), "help" | "--help" | "-h") {
        print!("{USAGE}");
        return 0;
    }
    let o = match parse(rest.to_vec()) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("sta-bench: {e}");
            return 2;
        }
    };
    let result = match cmd.as_str() {
        "list" => list(&o),
        "run" => run_cmd(&o),
        "check" => check(&o),
        "session" => session::session(&o),
        "drive" => session::drive(&o),
        "diff" => diff::diff(&o),
        "parity" => parity::parity(&o),
        "capture" => capture::capture_cmd(&o),
        other => Err(Fail::Usage(format!("unknown command `{other}`"))),
    };
    match result {
        Ok(code) => code,
        Err(Fail::Usage(e)) => {
            eprintln!("sta-bench: {e}\n(sta-bench --help)");
            2
        }
        Err(Fail::Error(e)) => {
            eprintln!("sta-bench: {e}");
            1
        }
    }
}

pub(crate) enum Fail {
    Usage(String),
    Error(String),
}

pub(crate) type Outcome = Result<i32, Fail>;

fn list(o: &Opts) -> Outcome {
    let cfg = config(o);
    let what = o.rest.first().map_or("", String::as_str);
    if !matches!(what, "" | "scenarios" | "scenes") {
        return Err(Fail::Usage(format!("list: scenarios or scenes, not `{what}`")));
    }
    if what != "scenes" {
        println!("scenarios ({}):", cfg.scenario_dir.display());
        for p in discover(&cfg.scenario_dir) {
            let name = p.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
            match load_scenario(&cfg, &p) {
                Ok(sc) => println!("  {name:<20} tier {}  {}", tier(&sc).unwrap_or(1), sc.title),
                Err(e) => println!("  {name:<20} unreadable: {e}"),
            }
        }
    }
    if what != "scenarios" {
        println!("scenes ({}):", cfg.scene_dir.display());
        let mut scenes: Vec<String> = std::fs::read_dir(&cfg.scene_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                (p.extension()? == "ini").then(|| p.file_stem().unwrap().to_string_lossy().into_owned())
            })
            .collect();
        scenes.sort();
        for s in scenes {
            println!("  {s}");
        }
    }
    Ok(0)
}

/// A scenario named on the command line: a path, or a name in `scenario_dir`.
fn scenario_path(cfg: &RunConfig, arg: &str) -> PathBuf {
    let p = Path::new(arg);
    if p.extension().is_some_and(|e| e == "md") || p.exists() {
        p.to_owned()
    } else {
        cfg.scenario_dir.join(format!("{arg}.md"))
    }
}

fn failed_case(name: &str, error: String) -> CaseResult {
    CaseResult {
        name: name.to_owned(),
        title: String::new(),
        passed: false,
        error: Some(error),
        steps: Vec::new(),
        log: Vec::new(),
        digests: Vec::new(),
        ticks: 0,
        artifacts: Default::default(),
    }
}

fn print_case(c: &CaseResult) {
    let n = |s| c.steps.iter().filter(|x| x.status == s).count();
    println!(
        "{:<5} {}  ({} ticks; steps {} pass, {} fail, {} skip)",
        if c.passed { "PASS" } else { "FAIL" },
        c.name,
        c.ticks,
        n(Status::Pass),
        n(Status::Fail),
        n(Status::Skip)
    );
    if !c.passed {
        print!("{}", c.summary());
    }
}

fn finish(o: &Opts, cases: &[CaseResult], staged: Option<&Path>) -> Outcome {
    for c in cases {
        print_case(c);
    }
    let bad = cases.iter().filter(|c| !c.passed).count();
    println!("{} cases, {} passed, {} failed", cases.len(), cases.len() - bad, bad);
    if !o.no_report {
        match write_report(&report_root(o), cases) {
            Ok(dir) => {
                if let Some(s) = staged {
                    tier2::adopt_frames(s, &dir, cases);
                }
                println!("report: {}", dir.display());
            }
            Err(e) => eprintln!("sta-bench: could not write the report: {e}"),
        }
    }
    if let Some(s) = staged {
        let _ = std::fs::remove_dir_all(s);
    }
    Ok(i32::from(bad > 0))
}

fn run_cmd(o: &Opts) -> Outcome {
    if o.rest.is_empty() {
        return Err(Fail::Usage("run: name a scenario (sta-bench list)".into()));
    }
    let cfg = config(o);
    let staged = report_root(o).join(format!(".staging-{}", std::process::id()));
    let mut cases = Vec::new();
    for arg in &o.rest {
        let path = scenario_path(&cfg, arg);
        let name = path.file_stem().map_or_else(|| arg.clone(), |s| s.to_string_lossy().into_owned());
        let sc = match load_scenario(&cfg, &path) {
            Ok(sc) => sc,
            Err(e) => {
                cases.push(failed_case(&name, e));
                continue;
            }
        };
        let mut case = run::run_scenario(&cfg, &sc, &name);
        if o.tier == Some(2) || (o.tier.is_none() && tier(&sc) == Some(2)) {
            tier2::run_case(&cfg, &sc, &mut case, o, &staged);
        }
        cases.push(case);
    }
    finish(o, &cases, Some(&staged))
}

fn check(o: &Opts) -> Outcome {
    let cfg = config(o);
    let files = discover(&cfg.scenario_dir);
    if files.is_empty() {
        return Err(Fail::Error(format!("no scenarios in {}", cfg.scenario_dir.display())));
    }
    let mut cases = Vec::new();
    for p in files {
        let name = p.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        match load_scenario(&cfg, &p) {
            Ok(sc) if tier(&sc) == Some(2) => println!("skip  {name} (tier 2: sta-bench run {name})"),
            Ok(sc) => cases.push(run::run_scenario(&cfg, &sc, &name)),
            Err(e) => cases.push(failed_case(&name, e)),
        }
    }
    finish(o, &cases, None)
}

/// A step the CLI adds to a case (line 0: not from the scenario file).
pub(crate) fn step(status: Status, text: &str, detail: impl Into<String>) -> StepOutcome {
    StepOutcome { line: 0, text: text.to_owned(), status, detail: detail.into() }
}
