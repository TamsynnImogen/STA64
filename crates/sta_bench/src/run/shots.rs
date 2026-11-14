//! `Take a screenshot called "NAME"` and `Expect shots "A" and "B" to match
//! [within X]` (docs/TESTBENCH.md, "Frames").
//!
//! The tier-1 run has no renderer, so it only records what a scenario asks
//! for: each screenshot as `(name, tick)`, each comparison as a [`ShotCheck`],
//! and fails a scenario that compares a name it never took. The tier-2 run
//! (`sta-bench run --tier 2`) draws the frames in a `sta64 --headless
//! --frames` process, at the recorded ticks, and [`judge`] settles the steps.
//!
//! "Screenshot" is always the in-app offscreen frame, never the user's screen.

use std::fs;
use std::path::Path;

use sta_core::Tick;

use super::{CaseResult, Status};
use crate::pair::{frame_diff, read_png};

/// A `Take a screenshot` step: the frame at `tick`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shot {
    pub name: String,
    pub tick: Tick,
    pub line: usize,
}

/// An `Expect shots` step: the mean absolute difference (0..255, over every
/// channel) the two frames may have; 0 is "identical".
#[derive(Debug, Clone, PartialEq)]
pub struct ShotCheck {
    pub a: String,
    pub b: String,
    pub within: f64,
    pub line: usize,
}

/// The file a frame is written to: `frames/<name>.png` in the case's report directory.
pub fn file_name(name: &str) -> String {
    format!("{name}.png")
}

/// A `Take a screenshot` step, at the tier-1 run.
pub(crate) fn record(shots: &mut Vec<Shot>, name: &str, line: usize, tick: Tick) -> (Status, String) {
    if !crate::parse::scenario::is_shot_name(name) {
        return (Status::Fail, format!("`{name}`: a frame name is letters, digits, `-`, `_` and `.` only (at most 64)"));
    }
    if shots.iter().any(|s| s.name.eq_ignore_ascii_case(name)) {
        return (Status::Fail, format!("a screenshot called `{name}` was already taken"));
    }
    shots.push(Shot { name: name.to_owned(), tick, line });
    (Status::Skip, format!("frame at tick {tick}: drawn at tier 2 (--tier 2)"))
}

/// An `Expect shots` step, at the tier-1 run.
pub(crate) fn expect(
    shots: &[Shot],
    checks: &mut Vec<ShotCheck>,
    (a, b, within): (&str, &str, Option<f64>),
    line: usize,
) -> (Status, String) {
    for n in [a, b] {
        if !shots.iter().any(|s| s.name.eq_ignore_ascii_case(n)) {
            return (Status::Fail, format!("no screenshot called `{n}` was taken before this step"));
        }
    }
    let within = within.unwrap_or(0.0);
    if within < 0.0 {
        return (Status::Fail, "`within` is a mean difference, not below 0".into());
    }
    checks.push(ShotCheck { a: a.to_owned(), b: b.to_owned(), within, line });
    (Status::Skip, "compared at tier 2 (--tier 2)".into())
}

fn shot<'a>(case: &'a CaseResult, name: &str) -> Option<&'a Shot> {
    case.shots.iter().find(|s| s.name.eq_ignore_ascii_case(name))
}

/// The file the process wrote for `tick`, in `--frame-dir`.
pub fn tick_file(tick: Tick) -> String {
    format!("tick-{tick:06}.png")
}

fn settle(case: &mut CaseResult, line: usize, status: Status, detail: String) {
    if let Some(s) = case.steps.iter_mut().find(|s| s.line == line && s.status == Status::Skip) {
        s.status = status;
        s.detail = detail;
    }
    if status == Status::Fail {
        case.passed = false;
    }
}

/// Settle the screenshot and comparison steps of `case` from the frames a
/// `--frames` run left in `dir` (`tick-NNNNNN.png`). Each screenshot is also
/// copied to `<name>.png` there. Steps that cannot be settled stay skipped.
pub fn judge(case: &mut CaseResult, dir: &Path) {
    for s in case.shots.clone() {
        let from = dir.join(tick_file(s.tick));
        let to = dir.join(file_name(&s.name));
        match fs::copy(&from, &to) {
            Ok(_) => settle(case, s.line, Status::Pass, format!("{} (tick {})", file_name(&s.name), s.tick)),
            Err(e) => settle(case, s.line, Status::Fail, format!("{}: {e}", from.display())),
        }
    }
    for c in case.shot_checks.clone() {
        let (Some(a), Some(b)) = (shot(case, &c.a), shot(case, &c.b)) else { continue };
        let read = |s: &Shot| read_png(&dir.join(file_name(&s.name)));
        let (status, detail) = match (read(a), read(b)) {
            (Ok(x), Ok(y)) => {
                let d = frame_diff(&x, &y);
                let text = format!(
                    "\"{}\" (tick {}) and \"{}\" (tick {}): mean {:.3}, largest {}, {:.2}% of pixels differ, within {}",
                    c.a,
                    a.tick,
                    c.b,
                    b.tick,
                    d.mean,
                    d.max,
                    d.share * 100.0,
                    c.within
                );
                (if d.mean <= c.within { Status::Pass } else { Status::Fail }, text)
            }
            (Err(e), _) | (_, Err(e)) => (Status::Fail, e),
        };
        settle(case, c.line, status, detail);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pair::{Image, write_png};
    use crate::parse::Scenario;
    use crate::run::{self, RunConfig};
    use std::sync::Arc;

    fn flat(v: u8) -> Image {
        Image { w: 8, h: 4, rgb: vec![v; 8 * 4 * 3] }
    }

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("sta-shots-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    const SCENARIO: &str = "# t\n\nScene: planet\n\n\
        1. Take a screenshot called \"a\"\n\
        2. Run 10 ticks\n\
        3. Take a screenshot called \"b\"\n\
        4. Run 10 ticks\n\
        5. Take a screenshot called \"c\"\n\
        6. Expect shots \"a\" and \"b\" to match\n\
        7. Expect shots \"a\" and \"c\" to match within 3\n\
        8. Expect shots \"a\" and \"c\" to match within 1\n";

    /// Authored frames stand in for what `sta64 --frames` writes: tick 0 and 10 equal,
    /// tick 20 two levels brighter (mean difference 2).
    #[test]
    fn judge_settles_the_steps_from_the_frames() {
        let sc = Scenario::parse(SCENARIO).unwrap();
        let mut case = run::run_scenario(&Arc::new(RunConfig::fake()), &sc, "t");
        assert!(case.passed, "{}", case.summary());
        assert_eq!(case.shots.len(), 3);
        let d = dir("judge");
        for (tick, v) in [(0, 100), (10, 100), (20, 102)] {
            write_png(&d.join(tick_file(tick)), &flat(v)).unwrap();
        }
        judge(&mut case, &d);
        let st: Vec<Status> = case.steps.iter().map(|s| s.status).collect();
        use Status::{Fail, Pass};
        assert_eq!(st[0], Pass);
        assert_eq!(st[1], Pass, "a Run step is not a shot");
        assert_eq!(&st[5..], [Pass, Pass, Fail], "{}", case.summary());
        assert!(!case.passed);
        assert!(d.join("c.png").is_file());
        assert!(case.steps[7].detail.contains("mean 2.000"), "{}", case.summary());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_missing_frame_fails_its_step() {
        let sc = Scenario::parse(SCENARIO).unwrap();
        let mut case = run::run_scenario(&Arc::new(RunConfig::fake()), &sc, "t");
        let d = dir("missing");
        write_png(&d.join(tick_file(0)), &flat(1)).unwrap();
        judge(&mut case, &d);
        assert_eq!(case.steps[2].status, Status::Fail);
        assert!(!case.passed);
        let _ = fs::remove_dir_all(&d);
    }
}
