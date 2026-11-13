//! The ODF formatter command (EDT-06), over the lossless syntax tree of
//! `sta_odf::cst`.
//!
//! ```text
//! odf_fmt [--grouped] [--groups FILE] [--write | --check] [--report FILE] PATH...
//! ```
//!
//! PATH is an `.odf` file or a folder searched recursively (symlinks are
//! skipped). Without `--write` it is a dry run that lists the files that
//! would change. Each file keeps its own line endings and encoding; only
//! whitespace is touched, and with `--grouped` the entries are also gathered
//! by the key-group table (the built-in one, or `--groups FILE`, see
//! `sta_odf/src/cst/fo_groups.txt` for its format).
//!
//! `--check` writes nothing and verifies, for every file, that the syntax
//! tree rebuilds it byte for byte, that formatting (and grouping, with
//! `--grouped`) is idempotent and keeps the includes, the keys and the
//! values the reader sees (in order for whitespace formatting, as a set for
//! the grouped layout) and adds no line-ending kind the file did not have.
//! Failures name the file and, for a value difference, the key; never
//! content.
//!
//! `--report FILE` writes a JSON summary. Exit status: 0 nothing to change,
//! all written or all checks passed; 1 files would change (dry run) or a
//! check failed; 2 usage or I/O error.

use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use sta_odf::{parse, Cst, GroupTable};

use crate::fsutil::{collect, json_str, write_atomically};

const USAGE: &str = "Usage: odf_fmt [--grouped] [--groups FILE] [--write | --check] [--report FILE] PATH...\n       odf_fmt --help";

/// What formatting does to a file.
#[derive(Clone, Copy)]
pub enum Layout<'a> {
    /// Whitespace only.
    Plain,
    /// Whitespace, then entries gathered by group.
    Grouped(&'a GroupTable),
}

impl Layout<'_> {
    pub fn apply(&self, cst: &Cst) -> Vec<u8> {
        match self {
            Layout::Plain => cst.format(),
            Layout::Grouped(t) => cst.format_grouped(t),
        }
    }
}

fn eol_kinds(b: &[u8]) -> (bool, bool, bool) {
    let (mut crlf, mut lf, mut cr) = (false, false, false);
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'\n' if i > 0 && b[i - 1] == b'\r' => crlf = true,
            b'\n' => lf = true,
            b'\r' if b.get(i + 1) != Some(&b'\n') => cr = true,
            _ => {}
        }
    }
    (crlf, lf, cr)
}

/// Everything that goes wrong when `bytes` is formatted in `layout`. Empty
/// when the file round-trips, formats idempotently and reads the same.
/// Messages never contain file content (key names only).
pub fn check_bytes(bytes: &[u8], layout: Layout) -> Vec<String> {
    let mut problems = Vec::new();
    let cst = Cst::parse(bytes);
    if cst.to_bytes() != bytes {
        problems.push("syntax tree is not lossless".to_owned());
        return problems;
    }
    let out = layout.apply(&cst);
    if layout.apply(&Cst::parse(&out)) != out {
        problems.push("format is not idempotent".to_owned());
    }
    let (a, b) = (
        parse(&String::from_utf8_lossy(bytes)),
        parse(&String::from_utf8_lossy(&out)),
    );
    if a.includes != b.includes {
        problems.push("includes differ after formatting".to_owned());
    }
    if cst.includes() != a.includes {
        problems.push("includes differ from the reader's".to_owned());
    }
    let kv = |r: &sta_odf::RawOdf| -> Vec<(String, String)> {
        r.entries.iter().map(|e| (e.key.clone(), e.value.clone())).collect()
    };
    let (mut ka, mut kb) = (kv(&a), kv(&b));
    let repeated = {
        let mut keys: Vec<String> = ka.iter().map(|(k, _)| k.to_ascii_lowercase()).collect();
        keys.sort();
        keys.windows(2).any(|w| w[0] == w[1])
    };
    // Order is part of the meaning for whitespace formatting and for a file
    // with a repeated key (the last one wins); a grouped file without
    // repeats is compared as a set.
    if matches!(layout, Layout::Grouped(_)) && !repeated {
        ka.sort();
        kb.sort();
    }
    if ka.len() != kb.len() {
        problems.push(format!("{} entries before, {} after", ka.len(), kb.len()));
    } else if let Some(i) = (0..ka.len()).find(|&i| ka[i] != kb[i]) {
        problems.push(format!("entry differs at key {}", json_str(&ka[i].0)));
    }
    // No line may be lost, duplicated or edited beyond whitespace.
    let lines = |b: &[u8]| String::from_utf8_lossy(b).split(['\n', '\r']).filter(|l| !l.trim().is_empty()).count();
    if matches!(layout, Layout::Grouped(_)) && lines(bytes) != lines(&out) {
        problems.push("the number of non-blank lines changed".to_owned());
    }
    let (ia, ib) = (eol_kinds(bytes), eol_kinds(&out));
    if (ib.0 && !ia.0) || (ib.1 && !ia.1) || (ib.2 && !ia.2) {
        problems.push("a line-ending kind appeared that the file did not have".to_owned());
    }
    problems
}

struct Opts {
    write: bool,
    check: bool,
    grouped: bool,
    groups: Option<PathBuf>,
    report: Option<PathBuf>,
    paths: Vec<PathBuf>,
}

fn parse_args(args: &[OsString]) -> Result<Option<Opts>, String> {
    let mut o = Opts { write: false, check: false, grouped: false, groups: None, report: None, paths: Vec::new() };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.to_str() {
            Some("--help") => return Ok(None),
            Some("--write") => o.write = true,
            Some("--check") => o.check = true,
            Some("--grouped") => o.grouped = true,
            Some("--groups") => {
                o.groups = Some(it.next().ok_or("--groups needs a file")?.into());
                o.grouped = true;
            }
            Some("--report") => o.report = Some(it.next().ok_or("--report needs a file")?.into()),
            Some(s) if s.starts_with("--") => return Err(format!("unknown option {s}")),
            _ => o.paths.push(a.into()),
        }
    }
    if o.write && o.check {
        return Err("--write and --check exclude each other".into());
    }
    if o.paths.is_empty() {
        return Err("expected at least one path".into());
    }
    Ok(Some(o))
}

/// Run the command with `args` (without the program name). Returns the exit
/// status. Output goes to `out`, problems to `err`.
pub fn run(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match run_inner(args, out, err) {
        Ok(code) => code,
        Err(e) => {
            let _ = writeln!(err, "odf_fmt: {e}\n{USAGE}");
            2
        }
    }
}

fn run_inner(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> Result<i32, String> {
    let Some(o) = parse_args(args)? else {
        let _ = writeln!(out, "{USAGE}");
        return Ok(0);
    };
    let custom;
    let table = match &o.groups {
        Some(p) => {
            let text = fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
            custom = GroupTable::parse(&text).map_err(|e| format!("{}: {e}", p.display()))?;
            &custom
        }
        None => GroupTable::fo(),
    };
    let layout = if o.grouped { Layout::Grouped(table) } else { Layout::Plain };

    let (mut files, mut skipped) = (Vec::new(), Vec::new());
    for p in &o.paths {
        if fs::symlink_metadata(p).is_ok_and(|m| m.is_file()) {
            files.push(p.clone());
        } else {
            collect(p, "odf", &mut files, &mut skipped).map_err(|e| format!("{}: {e}", p.display()))?;
        }
    }
    for s in &skipped {
        let _ = writeln!(err, "odf_fmt: skipping symlink {}", s.display());
    }
    let (mut changed, mut errors, mut failed) = (Vec::<PathBuf>::new(), Vec::<(PathBuf, String)>::new(), Vec::<(PathBuf, Vec<String>)>::new());
    for f in &files {
        let bytes = match fs::read(f) {
            Ok(b) => b,
            Err(e) => {
                errors.push((f.clone(), e.to_string()));
                continue;
            }
        };
        if o.check {
            // Plain formatting is always checked, the grouped layout on top.
            let mut problems = check_bytes(&bytes, Layout::Plain);
            if o.grouped {
                problems.extend(check_bytes(&bytes, layout).into_iter().map(|p| format!("grouped: {p}")));
            }
            if !problems.is_empty() {
                failed.push((f.clone(), problems));
            }
            if layout.apply(&Cst::parse(&bytes)) != bytes {
                changed.push(f.clone());
            }
            continue;
        }
        let cst = Cst::parse(&bytes);
        // Never write from a tree that cannot rebuild the file.
        if cst.to_bytes() != bytes {
            errors.push((f.clone(), "syntax tree is not lossless".into()));
            continue;
        }
        let formatted = layout.apply(&cst);
        if formatted == bytes {
            continue;
        }
        if o.write {
            if let Err(e) = write_atomically(f, &formatted) {
                errors.push((f.clone(), e.to_string()));
                continue;
            }
        } else {
            let _ = writeln!(out, "would change: {}", f.display());
        }
        changed.push(f.clone());
    }
    for (f, e) in &errors {
        let _ = writeln!(err, "odf_fmt: {}: {e}", f.display());
    }
    for (f, ps) in &failed {
        for p in ps {
            let _ = writeln!(err, "odf_fmt: {}: {p}", f.display());
        }
    }
    let verb = if o.check { "would be reformatted" } else if o.write { "rewritten" } else { "would change" };
    let _ = writeln!(
        out,
        "{} files, {} {verb}, {} errors{}",
        files.len(),
        changed.len(),
        errors.len(),
        if o.check { format!(", {} failed the check", failed.len()) } else { String::new() }
    );
    if let Some(r) = &o.report {
        let list = |v: &[PathBuf]| v.iter().map(|p| json_str(&p.to_string_lossy())).collect::<Vec<_>>().join(",");
        let errs = errors
            .iter()
            .map(|(p, e)| format!("{{\"file\":{},\"error\":{}}}", json_str(&p.to_string_lossy()), json_str(e)))
            .collect::<Vec<_>>()
            .join(",");
        let fails = failed
            .iter()
            .map(|(p, ps)| {
                let ps = ps.iter().map(|s| json_str(s)).collect::<Vec<_>>().join(",");
                format!("{{\"file\":{},\"problems\":[{ps}]}}", json_str(&p.to_string_lossy()))
            })
            .collect::<Vec<_>>()
            .join(",");
        let json = format!(
            "{{\"files\":{},\"mode\":{},\"grouped\":{},\"changed\":[{}],\"errors\":[{errs}],\"failed\":[{fails}]}}\n",
            files.len(),
            json_str(if o.check { "check" } else if o.write { "write" } else { "dry-run" }),
            o.grouped,
            list(&changed)
        );
        write_report(r, &json).map_err(|e| format!("{}: {e}", r.display()))?;
    }
    Ok(if !errors.is_empty() {
        2
    } else if !failed.is_empty() || (!o.write && !o.check && !changed.is_empty()) {
        1
    } else {
        0
    })
}

fn write_report(path: &Path, json: &str) -> std::io::Result<()> {
    fs::write(path, json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("odf_fmt-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn go(args: &[&str]) -> (i32, String, String) {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let (mut o, mut e) = (Vec::new(), Vec::new());
        let c = run(&args, &mut o, &mut e);
        (c, String::from_utf8(o).unwrap(), String::from_utf8(e).unwrap())
    }

    const CRLF: &[u8] = b"// c\r\nweapon1 =  \"a\"  \r\n\r\n\r\nclassLabel=\"x\"\r\nzzz = 1\r\n";

    #[test]
    fn dry_run_lists_and_changes_nothing_then_write_keeps_crlf() {
        let d = scratch("write");
        let f = d.join("a.odf");
        fs::write(&f, CRLF).unwrap();
        let (c, out, _) = go(&[d.to_str().unwrap()]);
        assert_eq!(c, 1);
        assert!(out.contains("would change") && out.contains("1 files, 1 would change"));
        assert_eq!(fs::read(&f).unwrap(), CRLF);
        let (c, _, _) = go(&["--write", d.to_str().unwrap()]);
        assert_eq!(c, 0);
        let after = fs::read(&f).unwrap();
        assert_eq!(after, b"// c\r\nweapon1 = \"a\"\r\n\r\nclassLabel = \"x\"\r\nzzz = 1\r\n");
        // Second run: nothing to do.
        assert_eq!(go(&[d.to_str().unwrap()]).0, 0);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn grouped_write_is_idempotent_and_keeps_line_endings() {
        let d = scratch("grouped");
        let f = d.join("a.odf");
        fs::write(&f, CRLF).unwrap();
        assert_eq!(go(&["--grouped", "--write", f.to_str().unwrap()]).0, 0);
        let after = fs::read(&f).unwrap();
        assert_eq!(after, b"// c\r\nclassLabel = \"x\"\r\n\r\nweapon1 = \"a\"\r\n\r\nzzz = 1\r\n");
        assert_eq!(go(&["--grouped", f.to_str().unwrap()]).0, 0);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn check_passes_clean_files_and_reports_the_report_json() {
        let d = scratch("check");
        fs::write(d.join("a.odf"), CRLF).unwrap();
        fs::write(d.join("b.odf"), b"a = \xe9\nclassLabel = 1 // x\nweapon1 = 2").unwrap();
        let rep = d.join("r.json");
        let (c, out, err) = go(&["--check", "--grouped", "--report", rep.to_str().unwrap(), d.to_str().unwrap()]);
        assert_eq!((c, err.as_str()), (0, ""), "{out}");
        assert!(out.contains("2 files") && out.contains("0 failed the check"));
        let json = fs::read_to_string(&rep).unwrap();
        assert!(json.contains("\"mode\":\"check\"") && json.contains("\"failed\":[]"));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn check_bytes_names_no_content() {
        for src in [CRLF, b"a = 1\rb = 2\n", b"x = \"secret\"\n\"more\"\n", b""] {
            for l in [Layout::Plain, Layout::Grouped(GroupTable::fo())] {
                assert_eq!(check_bytes(src, l), Vec::<String>::new());
            }
        }
    }

    #[test]
    fn usage_errors_exit_2() {
        assert_eq!(go(&[]).0, 2);
        assert_eq!(go(&["--nope", "x"]).0, 2);
        assert_eq!(go(&["--write", "--check", "x"]).0, 2);
        assert_eq!(go(&["--write", "/nonexistent-odf-fmt-path"]).0, 2);
        assert_eq!(go(&["--help"]).0, 0);
    }

    #[test]
    fn custom_group_table_is_read() {
        let d = scratch("table");
        fs::write(d.join("t.txt"), "[first]\nzzz\n[second]\nclassLabel\n").unwrap();
        let f = d.join("a.odf");
        fs::write(&f, b"classLabel = 1\nzzz = 2\n").unwrap();
        let t = d.join("t.txt");
        assert_eq!(go(&["--groups", t.to_str().unwrap(), "--write", f.to_str().unwrap()]).0, 0);
        assert_eq!(fs::read(&f).unwrap(), b"zzz = 2\n\nclassLabel = 1\n");
        fs::remove_dir_all(&d).unwrap();
    }
}
