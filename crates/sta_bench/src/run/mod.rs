//! The tier-1 runner: scene -> `GameSetup` -> `Sim`, scenario steps against
//! it, and the report directory (`docs/TESTBENCH.md`, "Architecture").
//!
//! In-process, no GPU, milliseconds a case. Bench commands reach the sim as
//! `sta_sim::Command`s: orders from the owner of the unit, edits from
//! `PlayerId::BENCH` (S2), applied at the next tick boundary. Scene objects
//! and `spawn` are placed directly with `Sim::spawn_def` before the step they
//! precede, so ids are known at once (see TESTBENCH, "Tier 1 runner").
//!
//! Entry points: [`run_scenario`], [`run_scenario_file`], [`discover`],
//! [`write_report`]. The checks a scenario can make are in [`check`] (state)
//! and [`marks`] (events).

mod bench;
pub mod check;
mod json;
pub mod marks;
mod report;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sta_content::{ContentDb, MapDef};
use sta_core::{Fixed, TICK_HZ, Tick};

pub use bench::Bench;
pub use report::{default_report_dir, write_report};

use crate::parse::scenario::{Scenario, StepKind};
use crate::parse::scene::{MapObjects, Start};
use crate::parse::{SceneFile, TimeSpec};

/// `Fixed` as `f64`, for reports and tolerance checks only.
pub(crate) fn to_f64(f: Fixed) -> f64 {
    f.to_bits() as f64 / 4_294_967_296.0
}

/// What a runner needs besides the scenario.
pub struct RunConfig {
    pub content: Arc<ContentDb>,
    /// Stock ODF name (lower case) -> a name in `content`, for scenes written
    /// against the game's ODFs (`fgalaxy`) that run on other content.
    pub aliases: BTreeMap<String, String>,
    /// Named maps `Map=` can load (the BZN loader arrives with CON-07).
    pub maps: BTreeMap<String, MapDef>,
    /// `bench/scenes`.
    pub scene_dir: PathBuf,
    /// `bench/scenarios`.
    pub scenario_dir: PathBuf,
    /// Where `Start=snapshot:` and `Start=replay:` files live, and where
    /// `Save snapshot as` writes. Relative file names resolve here.
    pub data_dir: PathBuf,
}

impl RunConfig {
    /// `fake::db()` with the stock names the checked-in scenes use mapped on
    /// (Galaxy -> cruiser, Borg cube -> frigate, class M planet -> moon),
    /// the repository's `bench/` directories and a temporary data directory.
    pub fn fake() -> RunConfig {
        let bench = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench");
        let aliases = [
            ("fgalaxy", "fx_cruiser"),
            ("bbattle1", "fx_frigate"),
            ("pb_clssm", "fx_moon"),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
        .collect();
        let maps = [("fx_two_bases".to_owned(), sta_content::fake::two_bases())]
            .into_iter()
            .collect();
        RunConfig {
            content: Arc::new(fake_content()),
            aliases,
            maps,
            scene_dir: bench.join("scenes"),
            scenario_dir: bench.join("scenarios"),
            data_dir: std::env::temp_dir().join("sta64-bench-data"),
        }
    }

    pub fn key_of(&self, odf: &str) -> Option<sta_core::ContentKey> {
        let name = self
            .aliases
            .get(&odf.to_ascii_lowercase())
            .map_or(odf, String::as_str);
        self.content.key(name)
    }

    pub fn data_path(&self, file: &str) -> PathBuf {
        let p = Path::new(file);
        if p.is_absolute() {
            p.to_owned()
        } else {
            self.data_dir.join(p)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pass,
    Fail,
    /// Not run here (tier-2 or dropped steps).
    Skip,
    Note,
}

#[derive(Debug, Clone)]
pub struct StepOutcome {
    pub line: usize,
    pub text: String,
    pub status: Status,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct CaseResult {
    pub name: String,
    pub title: String,
    pub passed: bool,
    /// Set when the case could not start (scene missing, content unknown).
    pub error: Option<String>,
    pub steps: Vec<StepOutcome>,
    /// `bench.log`.
    pub log: Vec<String>,
    /// `(tick, digest)` for every tick run.
    pub digests: Vec<(Tick, u64)>,
    pub ticks: Tick,
    /// `objects.jsonl`, `marks`.
    pub artifacts: BTreeMap<String, String>,
}

impl CaseResult {
    pub fn failures(&self) -> impl Iterator<Item = &StepOutcome> {
        self.steps.iter().filter(|s| s.status == Status::Fail)
    }

    /// The report line per step, for test output.
    pub fn summary(&self) -> String {
        let mut out = format!(
            "{} [{}]\n",
            self.name,
            if self.passed { "pass" } else { "FAIL" }
        );
        if let Some(e) = &self.error {
            out += &format!("  error: {e}\n");
        }
        for s in &self.steps {
            let tag = match s.status {
                Status::Pass => "ok  ",
                Status::Fail => "FAIL",
                Status::Skip => "skip",
                Status::Note => "note",
            };
            out += &format!("  {tag} line {:>3}: {} -- {}\n", s.line, s.text, s.detail);
        }
        out
    }
}

/// The scenario's `Tier:` header (`1` = in-process); `None` when absent.
pub fn tier(s: &Scenario) -> Option<u8> {
    s.headers
        .other
        .iter()
        .find(|h| h.key.eq_ignore_ascii_case("tier"))
        .and_then(|h| h.value.trim().parse().ok())
}

/// `*.md` scenario files in `dir` that do not start with `_` (fragments), sorted.
pub fn discover(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "md")
                && !p
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('_'))
        })
        .collect();
    v.sort();
    v
}

/// Load, parse (expanding `Include`) and run one scenario file.
pub fn run_scenario_file(cfg: &Arc<RunConfig>, path: &Path) -> Result<CaseResult, String> {
    run_file(cfg, path, 0)
}

fn load_scenario(cfg: &RunConfig, path: &Path) -> Result<Scenario, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut load = |n: &str| std::fs::read_to_string(cfg.scenario_dir.join(format!("{n}.md"))).ok();
    Scenario::parse_with(&text, &mut load).map_err(|e| format!("{}: {e}", path.display()))
}

fn run_file(cfg: &Arc<RunConfig>, path: &Path, depth: u8) -> Result<CaseResult, String> {
    let sc = load_scenario(cfg, path)?;
    let name = path
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    Ok(run(cfg, &sc, &name, depth))
}

/// Run a parsed scenario.
pub fn run_scenario(cfg: &Arc<RunConfig>, sc: &Scenario, name: &str) -> CaseResult {
    run(cfg, sc, name, 0)
}

fn scene_for(cfg: &RunConfig, sc: &Scenario) -> Result<SceneFile, String> {
    let Some(r) = &sc.headers.scene else {
        return Ok(SceneFile::default());
    };
    let path = cfg.scene_dir.join(format!("{}.ini", r.name));
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut scene = SceneFile::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    for (k, v) in &r.overrides {
        let s = &mut scene.scene;
        match k.to_ascii_lowercase().as_str() {
            "seed" => {
                s.seed = Some(
                    v.parse()
                        .map_err(|_| format!("Scene override Seed={v}: expected a number"))?,
                )
            }
            "pause" => {
                s.pause = Some(TimeSpec::parse(v).ok_or_else(|| {
                    format!("Scene override Pause={v}: expected ticks or seconds")
                })?)
            }
            "map" => s.map = Some(v.clone()),
            "profile" => s.profile = Some(v.clone()),
            "delay" => s.delay = v.parse().ok(),
            "mapobjects" => {
                s.map_objects = Some(match v.to_ascii_lowercase().as_str() {
                    "keep" => MapObjects::Keep,
                    "strip" => MapObjects::Strip,
                    _ => return Err(format!("Scene override MapObjects={v}: keep or strip")),
                })
            }
            "start" => {
                s.start = Some(if let Some(f) = v.strip_prefix("snapshot:") {
                    Start::Snapshot(f.to_owned())
                } else if let Some(f) = v.strip_prefix("replay:") {
                    Start::Replay(f.to_owned())
                } else {
                    return Err(format!(
                        "Scene override Start={v}: snapshot:<file> or replay:<file>"
                    ));
                })
            }
            _ => {
                return Err(format!(
                    "Scene override `{k}={v}` is not one the runner knows"
                ));
            }
        }
    }
    Ok(scene)
}

fn parse_hex(s: &str) -> Option<u64> {
    u64::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()
}

fn run(cfg: &Arc<RunConfig>, sc: &Scenario, name: &str, depth: u8) -> CaseResult {
    let mut res = CaseResult {
        name: name.to_owned(),
        title: sc.title.clone(),
        passed: false,
        error: None,
        steps: Vec::new(),
        log: Vec::new(),
        digests: Vec::new(),
        ticks: 0,
        artifacts: BTreeMap::new(),
    };
    let built = scene_for(cfg, sc).and_then(|scene| Bench::build(cfg.clone(), &scene));
    let mut b = match built {
        Ok(b) => b,
        Err(e) => {
            res.error = Some(e);
            return res;
        }
    };
    let cap = sc
        .headers
        .timeout_secs
        .map_or(120 * u64::from(TICK_HZ), |s| s * u64::from(TICK_HZ));
    for step in &sc.steps {
        let (status, detail) = run_step(cfg, &mut b, &step.kind, &step.text, cap, depth);
        res.steps.push(StepOutcome {
            line: step.line,
            text: step.text.clone(),
            status,
            detail,
        });
    }
    res.passed = res.steps.iter().all(|s| s.status != Status::Fail);
    res.ticks = b.tick();
    res.digests = (0..=b.tick())
        .filter_map(|t| b.digest_at(t).ok().map(|d| (t, d)))
        .collect();
    res.artifacts = std::mem::take(&mut b.artifacts);
    res.artifacts
        .entry("objects.jsonl".into())
        .or_insert_with(|| b.objects_jsonl());
    res.artifacts
        .entry("marks".into())
        .or_insert_with(|| marks::render(&b, &b.marks()));
    res.log = std::mem::take(&mut b.log);
    res
}

fn pass(s: impl Into<String>) -> (Status, String) {
    (Status::Pass, s.into())
}
fn fail(s: impl Into<String>) -> (Status, String) {
    (Status::Fail, s.into())
}
fn verdict(r: Result<String, String>) -> (Status, String) {
    match r {
        Ok(m) => pass(m),
        Err(m) => fail(m),
    }
}

/// Run `n` ticks, stopping early when paused.
fn run_ticks(b: &mut Bench, n: Tick) -> (Status, String) {
    let start = b.tick();
    for _ in 0..n {
        if !b.advance() {
            return (
                Status::Pass,
                format!(
                    "paused at tick {} after {} of {n} ticks",
                    b.tick(),
                    b.tick() - start
                ),
            );
        }
    }
    pass(format!("ran {n} ticks to tick {}", b.tick()))
}

fn run_step(
    cfg: &Arc<RunConfig>,
    b: &mut Bench,
    kind: &StepKind,
    text: &str,
    cap: Tick,
    depth: u8,
) -> (Status, String) {
    match kind {
        StepKind::Scene { commands } => {
            let mut answers = Vec::new();
            for c in commands {
                let a = b.exec_timed(c);
                b.log
                    .push(format!("tick {}: {:?} -> {a}", b.tick(), c.command));
                answers.push(a);
            }
            // A command that failed (`! ...`) fails the step, so a typo in a
            // scenario cannot pass unnoticed.
            match answers.iter().find(|a| a.starts_with('!')) {
                Some(bad) => fail(bad.clone()),
                None => pass(answers.join("; ")),
            }
        }
        StepKind::ExpectSceneAnswers {
            command,
            text,
            negated,
        } => {
            let a = b.exec_timed(command);
            b.log
                .push(format!("tick {}: {:?} -> {a}", b.tick(), command.command));
            let has = a.contains(text.as_str());
            if has != *negated {
                pass(format!("answered \"{a}\""))
            } else {
                fail(format!("answered \"{a}\""))
            }
        }
        StepKind::Run(t) => run_ticks(b, t.ticks(TICK_HZ)),
        StepKind::WaitSeconds(s) => run_ticks(b, TimeSpec::Seconds(*s).ticks(TICK_HZ)),
        StepKind::WaitUntil { within, condition } => {
            let limit = within.map_or(cap, |w| w.ticks(TICK_HZ));
            let start = b.tick();
            loop {
                if check::count(b, condition) == condition.count {
                    return pass(format!("after {} ticks", b.tick() - start));
                }
                if b.tick() - start >= limit {
                    return fail(format!(
                        "count is {}, not {}, after {limit} ticks",
                        check::count(b, condition),
                        condition.count
                    ));
                }
                if !b.advance() {
                    return fail("paused before the condition held");
                }
            }
        }
        StepKind::Expect(cond) => {
            let n = check::count(b, cond);
            if n == cond.count {
                pass(format!("{n} at tick {}", b.tick()))
            } else {
                fail(format!("{n}, wanted {} (tick {})", cond.count, b.tick()))
            }
        }
        StepKind::SelectClass { class, .. } => {
            let want = class.to_ascii_lowercase();
            let ids: Vec<_> = b
                .view()
                .units
                .iter()
                .filter(|u| {
                    cfg.content.unit(u.def).is_some_and(|d| {
                        d.odf.eq_ignore_ascii_case(&want)
                            || d.class.0 == want
                            || cfg
                                .aliases
                                .get(&want)
                                .is_some_and(|a| a.eq_ignore_ascii_case(&d.odf))
                    })
                })
                .map(|u| u.id)
                .collect();
            b.selection = ids;
            pass(format!("selected {}", b.selection.len()))
        }
        StepKind::OrderAttack { who, target } => {
            let names: Vec<String> = match who {
                crate::parse::scenario::Who::Name(n) => vec![n.clone()],
                crate::parse::scenario::Who::Selection => {
                    b.selection.iter().map(|&i| format!("#{}", i.0)).collect()
                }
            };
            let mut last = String::from("nothing selected");
            for n in names {
                last = b.exec(&crate::parse::BenchCommand::Attack {
                    who: n,
                    target: target.clone(),
                });
                if last.starts_with('!') {
                    return fail(last);
                }
            }
            pass(last)
        }
        StepKind::Screenshot(_) | StepKind::DumpFrame(_) | StepKind::ShotsMatch { .. } => {
            (Status::Skip, "frames are tier 2".into())
        }
        StepKind::ExpectNoCrash => pass("the run got here"),
        StepKind::ExpectDigest { at, hex } => {
            let t = at.ticks(TICK_HZ);
            match (parse_hex(hex), b.digest_at(t)) {
                (None, _) => fail(format!("`{hex}` is not hex")),
                (_, Err(e)) => fail(e),
                (Some(want), Ok(got)) if want == got => {
                    pass(format!("digest at tick {t} is {got:016x}"))
                }
                (Some(want), Ok(got)) => fail(format!(
                    "digest at tick {t} is {got:016x}, wanted {want:016x}"
                )),
            }
        }
        StepKind::ExpectDigestMatches { at, scenario } => {
            let t = at.ticks(TICK_HZ);
            if depth >= 1 {
                return fail("`to match run` nests too deep");
            }
            let other = match run_file(
                cfg,
                &cfg.scenario_dir.join(format!("{scenario}.md")),
                depth + 1,
            ) {
                Ok(o) => o,
                Err(e) => return fail(e),
            };
            let theirs = other.digests.iter().find(|(x, _)| *x == t).map(|d| d.1);
            match (b.digest_at(t), theirs) {
                (Ok(a), Some(c)) if a == c => pass(format!(
                    "digest at tick {t} equals run \"{scenario}\" ({a:016x})"
                )),
                (Ok(a), Some(c)) => fail(format!(
                    "digest at tick {t} is {a:016x}, run \"{scenario}\" has {c:016x}"
                )),
                (Err(e), _) => fail(e),
                (_, None) => fail(format!(
                    "run \"{scenario}\" never reached tick {t} (ran {} ticks)",
                    other.ticks
                )),
            }
        }
        StepKind::ExpectMark { mark, by } => match marks::parse_spec(mark) {
            Ok(spec) => verdict(check::eval(
                b,
                &check::Pred::Mark {
                    spec,
                    yes: true,
                    window: check::Window::By(by.ticks(TICK_HZ)),
                },
            )),
            Err(e) => fail(e),
        },
        StepKind::ExpectLog { log, text, negated } => {
            let hay = if log == "bench.log" {
                b.log.join("\n")
            } else if let Some(a) = b.artifacts.get(log.as_str()) {
                a.clone()
            } else {
                return fail(format!(
                    "no log called \"{log}\" (bench.log, or a file a command wrote)"
                ));
            };
            let has = hay.contains(text.as_str());
            if has != *negated {
                pass(format!(
                    "\"{log}\" {} \"{text}\"",
                    if has { "contains" } else { "does not contain" }
                ))
            } else {
                fail(format!(
                    "\"{log}\" {} \"{text}\"",
                    if has { "contains" } else { "does not contain" }
                ))
            }
        }
        StepKind::Note(t) => (Status::Note, t.clone()),
        StepKind::Include(n) => fail(format!("Include \"{n}\" was not expanded")),
        // The parser reads every `Wait up to ...` as one of Refit's OCR or log waits;
        // `Wait up to 60 seconds until <state predicate>` is ours (see `check`).
        StepKind::Dropped(k) => run_extra(b, text, cap)
            .unwrap_or_else(|| (Status::Skip, format!("dropped step ({k:?})"))),
        StepKind::Unrecognised => {
            run_extra(b, text, cap).unwrap_or_else(|| fail("step not recognised"))
        }
    }
}

/// Steps the parser left `Unrecognised` get a second reading as a state check
/// (see [`check`]).
fn run_extra(b: &mut Bench, text: &str, cap: Tick) -> Option<(Status, String)> {
    let extra = match check::parse(text) {
        Ok(Some(e)) => e,
        Ok(None) => return None,
        Err(e) => return Some(fail(e)),
    };
    Some(match extra {
        check::Extra::Expect(p) => verdict(check::eval(b, &p)),
        check::Extra::Wait { within, pred } => verdict(check::wait(
            b,
            &pred,
            within.map_or(cap, |w| w.ticks(TICK_HZ)),
        )),
        check::Extra::RunUntil(t) => {
            let n = t.saturating_sub(b.tick());
            run_ticks(b, n)
        }
        check::Extra::SaveSnapshot(f) => verdict(b.save_snapshot(&f)),
        check::Extra::SaveReplay(f) => verdict(b.save_replay(&f)),
    })
}

/// `fake::db()` plus a construction rig (`fx_rig`) and with `pay_on_queue` on:
/// what the construction scenarios build with. The rig deploys any of the
/// fake content's stations.
pub fn fake_content() -> ContentDb {
    use sta_content::{MoveStats, UnitDef};
    let fake = sta_content::fake::db();
    let mut features = fake.features().clone();
    features.pay_on_queue = true;
    let mut rig = UnitDef::new("fx_rig", "constructionrig");
    rig.race = "fx".into();
    rig.movement = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
    rig.hull = Fixed::from_int(100);
    let mut b = ContentDb::builder()
        .profile(fake.profile())
        .features(features)
        .unit(rig);
    for w in fake.weapons() {
        b = b.weapon(w.clone());
    }
    for u in fake.units() {
        b = b.unit(u.clone());
    }
    b.build().expect("fake content plus a rig is consistent")
}
