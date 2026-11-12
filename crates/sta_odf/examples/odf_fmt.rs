//! `odf_fmt`: format ODF files with the lossless syntax tree (EDT-06).
//!
//! This is the library half's stand-in for `crates/sta_tools/src/bin/odf_fmt.rs`:
//! it moves there, unchanged, when EDT-01 creates the `sta_tools` crate.
//!
//!     cargo run -p sta_odf --example odf_fmt -- [--write] [--report FILE] PATH...
//!
//! PATH is an `.odf` file or a folder searched recursively. Default is a dry
//! run that lists the files that would change; `--write` rewrites them
//! atomically (temp file in the same folder, then rename). `--report FILE`
//! writes a JSON summary. Exit status: 0 nothing to change or all written,
//! 1 files would change (dry run), 2 usage or I/O error.

use std::fs;
use std::path::{Path, PathBuf};

use sta_odf::Cst;

const USAGE: &str = "Usage: odf_fmt [--write] [--report FILE] PATH...\n       odf_fmt --help";

fn collect(path: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        // Never follow links: a loop would not end, and a rewrite would
        // replace the link instead of its target.
        eprintln!("odf_fmt: skipping symlink {}", path.display());
    } else if meta.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(path)?.collect::<Result<_, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            collect(&e.path(), out)?;
        }
    } else if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("odf"))
    {
        out.push(path.to_owned());
    }
    Ok(())
}

/// Temp file beside the target (new, never a followed link), same
/// permissions, flushed to disk, then renamed over it.
fn write_atomically(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = target.with_extension("odf.fmt-tmp");
    let result = (|| {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.set_permissions(fs::metadata(target)?.permissions())?;
        f.sync_all()?;
        fs::rename(&tmp, target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn run() -> Result<i32, String> {
    let (mut write, mut report, mut paths) = (false, None, Vec::new());
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--help" => {
                println!("{USAGE}");
                return Ok(0);
            }
            "--write" => write = true,
            "--report" => report = Some(args.next().ok_or("--report needs a file")?),
            _ if a.starts_with("--") => return Err(format!("unknown option {a}")),
            _ => paths.push(PathBuf::from(a)),
        }
    }
    if paths.is_empty() {
        return Err("expected at least one path".into());
    }
    let mut files = Vec::new();
    for p in &paths {
        if fs::symlink_metadata(p).is_ok_and(|m| m.is_file()) {
            files.push(p.clone());
        } else {
            collect(p, &mut files).map_err(|e| format!("{}: {e}", p.display()))?;
        }
    }
    let (mut changed, mut errors) = (Vec::new(), Vec::new());
    for f in &files {
        let bytes = match fs::read(f) {
            Ok(b) => b,
            Err(e) => {
                errors.push((f.clone(), e.to_string()));
                continue;
            }
        };
        let cst = Cst::parse(&bytes);
        // Never write from a tree that cannot rebuild the file.
        if cst.to_bytes() != bytes {
            errors.push((f.clone(), "syntax tree is not lossless".into()));
            continue;
        }
        let out = cst.format();
        if out == bytes {
            continue;
        }
        if write {
            if let Err(e) = write_atomically(f, &out) {
                errors.push((f.clone(), e.to_string()));
                continue;
            }
        } else {
            println!("would change: {}", f.display());
        }
        changed.push(f.clone());
    }
    for (f, e) in &errors {
        eprintln!("odf_fmt: {}: {e}", f.display());
    }
    println!(
        "{} files, {} {}, {} errors",
        files.len(),
        changed.len(),
        if write { "rewritten" } else { "would change" },
        errors.len()
    );
    if let Some(r) = report {
        let list = |v: &[PathBuf]| {
            v.iter()
                .map(|p| json_str(&p.to_string_lossy()))
                .collect::<Vec<_>>()
                .join(",")
        };
        let errs = errors
            .iter()
            .map(|(p, e)| {
                format!(
                    "{{\"file\":{},\"error\":{}}}",
                    json_str(&p.to_string_lossy()),
                    json_str(e)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let json = format!(
            "{{\"files\":{},\"write\":{write},\"changed\":[{}],\"errors\":[{errs}]}}\n",
            files.len(),
            list(&changed)
        );
        fs::write(&r, json).map_err(|e| format!("{r}: {e}"))?;
    }
    Ok(if !errors.is_empty() {
        2
    } else if !write && !changed.is_empty() {
        1
    } else {
        0
    })
}

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("odf_fmt: {e}\n{USAGE}");
            std::process::exit(2);
        }
    }
}
