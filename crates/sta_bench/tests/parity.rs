//! Parity diff against the fixtures (`tests/fixtures/parity/`; see NOTICE.md).

use std::path::PathBuf;

use sta_bench::parity::{self, Finding, Source, Tolerances};

fn dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/parity").join(name)
}

#[test]
fn identical_runs_are_empty() {
    for (d, s) in [("refit", Source::Refit), ("ours", Source::Ours)] {
        let (a, b) = (parity::load(&dir(d), s).unwrap(), parity::load(&dir(d), s).unwrap());
        let diff = parity::diff(&a, &b, &Tolerances::default());
        assert!(diff.is_empty(), "{}", diff.report("a", "b"));
        assert!(diff.marks_compared > 0 && diff.objects_compared == 3, "{d}");
    }
}

#[test]
fn refit_against_ours() {
    let a = parity::load(&dir("refit"), Source::Refit).unwrap();
    let b = parity::load(&dir("ours"), Source::Ours).unwrap();
    // Refit marks in seconds from `ready`; ours from ticks.
    let cloak = a.marks.iter().find(|m| m.kind == "cloak.start").unwrap();
    assert!((cloak.t - 2.97).abs() < 1e-9);
    let ours = b.marks.iter().find(|m| m.kind == "cloak.start").unwrap();
    assert!((ours.t - 89.0 / 30.0).abs() < 1e-9);

    let d = parity::diff(&a, &b, &Tolerances::default());
    // The only difference is the warbird's ODF (rwarbird vs fx_cloaker) and
    // the objects' ODFs; cloak, decloak and fire marks agree within 0.1 s.
    assert!(
        d.findings.iter().all(|f| matches!(f, Finding::ObjectField { field: "odf", .. })),
        "{}",
        d.report("refit", "ours")
    );
    assert!(d.marks_compared >= 5, "{}", d.report("refit", "ours"));
    assert!(d.notes.iter().any(|n| n.contains("hit x") && n.contains("spawn x")), "{:?}", d.notes);

    // A tight tolerance finds the 0.03 s offsets.
    let tight = Tolerances::parse("time=0.001").unwrap();
    let d = parity::diff(&a, &b, &tight);
    assert!(d.findings.iter().any(|f| matches!(f, Finding::MarkTime { .. })));
    assert!(d.report("refit", "ours").contains("delta"));
}

#[test]
fn missing_files_are_an_error() {
    assert!(parity::load(&dir("nope"), Source::Refit).is_err());
    assert!(parity::load(&dir("ours"), Source::Refit).is_err());
}

#[test]
fn cli_exit_codes() {
    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let (r, o) = (dir("refit"), dir("ours"));
    let (r, o) = (r.to_str().unwrap(), o.to_str().unwrap());
    // Differences (the ODFs): exit 1; with a time tolerance of 1 s still the ODFs.
    assert_eq!(sta_bench::cli::main(args(&["parity", r, o])), 1);
    assert_eq!(sta_bench::cli::main(args(&["parity", r])), 2);
    assert_eq!(sta_bench::cli::main(args(&["parity", "--tol", "x=1", r, o])), 2);
    assert_eq!(sta_bench::cli::main(args(&["parity", "--names"])), 0);
}
