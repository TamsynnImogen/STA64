//! The report directory: `<root>/<run>/{report.md, results.json, <case>/...}`
//! with `latest` pointing at the run (TESTBENCH, "Sessions and reports").

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
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let run = format!("{stamp}-{}", std::process::id());
    let dir = root.join(&run);
    fs::create_dir_all(&dir)?;

    let mut md = String::from(
        "# Bench report\n\n| case | result | ticks | steps (pass/fail/skip) |\n|---|---|---|---|\n",
    );
    let mut js = String::from("{\"run\":");
    js += &json::string(&run);
    js += ",\"cases\":[";
    for (i, c) in cases.iter().enumerate() {
        let n = |s| c.steps.iter().filter(|x| x.status == s).count();
        md += &format!(
            "| {} | {} | {} | {}/{}/{} |\n",
            c.name,
            if c.passed { "pass" } else { "FAIL" },
            c.ticks,
            n(Status::Pass),
            n(Status::Fail),
            n(Status::Skip)
        );
        if i > 0 {
            js.push(',');
        }
        js += &format!(
            "{{\"name\":{},\"title\":{},\"passed\":{},\"ticks\":{},\"error\":{},\"steps\":[",
            json::string(&c.name),
            json::string(&c.title),
            c.passed,
            c.ticks,
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

    let latest = root.join("latest");
    let _ = fs::remove_file(&latest);
    #[cfg(unix)]
    let _ = std::os::unix::fs::symlink(&run, &latest);
    Ok(dir)
}
