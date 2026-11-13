//! The report directory: `<root>/<run>/{report.md, results.json, <case>/...}`
//! with `latest` pointing at the run (TESTBENCH, "Sessions and reports").

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::json;
use super::{CaseResult, Status};

/// `$XDG_DATA_HOME/sta64/bench/results` (or `~/.local/share/...`).
pub fn default_report_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("sta64/bench/results")
}

fn status_word(s: Status) -> &'static str {
    match s {
        Status::Pass => "pass",
        Status::Fail => "fail",
        Status::Skip => "skip",
        Status::Note => "note",
    }
}

/// Write one run under `root/<run>/` and repoint `root/latest`. Returns the run directory.
pub fn write_report(root: &Path, cases: &[CaseResult]) -> io::Result<PathBuf> {
    write_report_timed(root, cases, &Timing::default())
}

/// Wall times of a run, kept out of `results.json` and `report.md` so those
/// diff cleanly between runs; they go to `timing.json` and `index.html`.
#[derive(Default, Clone)]
pub struct Timing {
    /// Case name to wall milliseconds.
    pub cases_ms: BTreeMap<String, u64>,
    pub total_ms: u64,
    pub jobs: usize,
    pub gpu_jobs: usize,
}

/// Escape text for HTML.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn index_html(run: &str, cases: &[&CaseResult], t: &Timing) -> String {
    let bad = cases.iter().filter(|c| !c.passed).count();
    let mut h = format!(
        "<!doctype html><meta charset=utf-8><title>Bench {}</title>\
<style>body{{font:14px sans-serif;margin:1.5em}}td,th{{padding:2px 10px;text-align:left}}\
.fail{{color:#b00020;font-weight:bold}}.pass{{color:#1b7a2f}}pre{{background:#f4f4f4;padding:6px}}</style>\
<h1>Bench report {}</h1><p class={}>{} cases, {} passed, {} failed</p>\
<p>wall {} ms, jobs {}, gpu jobs {} (<a href=report.md>report.md</a>, <a href=results.json>results.json</a>, \
<a href=timing.json>timing.json</a>)</p>\
<table><tr><th>case<th>result<th>ticks<th>final digest<th>ms<th>frames</tr>\n",
        esc(run),
        esc(run),
        if bad == 0 { "pass" } else { "fail" },
        cases.len(),
        cases.len() - bad,
        bad,
        t.total_ms,
        t.jobs,
        t.gpu_jobs
    );
    for c in cases {
        let frames = if c.artifacts.contains_key("frames.txt") { format!("<a href=\"{0}/frames.txt\">hashes</a> <a href=\"{0}/frames/\">png</a>", esc(&c.name)) } else { String::new() };
        h += &format!(
            "<tr><td><a href=\"{0}/log.txt\">{0}</a> {1}<td class={2}>{3}<td>{4}<td>{5}<td>{6}<td>{7}</tr>\n",
            esc(&c.name),
            esc(&c.title),
            if c.passed { "pass" } else { "fail" },
            if c.passed { "pass" } else { "FAIL" },
            c.ticks,
            c.digests.last().map_or(String::new(), |(_, d)| format!("{d:016x}")),
            t.cases_ms.get(&c.name).map_or(String::new(), u64::to_string),
            frames
        );
    }
    h += "</table>\n";
    for c in cases.iter().filter(|c| !c.passed) {
        h += &format!("<h2 id=\"{0}\">{0}</h2><pre>{1}</pre>\n", esc(&c.name), esc(&c.summary()));
    }
    h
}

/// As [`write_report`], with wall times. Cases are written sorted by name so
/// two runs of the same scenarios diff cleanly however they were scheduled.
pub fn write_report_timed(root: &Path, cases: &[CaseResult], timing: &Timing) -> io::Result<PathBuf> {
    let mut sorted: Vec<&CaseResult> = cases.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    let cases = &sorted;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let run = format!("{stamp}-{}", std::process::id());
    let dir = root.join(&run);
    fs::create_dir_all(&dir)?;

    let failed = cases.iter().filter(|c| !c.passed).count();
    let mut md = format!(
        "# Bench report\n\n**{}**: {} cases, {} passed, {} failed\n\n| case | result | ticks | final digest | steps (pass/fail/skip) |\n|---|---|---|---|---|\n",
        if failed == 0 { "PASS" } else { "FAIL" },
        cases.len(),
        cases.len() - failed,
        failed
    );
    let mut js = String::from("{\"run\":");
    js += &json::string(&run);
    js += &format!(",\"passed\":{},\"failed\":{},\"cases\":[", cases.len() - failed, failed);
    for (i, c) in cases.iter().enumerate() {
        let n = |s| c.steps.iter().filter(|x| x.status == s).count();
        md += &format!(
            "| {} | {} | {} | {} | {}/{}/{} |\n",
            c.name,
            if c.passed { "pass" } else { "FAIL" },
            c.ticks,
            c.digests.last().map_or(String::new(), |(_, d)| format!("{d:016x}")),
            n(Status::Pass),
            n(Status::Fail),
            n(Status::Skip)
        );
        if i > 0 {
            js.push(',');
        }
        js += &format!(
            "{{\"name\":{},\"title\":{},\"passed\":{},\"ticks\":{},\"final_digest\":{},\"error\":{},\"steps\":[",
            json::string(&c.name),
            json::string(&c.title),
            c.passed,
            c.ticks,
            c.digests.last().map_or("null".into(), |(_, d)| format!("\"{d:016x}\"")),
            c.error.as_deref().map_or("null".into(), json::string)
        );
        for (k, s) in c.steps.iter().enumerate() {
            if k > 0 {
                js.push(',');
            }
            js += &format!(
                "{{\"line\":{},\"status\":\"{}\",\"text\":{},\"detail\":{}}}",
                s.line,
                status_word(s.status),
                json::string(&s.text),
                json::string(&s.detail)
            );
        }
        js += "]}";

        let cd = dir.join(&c.name);
        fs::create_dir_all(&cd)?;
        fs::write(cd.join("log.txt"), c.summary())?;
        fs::write(cd.join("bench.log"), c.log.join("\n") + "\n")?;
        for (name, body) in &c.artifacts {
            fs::write(cd.join(name), body)?;
        }
        let digests: String = c
            .digests
            .iter()
            .map(|(t, d)| format!("{t} {d:016x}\n"))
            .collect();
        fs::write(cd.join("digests"), digests)?;
    }
    js += "]}\n";
    for c in cases.iter().filter(|c| !c.passed) {
        md += &format!("\n## {}\n\n```\n{}```\n", c.name, c.summary());
    }
    fs::write(dir.join("report.md"), md)?;
    fs::write(dir.join("results.json"), js)?;
    fs::write(dir.join("index.html"), index_html(&run, cases, timing))?;
    let mut tj = format!("{{\"total_ms\":{},\"jobs\":{},\"gpu_jobs\":{},\"cases_ms\":{{", timing.total_ms, timing.jobs, timing.gpu_jobs);
    tj += &timing.cases_ms.iter().map(|(k, v)| format!("{}:{v}", json::string(k))).collect::<Vec<_>>().join(",");
    tj += "}}\n";
    fs::write(dir.join("timing.json"), tj)?;

    let latest = root.join("latest");
    let _ = fs::remove_file(&latest);
    #[cfg(unix)]
    let _ = std::os::unix::fs::symlink(&run, &latest);
    Ok(dir)
}
