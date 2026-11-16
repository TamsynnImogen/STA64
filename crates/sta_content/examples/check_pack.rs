//! Coverage report and `why` for an install or a mod (CON-34, D7). Read-only.
//!
//! ```text
//! cargo run -p sta_content --example check_pack -- <root> [<mod> | --all]
//! cargo run -p sta_content --example check_pack -- <root> <mod> --why <file> [<key>]
//! cargo run -p sta_content --example check_pack -- <root> [<mod>] --keys | --cfg-unread
//! cargo run -p sta_content --example check_pack -- <root> [<mod>] --lookup-changes
//! ```
//!
//! `<root>` is an install (with `Data`). Without a mod the `Data` layer is
//! checked; `--all` checks `Data` and every mod under `Data/Mods`, each
//! mounted by its own rules (`standalone`, `ignoreParentMaps`). `--why`
//! names the layer that supplies a file (an ODF stem, a path, `RTS_CFG.h`)
//! and, with a key, the file and line that set the key.

use std::path::Path;
use std::io::{self, Write};

use sta_content::coverage;
use sta_vfs::Vfs;

fn mount(root: &Path, m: Option<&str>) -> Result<Vfs, String> {
    Vfs::mount_install(root, m).map_err(|e| format!("{}: {e}", m.unwrap_or("Data")))
}

fn write_lookup_audit(out: &mut impl Write, report: &coverage::LookupAudit) -> io::Result<()> {
    for file in &report.changed {
        for (change, source) in &file.changes {
            writeln!(out, "change\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{}\t{}",
                file.file, file.layer, change.key, change.exact, change.selected,
                source.file, source.layer, change.source.version, source.line)?;
        }
    }
    for (file, error) in &report.errors {
        writeln!(out, "error\t{file:?}\t{error:?}")?;
    }
    writeln!(out, "summary\tfiles={}\tchanged_files={}\tchanged_queries={}\terrors={}",
        report.files, report.changed.len(), report.changed_queries(), report.errors.len())
}

fn run(args: &[String]) -> Result<(), String> {
    let usage = "usage: check_pack <root> [<mod> | --all] [--why <file> [<key>] | --keys | --cfg-unread | --lookup-changes]";
    let Some(root) = args.first().filter(|r| !r.starts_with("--")) else { return Err(usage.into()) };
    let root = Path::new(root);
    let mut rest = &args[1..];
    let mut target: Option<&str> = None;
    let mut all = false;
    if let Some(first) = rest.first() {
        if first == "--all" {
            all = true;
            rest = &rest[1..];
        } else if !first.starts_with("--") {
            target = Some(first);
            rest = &rest[1..];
        }
    }
    match rest {
        [] => {}
        [flag] if flag == "--keys" => {
            // Every ODF key no loader is known to read, most used first.
            for (k, n) in coverage::check(&mount(root, target)?).odf.unknown {
                println!("{k}\t{n}");
            }
            return Ok(());
        }
        [flag] if flag == "--cfg-unread" => {
            // Every own cfg entry no STA64 code reads: file, line, key, value.
            for c in coverage::check(&mount(root, target)?).cfg_unread {
                println!("{}\t{}\t{}\t{}", c.file, c.line, c.key, c.value);
            }
            return Ok(());
        }
        [flag] if flag == "--lookup-changes" => {
            if all {
                return Err("--lookup-changes with --all is unsupported; run once per mod".into());
            }
            let vfs = mount(root, target)?;
            let report = coverage::lookup_change_audit(&vfs, sta_content::Profile::detect(&vfs));
            write_lookup_audit(&mut io::stdout().lock(), &report).map_err(|e| e.to_string())?;
            if !report.errors.is_empty() {
                return Err(format!("{} ODF resolution error(s)", report.errors.len()));
            }
            return Ok(());
        }
        [flag, file] if flag == "--why" => {
            print!("{}", coverage::why_text(&mount(root, target)?, file, None));
            return Ok(());
        }
        [flag, file, key] if flag == "--why" => {
            print!("{}", coverage::why_text(&mount(root, target)?, file, Some(key)));
            return Ok(());
        }
        _ => return Err(usage.into()),
    }
    if all {
        let data = std::fs::read_dir(root)
            .map_err(|e| e.to_string())?
            .flatten()
            .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case("data"))
            .ok_or("no Data folder")?
            .path();
        let (mods, notes) = sta_vfs::mods::list_mods(&data).map_err(|e| e.to_string())?;
        println!("== Data");
        print!("{}", coverage::check(&mount(root, None)?));
        for m in &mods {
            println!("\n== {}", m.folder);
            match mount(root, Some(&m.folder)) {
                Ok(v) => print!("{}", coverage::check(&v)),
                Err(e) => println!("cannot mount: {e}"),
            }
        }
        for n in notes {
            println!("\nnote: {n}");
        }
    } else {
        print!("{}", coverage::check(&mount(root, target)?));
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("{e}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_audit_output_escapes_values_and_counts_errors() {
        let change = sta_odf::LookupChange {
            key: "key".into(), exact: Some("a\tb".into()), selected: "one\ntwo".into(),
            source: sta_odf::KeySource { file: "child.odf".into(), version: 0, line: 2 },
        };
        let report = coverage::LookupAudit {
            files: 2,
            changed: vec![coverage::OdfLookupChanges {
                file: "odf/ships/child.odf".into(), layer: "Data".into(),
                changes: vec![(change, coverage::KeyWhy {
                    file: "odf/ships/child.odf".into(), layer: "Data".into(),
                    line: 2, value: "one\ntwo".into(),
                })],
            }],
            errors: vec![("odf/ships/broken.odf".into(), "missing include".into())],
        };
        let mut out = Vec::new();
        write_lookup_audit(&mut out, &report).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Some(\"a\\tb\")\t\"one\\ntwo\""));
        assert!(text.contains("\"odf/ships/child.odf\"\t\"Data\"\t0\t2\n"));
        assert!(text.contains("error\t\"odf/ships/broken.odf\"\t\"missing include\""));
        assert!(text.ends_with("summary\tfiles=2\tchanged_files=1\tchanged_queries=1\terrors=1\n"));
    }

    #[test]
    fn lookup_audit_rejects_all_mode_before_mounting() {
        let args = ["/nonexistent".to_owned(), "--all".to_owned(), "--lookup-changes".to_owned()];
        assert!(run(&args).unwrap_err().contains("--lookup-changes with --all is unsupported"));
    }
}
