//! Refit's own scene and scenario files as fixtures.
//!
//! The checked-in copies (`bench/scenes/`, `tests/fixtures/`) always run. The
//! sweep over a whole Refit checkout runs when `REFIT_ROOT` names one, and is
//! skipped otherwise, like the format crates' install-gated checks:
//! `REFIT_ROOT=~/Projects/armada2-refit cargo test -p sta_bench -- --nocapture`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use sta_bench::parse::command::{BenchCommand, Switch};
use sta_bench::parse::scenario::{Clock, DroppedKind, StepKind};
use sta_bench::parse::scene::{Anchor, MapObjects};
use sta_bench::parse::{Scenario, SceneFile, System, Target};

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn scene(name: &str) -> SceneFile {
    let path = manifest()
        .join("../../bench/scenes")
        .join(format!("{name}.ini"));
    SceneFile::parse(&fs::read_to_string(&path).expect("fixture"))
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn scenario(name: &str) -> Scenario {
    let dir = manifest().join("tests/fixtures");
    let path = dir.join(format!("{name}.md"));
    let mut load = |n: &str| fs::read_to_string(dir.join(format!("{n}.md"))).ok();
    Scenario::parse_with(&fs::read_to_string(&path).expect("fixture"), &mut load)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn firing_ini() {
    let f = scene("firing");
    assert!(f.warnings.is_empty() && f.unknown.is_empty());
    let s = &f.scene;
    assert_eq!(s.anchor, Anchor::World);
    assert_eq!(
        s.switches
            .iter()
            .map(|x| (x.system, x.on))
            .collect::<Vec<_>>(),
        [
            (System::Fog, false),
            (System::Hud, false),
            (System::Grid, false)
        ]
    );
    assert_eq!(
        (s.seed, s.step_ms, s.pause, s.delay, s.map.as_deref()),
        (None, None, None, None, None)
    );
    let cam = s.camera.as_ref().expect("Camera=").command.clone();
    assert_eq!(
        cam,
        BenchCommand::Orbit {
            target: Target::Point([2000.0, 0.0, 2175.0]),
            yaw: 90.0,
            pitch: 25.0,
            distance: 700.0
        }
    );
    // Objects keep file order, which is build order.
    assert_eq!(
        f.objects
            .iter()
            .map(|o| o.name.as_str())
            .collect::<Vec<_>>(),
        ["target", "shooter"]
    );
    let t = f.object("target").unwrap();
    assert_eq!(
        (t.odf.as_str(), t.team, t.pos),
        ("bbattle1", 2, [2000.0, 0.0, 2350.0])
    );
    assert!(t.immortal && t.heal && !t.weapons && t.engines);
    assert_eq!(t.attack, None);
    let sh = f.object("shooter").unwrap();
    assert_eq!(
        (sh.odf.as_str(), sh.team, sh.pos, sh.heading),
        ("fgalaxy", 1, [2000.0, 0.0, 2000.0], 0.0)
    );
    assert!(!sh.engines && sh.weapons && sh.immortal && !sh.heal);
    assert_eq!(sh.attack.as_deref(), Some("target"));
    // A scene that lists objects wants a clean stage.
    assert_eq!(f.map_objects(), MapObjects::Strip);
}

#[test]
fn planet_ini() {
    let f = scene("planet");
    assert!(f.warnings.is_empty() && f.unknown.is_empty());
    assert_eq!(f.scene.anchor, Anchor::World);
    assert_eq!(f.scene.switch(System::Hud), Some(false));
    let cam = f.scene.camera.as_ref().unwrap().command.clone();
    assert_eq!(
        cam,
        BenchCommand::Orbit {
            target: Target::Object("ship".into()),
            yaw: 230.0,
            pitch: 15.0,
            distance: 450.0
        }
    );
    assert_eq!(f.objects.len(), 2);
    let p = f.object("planet").unwrap();
    assert_eq!(
        (p.odf.as_str(), p.team, p.pos),
        ("pb_clssm", 0, [2000.0, 0.0, 2000.0])
    );
    let s = f.object("ship").unwrap();
    assert_eq!(
        (s.odf.as_str(), s.team, s.pos, s.heading),
        ("fgalaxy", 1, [1650.0, 0.0, 1750.0], 60.0)
    );
    // Planets are neutral and not colonised by this scene.
    assert_eq!(p.population, None);
}

#[test]
fn cloak_scenario() {
    let sc = scenario("cloak");
    assert_eq!(sc.title, "Cloaked ships drawn as glass by Render3D.asi");
    let scene = sc.headers.scene.as_ref().expect("Setup: bench-setup.sh");
    assert_eq!((scene.name.as_str(), scene.overrides.len()), ("cloak", 0));
    assert_eq!(sc.headers.mod_name.as_deref(), Some("refit"));
    assert_eq!(sc.headers.timeout_secs, Some(360));
    assert_eq!(sc.steps.len(), 27);
    assert_eq!(sc.tally(), (23, 4, 0));

    // Step 5: Scene "cloak warbird on".
    let s5 = &sc.steps[4];
    let StepKind::Scene { commands } = &s5.kind else {
        panic!("{:?}", s5.kind)
    };
    assert!(matches!(&commands[0].command, BenchCommand::Cloak { name, .. } if name == "warbird"));
    // Step 9: Expect scene "query" answers "warbird handle".
    let StepKind::ExpectSceneAnswers {
        command,
        text,
        negated,
    } = &sc.steps[8].kind
    else {
        panic!()
    };
    assert_eq!(
        (&command.command, text.as_str(), *negated),
        (&BenchCommand::Query, "warbird handle", false)
    );
    // Refit's wall-clock waits stay distinct from `Run`.
    assert_eq!(sc.steps[5].kind, StepKind::WaitSeconds(1.0));
    assert!(matches!(
        sc.steps[0].kind,
        StepKind::Dropped(DroppedKind::Process)
    ));
    assert!(matches!(
        sc.steps[1].kind,
        StepKind::Dropped(DroppedKind::LogWait)
    ));
    assert_eq!(
        sc.steps[3].kind,
        StepKind::Screenshot("cloak-before".into())
    );
    assert_eq!(sc.steps[26].kind, StepKind::ExpectNoCrash);
    assert_eq!(sc.steps[26].line, sc.steps[25].line + 1);
}

#[test]
fn port_firing_scenario_maps_the_setup_header_to_a_scene() {
    let sc = scenario("port-firing");
    let scene = sc.headers.scene.as_ref().unwrap();
    assert_eq!(scene.name, "firing");
    assert_eq!(
        scene.overrides,
        [
            ("Seed".to_owned(), "7".to_owned()),
            ("Step".into(), "16".into()),
            ("Pause".into(), "200".into())
        ]
    );
    assert_eq!(sc.headers.clock, Some(Clock::Bench));
    let keys: Vec<_> = sc.headers.other.iter().map(|h| h.key.as_str()).collect();
    assert_eq!(keys, ["Resolutions", "Launch"]);
    assert_eq!(sc.steps[3].kind, StepKind::Screenshot("firing".into()));
    assert_eq!(sc.steps[4].kind, StepKind::DumpFrame("firing".into()));
}

#[test]
fn bench_determinism_scenario() {
    let sc = scenario("bench-determinism");
    let shots: Vec<_> = sc
        .steps
        .iter()
        .filter_map(|s| match &s.kind {
            StepKind::ShotsMatch { a, b, within } => Some((a.as_str(), b.as_str(), *within)),
            _ => None,
        })
        .collect();
    assert_eq!(
        shots,
        [
            ("first", "first again", Some(0.0)),
            ("first", "second", Some(0.01))
        ]
    );
    // `Expect "Render3D.log" contains ...` is a typed log check; the runner skips logs it has not got.
    assert!(
        sc.steps
            .iter()
            .any(|s| matches!(&s.kind, StepKind::ExpectLog { log, .. } if log == "Render3D.log"))
    );
}

#[test]
fn selection_scenario_orders_and_a_long_select() {
    let sc = scenario("qol-big-selection");
    let StepKind::Scene { commands } = &sc.steps[4].kind else {
        panic!()
    };
    let BenchCommand::Select { names, additive } = &commands[0].command else {
        panic!()
    };
    assert_eq!(
        (names.len(), names[19].as_str(), *additive),
        (20, "g20", false)
    );
    let StepKind::ExpectSceneAnswers { text, .. } = &sc.steps[5].kind else {
        panic!()
    };
    assert_eq!(text, "selected 20");
    // A judged `Check that ...` step has no counterpart here and stays visible.
    let unrec: Vec<_> = sc
        .steps
        .iter()
        .filter(|s| s.kind == StepKind::Unrecognised)
        .map(|s| s.line)
        .collect();
    assert_eq!(unrec, [sc.steps[8].line]);
    assert!(
        sc.steps[8]
            .text
            .starts_with("Check that the screenshot shows")
    );
    // Key presses are dropped input.
    assert!(matches!(
        sc.steps[9].kind,
        StepKind::Dropped(DroppedKind::Input)
    ));
}

#[test]
fn combat_scenario_with_bench_edits() {
    let sc = scenario("shields-hits");
    let cmds: Vec<BenchCommand> = sc
        .steps
        .iter()
        .filter_map(|s| match &s.kind {
            StepKind::Scene { commands } => Some(commands.iter().map(|c| c.command.clone())),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(
        matches!(&cmds[2], BenchCommand::Spawn { odf, team: 2, heading, .. } if odf == "kcruise1" && *heading == 180.0)
    );
    assert!(cmds.contains(&BenchCommand::Set {
        switch: Switch::Immortal,
        name: "ktgt".into(),
        on: true
    }));
    assert!(cmds.contains(&BenchCommand::Attack {
        who: "shooter".into(),
        target: "ktgt".into()
    }));
    assert_eq!(cmds.len(), 10);
}

#[test]
fn specials_scenario_parses_every_command() {
    let sc = scenario("specials");
    let n: usize = sc
        .steps
        .iter()
        .map(|s| match &s.kind {
            StepKind::Scene { commands } => commands.len(),
            _ => 0,
        })
        .sum();
    assert!(n >= 32, "{n}");
}

#[test]
fn ui_scenario_is_mostly_dropped_steps() {
    let sc = scenario("ui-menus-single");
    let (typed, dropped, unrec) = sc.tally();
    assert_eq!(typed + dropped + unrec, sc.steps.len());
    assert!(dropped > typed, "{typed} typed, {dropped} dropped");
}

#[test]
fn fragment_parses_alone() {
    let sc = scenario("_enter-instant-action");
    assert!(sc.title.starts_with("Fragment: "));
    assert!(sc.headers.scene.is_none());
    assert!(!sc.steps.is_empty());
}

// ---- the sweep over a whole Refit checkout --------------------------------

fn refit_root() -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var_os("REFIT_ROOT")?);
    assert!(
        root.join("testbench/scenarios").is_dir(),
        "REFIT_ROOT={} is not a Refit checkout",
        root.display()
    );
    Some(root)
}

fn files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == ext))
        .collect();
    v.sort();
    v
}

#[test]
fn sweep_refit_checkout() {
    let Some(root) = refit_root() else {
        eprintln!("REFIT_ROOT not set: skipping the sweep of Refit's scenes and scenarios");
        return;
    };

    // Scenes: every one must parse. Unknown keys are listed, not refused.
    let mut failures = Vec::new();
    let mut scenes = 0;
    let mut objects = 0;
    let mut unknown: BTreeMap<String, usize> = BTreeMap::new();
    for p in files(&root.join("testbench/scene/scenes"), "ini") {
        scenes += 1;
        match SceneFile::parse(&fs::read_to_string(&p).unwrap()) {
            Ok(f) => {
                objects += f.objects.len();
                for u in f.unknown {
                    *unknown
                        .entry(format!(
                            "[{}] {}",
                            u.section.split('.').next().unwrap(),
                            u.key
                        ))
                        .or_default() += 1;
                }
            }
            Err(e) => failures.push(format!("{}: {e}", p.display())),
        }
    }
    eprintln!(
        "scenes: {scenes} parsed, {objects} objects, {} failed",
        failures.len()
    );
    eprintln!("scene keys without a type: {unknown:?}");

    // Scenarios: every one must parse; steps are tallied.
    let dir = root.join("testbench/scenarios");
    let mut load = |n: &str| fs::read_to_string(dir.join(format!("{n}.md"))).ok();
    let (mut files_n, mut typed, mut dropped, mut unrec) = (0, 0, 0, 0);
    let mut unrec_texts: BTreeMap<String, usize> = BTreeMap::new();
    let mut verbs: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_kind: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut waits = 0;
    for p in files(&dir, "md") {
        if p.file_name().is_some_and(|n| n == "README.md") {
            continue;
        }
        files_n += 1;
        let sc = match Scenario::parse_with(&fs::read_to_string(&p).unwrap(), &mut load) {
            Ok(sc) => sc,
            Err(e) => {
                failures.push(format!("{}: {e}", p.display()));
                continue;
            }
        };
        let (t, d, u) = sc.tally();
        (typed, dropped, unrec) = (typed + t, dropped + d, unrec + u);
        for s in &sc.steps {
            match &s.kind {
                StepKind::Unrecognised => {
                    let shape: String = s
                        .text
                        .split('"')
                        .step_by(2)
                        .collect::<Vec<_>>()
                        .join("\"_\"");
                    *unrec_texts.entry(shape).or_default() += 1;
                }
                StepKind::Scene { commands } => {
                    for c in commands {
                        if let BenchCommand::Unknown { verb, .. } = &c.command {
                            *verbs.entry(verb.clone()).or_default() += 1;
                        }
                    }
                }
                StepKind::Dropped(k) => {
                    *by_kind
                        .entry(match k {
                            DroppedKind::Process => "process",
                            DroppedKind::Input => "input",
                            DroppedKind::Ocr => "ocr",
                            DroppedKind::LogWait => "log wait",
                        })
                        .or_default() += 1;
                }
                StepKind::WaitSeconds(_) => waits += 1,
                _ => {}
            }
        }
    }
    eprintln!(
        "scenarios: {files_n} parsed; steps: {} recognised ({waits} Wait N seconds), {dropped} dropped {by_kind:?}, {unrec} unrecognised",
        typed
    );
    eprintln!("unknown Scene verbs (answered `! unsupported`): {verbs:?}");
    let mut top: Vec<_> = unrec_texts.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (shape, n) in top.iter().take(25) {
        eprintln!("  {n:4}  {shape}");
    }
    assert!(
        failures.is_empty(),
        "{} files failed to parse:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(scenes > 0 && files_n > 0);
}
