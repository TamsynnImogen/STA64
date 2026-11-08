//! The runner's own tests: each check kind passes when it should and fails
//! when it should not, on the fake content.

use std::sync::Arc;

use sta_bench::parse::Scenario;
use sta_bench::run::{self, CaseResult, RunConfig, Status};

fn cfg() -> Arc<RunConfig> {
    Arc::new(RunConfig::fake())
}

/// Run `Scene: <scene>` with the given numbered steps.
fn case(cfg: &Arc<RunConfig>, scene: &str, steps: &[&str]) -> CaseResult {
    let mut text = format!("# t\n\nScene: {scene}\n\n");
    for (i, s) in steps.iter().enumerate() {
        text += &format!("{}. {s}\n", i + 1);
    }
    let sc = Scenario::parse(&text).expect("parses");
    run::run_scenario(cfg, &sc, "t")
}

fn passes(scene: &str, steps: &[&str]) {
    let r = case(&cfg(), scene, steps);
    assert!(r.passed, "{}", r.summary());
}

fn fails(scene: &str, steps: &[&str]) -> CaseResult {
    let r = case(&cfg(), scene, steps);
    assert!(!r.passed, "should have failed:\n{}", r.summary());
    r
}

#[test]
fn field_checks_at_a_tick() {
    passes(
        "firing",
        &[
            "Run 200 ticks",
            "Expect \"shooter\" hull is 800",
            "Expect \"shooter\" hull.max at tick 0 is 800",
            "Expect \"target\" hull at tick 200 is below 300",
            "Expect \"target\" shield.aft at tick 200 is below 150",
            "Expect \"target\" hull at tick 0 is at least 299",
            "Expect \"target\" shield.fore is at most 150",
            "Expect \"shooter\" z is 2000 within 0.001",
            "Expect \"shooter\" x is between 1999 and 2001",
        ],
    );
    fails("firing", &["Run 10 ticks", "Expect \"shooter\" hull is 1"]);
    let r = fails(
        "firing",
        &["Run 10 ticks", "Expect \"shooter\" hull at tick 20 is 800"],
    );
    assert!(r.summary().contains("not reached"), "{}", r.summary());
}

#[test]
fn position_goto_and_arrival() {
    passes(
        "planet",
        &[
            "Scene \"goto ship 1650 0 1400\"",
            "Wait up to 60 seconds until \"ship\" is within 5 of 1650 0 1400",
            "Expect \"ship\" is within 5 of 1650 0 1400",
            "Expect \"ship\" is within 700 of \"planet\"",
        ],
    );
    // Not there yet: the window is too short.
    fails(
        "planet",
        &[
            "Scene \"goto ship 1650 0 1400\"",
            "Wait up to 1 seconds until \"ship\" is within 5 of 1650 0 1400",
        ],
    );
    fails(
        "planet",
        &[
            "Scene \"goto ship 1650 0 1400\"",
            "Run 2 ticks",
            "Expect \"ship\" is within 5 of 1650 0 1400",
        ],
    );
}

#[test]
fn marks_in_windows() {
    passes(
        "firing",
        &[
            "Run 200 ticks",
            "Expect mark \"fire.start obj=shooter weapon=fx_beam\" at tick 0",
            "Expect mark \"hit obj=target by=shooter\" between ticks 0 and 5",
            "Expect no mark \"death\" by tick 200",
            "Expect no mark \"fire.start obj=target\" by tick 0",
            "Expect mark \"spawn obj=#2\" by tick 0",
        ],
    );
    fails(
        "firing",
        &[
            "Run 10 ticks",
            "Expect mark \"fire.start obj=shooter hp=0\" at tick 3",
        ],
    );
    fails(
        "firing",
        &[
            "Run 10 ticks",
            "Expect mark \"death obj=target\" by tick 10",
        ],
    );
    // A tick that has not happened cannot decide a mark.
    fails(
        "firing",
        &["Run 10 ticks", "Expect no mark \"death\" by tick 100"],
    );
}

#[test]
fn existence_after_kill_and_counts() {
    passes(
        "firing",
        &[
            "Expect 1 object on team 1",
            "Expect \"target\" exists",
            "Scene \"kill target\"",
            "Run 2 ticks",
            "Expect \"target\" does not exist",
        ],
    );
    passes(
        "firing",
        &[
            "Scene \"kill target\"",
            "Run 2 ticks",
            "Expect mark \"death obj=target\" by tick 1",
            "Expect \"target\" exists at tick 0",
        ],
    );
    fails("firing", &["Expect \"target\" does not exist"]);
    passes(
        "firing",
        &[
            "Scene \"kill target\"",
            "Wait until no ship is on team 2",
            "Expect no ship is on team 2",
        ],
    );
}

#[test]
fn resources_players_and_visibility() {
    passes(
        "planet",
        &[
            "Scene \"cargo ship metal 500\"",
            "Run 1 ticks",
            "Expect player 1 resource metal is 500",
            "Expect player 1 resource 2 at tick 1 is at least 499",
            "Expect player 1 resource dilithium is 0",
            "Expect player 1 is not eliminated",
            "Expect player 1 sees \"ship\"",
        ],
    );
    fails(
        "planet",
        &[
            "Run 1 ticks",
            "Expect player 1 resource metal is at least 1",
        ],
    );
    fails("planet", &["Expect player 9 is eliminated"]);
}

#[test]
fn digests_pinned_and_compared() {
    let cfg = cfg();
    let r = case(&cfg, "planet", &["Run 20 ticks"]);
    let (_, d) = r.digests[20];
    let pinned = format!("Expect digest at tick 20 is \"{d:016x}\"");
    passes("planet", &["Run 20 ticks", &pinned]);
    let wrong = "Expect digest at tick 20 is \"0000000000000001\"";
    fails("planet", &["Run 20 ticks", wrong]);
    // Different seeds, different run: a digest check against it can differ or not,
    // but the seed override must at least be accepted.
    passes("planet Seed=9", &["Run 5 ticks"]);
}

#[test]
fn time_and_pause_commands() {
    passes(
        "planet",
        &[
            "Expect scene \"pause\" answers \"paused at tick 0\"",
            "Run 10 ticks",
            "Expect scene \"tick\" answers \"tick 0\"",
            "Expect scene \"resume\" answers \"resumed\"",
            "Run 10 ticks",
            "Expect scene \"tick\" answers \"tick 10\"",
        ],
    );
    // `Pause=` in the scene header holds the sim at that tick.
    passes(
        "planet Pause=5",
        &["Run 50 ticks", "Expect scene \"tick\" answers \"tick 5\""],
    );
    // `at <tick>` commands land on their tick.
    passes(
        "firing",
        &[
            "Scene \"at 10 kill target\"",
            "Run 5 ticks",
            "Expect \"target\" exists",
            "Run 10 ticks",
            "Expect \"target\" does not exist",
        ],
    );
}

#[test]
fn bad_steps_and_commands_fail_loudly() {
    fails("planet", &["Frobnicate the widgets"]);
    fails("planet", &["Scene \"goto nobody 1 2 3\""]);
    fails("planet", &["Expect \"ship\" wibble is 3"]);
    fails("planet", &["Expect scene \"objects\" answers \"nope\""]);
    // A missing scene is a case error, not a panic.
    let r = case(&cfg(), "no-such-scene", &["Run 1 ticks"]);
    assert!(!r.passed && r.error.is_some());
    // Tier-2 steps are skipped, not passed.
    let r = case(&cfg(), "planet", &["Take a screenshot called \"a\""]);
    assert!(r.passed && r.steps[0].status == Status::Skip);
}

#[test]
fn unsupported_edits_are_logged_not_fatal() {
    let r = case(
        &cfg(),
        "firing",
        &[
            "Run 3 ticks",
            "Expect \"bench.log\" contains \"rejected by the sim\"",
        ],
    );
    assert!(r.passed, "{}", r.summary());
}

#[test]
fn artifacts_and_report_dir() {
    let cfg = cfg();
    let r = case(&cfg, "planet", &["Scene \"objects\"", "Scene \"marks\""]);
    let objects = &r.artifacts["objects.jsonl"];
    assert!(objects.lines().count() == 3, "{objects}");
    assert!(objects.lines().next().unwrap().contains("\"schema\":1"));
    let dir = std::env::temp_dir().join(format!("sta64-bench-report-test-{}", std::process::id()));
    let run = run::write_report(&dir, &[r]).unwrap();
    for f in [
        "report.md",
        "results.json",
        "t/log.txt",
        "t/bench.log",
        "t/objects.jsonl",
        "t/marks",
    ] {
        assert!(run.join(f).is_file(), "{f}");
    }
    #[cfg(unix)]
    assert!(dir.join("latest/report.md").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn replay_start_runs_the_recorded_commands() {
    use sta_core::{PlayerId, TeamId};
    use sta_sim::command::{Command, CommandKind, Edit};
    use sta_sim::replay::Recorder;
    use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};

    let mut cfg = RunConfig::fake();
    let db = cfg.content.clone();
    let setup = GameSetup {
        seed: 3,
        map: Arc::new(sta_content::MapDef::empty()),
        content: db.clone(),
        players: vec![PlayerSetup {
            id: PlayerId(1),
            team: TeamId(1),
            race: "fx".into(),
            control: Control::Idle,
        }],
        options: GameOptions::default(),
    };
    let mut rec = Recorder::new(&setup);
    let spawn = Edit::Spawn {
        odf: db.key("fx_cruiser").unwrap(),
        player: PlayerId(1),
        pos: sta_core::Vec3::ZERO,
        heading: (sta_core::Fixed::ZERO, sta_core::Fixed::ONE),
    };
    rec.step(&[Command {
        player: PlayerId::BENCH,
        seq: 0,
        kind: CommandKind::Edit(spawn),
    }]);
    for _ in 0..20 {
        rec.step(&[]);
    }
    let end = rec.sim().digest();
    let bytes = rec.finish().encode();

    let dir = std::env::temp_dir().join(format!("sta64-bench-replay-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("rec.replay"), bytes).unwrap();
    std::fs::write(
        dir.join("replayed.ini"),
        "[Scene]\nStart=replay:rec.replay\n",
    )
    .unwrap();
    cfg.scene_dir = dir.clone();
    cfg.data_dir = dir.clone();
    let cfg = Arc::new(cfg);
    let pinned = format!("Expect digest at tick 21 is \"{end:016x}\"");
    let r = case(
        &cfg,
        "replayed",
        &[
            "Run 21 ticks",
            "Expect mark \"spawn obj=#1\" by tick 0",
            &pinned,
            "Expect \"bench.log\" does not contain \"desync\"",
        ],
    );
    let _ = std::fs::remove_dir_all(&dir);
    // The last step is `does not contain`, which the parser may read as a typed log step.
    assert!(r.passed, "{}", r.summary());
}

#[test]
fn snapshot_start_continues_the_saved_run() {
    let dir = std::env::temp_dir().join(format!("sta64-bench-snap-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("resumed.ini"),
        "[Scene]\nStart=snapshot:mid.snap\n",
    )
    .unwrap();
    // Run planet with a goto, save at tick 30, go on to tick 60.
    let mut cfg = RunConfig::fake();
    cfg.data_dir = dir.clone();
    let cfg = Arc::new(cfg);
    let r = case(
        &cfg,
        "planet",
        &[
            "Scene \"goto ship 1650 0 1400\"",
            "Run 30 ticks",
            "Save snapshot as \"mid.snap\"",
            "Run 30 ticks",
        ],
    );
    assert!(r.passed, "{}", r.summary());
    let straight = r.digests[60].1;
    // Resume from the snapshot: the same digest 30 ticks on, the order carried along.
    let mut cfg = RunConfig::fake();
    cfg.data_dir = dir.clone();
    cfg.scene_dir = dir.clone();
    let cfg = Arc::new(cfg);
    let pinned = format!("Expect digest at tick 60 is \"{straight:016x}\"");
    let r = case(&cfg, "resumed", &["Run until tick 60", &pinned]);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(r.passed, "{}", r.summary());
}
