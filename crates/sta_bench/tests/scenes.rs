//! Runs every tier-1 scenario under `bench/scenarios/` on the fake content,
//! so a new scenario is picked up by `cargo test` with no edit here.
//! Each run writes a report under the target directory.

use std::sync::Arc;

use sta_bench::run::{self, RunConfig};

#[test]
fn tier1_scenarios_pass() {
    let cfg = Arc::new(RunConfig::fake());
    let mut cases = Vec::new();
    for path in run::discover(&cfg.scenario_dir) {
        let text = std::fs::read_to_string(&path).expect("scenario readable");
        let sc = sta_bench::parse::Scenario::parse(&text)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        if run::tier(&sc) != Some(1) {
            continue;
        }
        cases.push(run::run_scenario_file(&cfg, &path).unwrap_or_else(|e| panic!("{e}")));
    }
    assert!(
        !cases.is_empty(),
        "no tier-1 scenarios in {}",
        cfg.scenario_dir.display()
    );
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("bench-results");
    let report = run::write_report(&root, &cases).expect("report written");
    let mut bad = String::new();
    for c in &cases {
        eprintln!("{}", c.summary());
        if !c.passed {
            bad += &c.summary();
        }
    }
    assert!(
        bad.is_empty(),
        "failing scenarios (report: {}):\n{bad}",
        report.display()
    );
}
