//! `diff <a> <b>`: two report directories, case by case: digests per tick,
//! marks, objects by name, frames byte for byte (docs/TESTBENCH.md,
//! "Comparing with Refit"). Exit 1 when anything differs.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::{Fail, Opts, Outcome};

/// A report directory, or `latest`-style link to one.
fn report(arg: &str) -> Result<PathBuf, Fail> {
    let p = PathBuf::from(arg);
    if p.join("results.json").is_file() {
        Ok(p)
    } else {
        Err(Fail::Error(format!("{arg}: not a report directory (no results.json)")))
    }
}

fn cases(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    v.sort();
    v
}

fn text(dir: &Path, case: &str, file: &str) -> Option<String> {
    fs::read_to_string(dir.join(case).join(file)).ok()
}

/// The first line where two texts differ.
fn first_diff(a: &str, b: &str) -> Option<String> {
    let (mut la, mut lb) = (a.lines(), b.lines());
    let mut n = 0;
    loop {
        n += 1;
        match (la.next(), lb.next()) {
            (None, None) => return None,
            (x, y) if x == y => {}
            (x, y) => return Some(format!("line {n}: `{}` vs `{}`", x.unwrap_or("<end>"), y.unwrap_or("<end>"))),
        }
    }
}

/// `objects.jsonl` lines by `"name"`, header line skipped.
fn by_name(t: &str) -> BTreeMap<String, &str> {
    t.lines()
        .skip(1)
        .filter_map(|l| {
            let at = l.find("\"name\":\"")? + 8;
            Some((l[at..at + l[at..].find('"')?].to_owned(), l))
        })
        .collect()
}

fn case_diff(a: &Path, b: &Path, case: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (file, what) in [("digests", "digests"), ("marks", "marks")] {
        match (text(a, case, file), text(b, case, file)) {
            (Some(x), Some(y)) => {
                if let Some(d) = first_diff(&x, &y) {
                    out.push(format!("{what}: {d}"));
                }
            }
            (None, None) => {}
            (x, _) => out.push(format!("{what}: only in {}", if x.is_some() { "A" } else { "B" })),
        }
    }
    if let (Some(x), Some(y)) = (text(a, case, "objects.jsonl"), text(b, case, "objects.jsonl")) {
        let (ma, mb) = (by_name(&x), by_name(&y));
        for (n, la) in &ma {
            match mb.get(n) {
                None => out.push(format!("objects: `{n}` only in A")),
                Some(lb) if lb != la => out.push(format!("objects: `{n}` differs\n    A {la}\n    B {lb}")),
                _ => {}
            }
        }
        out.extend(mb.keys().filter(|n| !ma.contains_key(*n)).map(|n| format!("objects: `{n}` only in B")));
    }
    let frames = |d: &Path| -> BTreeMap<String, PathBuf> {
        fs::read_dir(d.join(case).join("frames"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
            .collect()
    };
    let (fa, fb) = (frames(a), frames(b));
    for (n, pa) in &fa {
        match fb.get(n) {
            None => out.push(format!("frames: {n} only in A")),
            Some(pb) if fs::read(pa).ok() != fs::read(pb).ok() => out.push(format!("frames: {n} differs")),
            _ => {}
        }
    }
    out.extend(fb.keys().filter(|n| !fa.contains_key(*n)).map(|n| format!("frames: {n} only in B")));
    out
}

pub(crate) fn diff(o: &Opts) -> Outcome {
    let [a, b] = o.rest.as_slice() else {
        return Err(Fail::Usage("diff <report-a> <report-b>".into()));
    };
    let (a, b) = (report(a)?, report(b)?);
    let (ca, cb) = (cases(&a), cases(&b));
    let mut differing = 0;
    for c in ca.iter().filter(|c| cb.contains(c)) {
        let d = case_diff(&a, &b, c);
        if d.is_empty() {
            println!("same  {c}");
        } else {
            differing += 1;
            println!("DIFF  {c}");
            for l in d {
                println!("  {l}");
            }
        }
    }
    for c in ca.iter().filter(|c| !cb.contains(c)) {
        differing += 1;
        println!("DIFF  {c}: only in A");
    }
    for c in cb.iter().filter(|c| !ca.contains(c)) {
        differing += 1;
        println!("DIFF  {c}: only in B");
    }
    println!("{differing} of {} cases differ", ca.iter().chain(cb.iter().filter(|c| !ca.contains(c))).count());
    Ok(i32::from(differing > 0))
}
