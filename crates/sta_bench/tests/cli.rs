//! The `sta-bench` binary on tier-1 scenarios: list, check, diff.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bench_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench")
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sta-bench-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sta-bench")).args(args).output().unwrap()
}

fn text(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// The single run directory under a report root.
fn run_dir(root: &Path) -> PathBuf {
    std::fs::read_dir(root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.join("results.json").is_file())
        .expect("a report directory")
}

#[test]
fn list_shows_scenarios_with_tier_and_scenes() {
    let o = run(&["list"]);
    assert!(o.status.success());
    let t = text(&o);
    assert!(t.contains("planet") && t.contains("tier 1"), "{t}");
    assert!(t.contains("scenes (") && t.contains("synthetic"), "{t}");
    let only = text(&run(&["list", "scenes"]));
    assert!(!only.contains("scenarios (") && only.contains("planet"), "{only}");
    assert_eq!(run(&["list", "bogus"]).status.code(), Some(2));
}

#[test]
fn check_passes_the_repository_and_fails_a_broken_scenario() {
    let out = tmp("check-ok");
    let o = run(&["check", "--out", out.to_str().unwrap()]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("0 failed"), "{}", text(&o));
    assert!(run_dir(&out).join("planet/digests").is_file());

    // A bench directory whose one scenario expects the impossible.
    let dir = tmp("check-bad");
    std::fs::create_dir_all(dir.join("scenarios")).unwrap();
    std::fs::create_dir_all(dir.join("scenes")).unwrap();
    std::fs::copy(bench_dir().join("scenes/planet.ini"), dir.join("scenes/planet.ini")).unwrap();
    std::fs::write(
        dir.join("scenarios/bad.md"),
        "# bad\n\nScene: planet\n\n1. Run 5 ticks\n2. Expect \"ship\" speed is 99\n",
    )
    .unwrap();
    let o = run(&["check", "--bench-dir", dir.to_str().unwrap(), "--no-report"]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("FAIL  bad"), "{}", text(&o));
}

#[test]
fn run_scenarios_and_diff_reports() {
    let (a, b) = (tmp("diff-a"), tmp("diff-b"));
    for d in [&a, &b] {
        let o = run(&["run", "planet", "firing", "--out", d.to_str().unwrap()]);
        assert!(o.status.success(), "{}", text(&o));
    }
    let (ra, rb) = (run_dir(&a), run_dir(&b));
    let o = run(&["diff", ra.to_str().unwrap(), rb.to_str().unwrap()]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("same  planet") && text(&o).contains("0 of 2"), "{}", text(&o));

    // Change one digest, one object and one frame in B.
    let d = rb.join("planet/digests");
    let changed = std::fs::read_to_string(&d).unwrap().replacen('0', "f", 1);
    std::fs::write(&d, changed).unwrap();
    let j = rb.join("planet/objects.jsonl");
    let changed = std::fs::read_to_string(&j).unwrap().replace("\"hull\":[", "\"hull\":[1");
    std::fs::write(&j, changed).unwrap();
    for r in [&ra, &rb] {
        std::fs::create_dir_all(r.join("planet/frames")).unwrap();
    }
    std::fs::write(ra.join("planet/frames/f.png"), b"one").unwrap();
    std::fs::write(rb.join("planet/frames/f.png"), b"two").unwrap();
    let o = run(&["diff", ra.to_str().unwrap(), rb.to_str().unwrap()]);
    let t = text(&o);
    assert_eq!(o.status.code(), Some(1), "{t}");
    assert!(t.contains("DIFF  planet") && t.contains("digests: line"), "{t}");
    assert!(t.contains("objects: `ship` differs") && t.contains("frames: f.png differs"), "{t}");
    assert!(t.contains("same  firing"), "{t}");
    assert_eq!(run(&["diff", ra.to_str().unwrap(), "/nonexistent"]).status.code(), Some(1));
}

/// Every file of a report except the run id, wall times and the index page.
fn report_files(dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    fn rec(root: &Path, d: &Path, out: &mut std::collections::BTreeMap<String, Vec<u8>>) {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                rec(root, &p, out);
            } else {
                let rel = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
                if rel == "timing.json" || rel == "index.html" {
                    continue;
                }
                let mut body = std::fs::read(&p).unwrap();
                if rel == "results.json" {
                    // Drop the run id: `{"run":"<stamp>-<pid>",...`.
                    let t = String::from_utf8(body).unwrap();
                    let at = t.find(",\"passed\"").unwrap();
                    body = t[at..].as_bytes().to_vec();
                }
                out.insert(rel, body);
            }
        }
    }
    let mut out = Default::default();
    rec(dir, dir, &mut out);
    out
}

#[test]
fn jobs_do_not_change_the_report_and_it_has_an_index() {
    let (one, two) = (tmp("jobs1"), tmp("jobs2"));
    let a = run(&["check", "--jobs", "1", "--out", one.to_str().unwrap()]);
    let b = run(&["check", "--jobs", "2", "--out", two.to_str().unwrap()]);
    assert!(a.status.success() && b.status.success(), "{}{}", text(&a), text(&b));
    let (da, db) = (run_dir(&one), run_dir(&two));
    assert_eq!(report_files(&da), report_files(&db), "reports differ between --jobs 1 and 2");
    let index = std::fs::read_to_string(da.join("index.html")).unwrap();
    assert!(index.contains("<table>") && index.contains("planet/log.txt"), "{index}");
    let js = std::fs::read_to_string(da.join("results.json")).unwrap();
    assert!(js.contains("\"failed\":0") && js.contains("\"final_digest\""), "{js}");
    assert!(std::fs::read_to_string(da.join("timing.json")).unwrap().contains("\"jobs\":1"));
    assert!(std::fs::read_to_string(da.join("report.md")).unwrap().contains("**PASS**"));
    assert!(text(&b).contains("wall "), "{}", text(&b));
    assert_eq!(run(&["check", "--jobs", "x"]).status.code(), Some(2));
}

#[test]
fn a_failing_case_fails_the_run_at_any_jobs() {
    let dir = tmp("jobs-bad");
    std::fs::create_dir_all(dir.join("scenarios")).unwrap();
    std::fs::create_dir_all(dir.join("scenes")).unwrap();
    std::fs::copy(bench_dir().join("scenes/planet.ini"), dir.join("scenes/planet.ini")).unwrap();
    std::fs::write(dir.join("scenarios/bad.md"), "# bad\n\nScene: planet\n\n1. Run 5 ticks\n2. Expect \"ship\" speed is 99\n").unwrap();
    let out = tmp("jobs-bad-out");
    let o = run(&["check", "--jobs", "3", "--bench-dir", dir.to_str().unwrap(), "--out", out.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(std::fs::read_to_string(run_dir(&out).join("report.md")).unwrap().contains("**FAIL**"));
}
