//! Read-only inspection of effective ODF-to-ODF references.
//! `cargo run -p sta_content --example odf_references -- <root> <odf> [--mod <name> | --loose-root]`

use std::fmt::Write;
use std::path::{Path, PathBuf};

use sta_content::OdfIndex;
use sta_odf::Odf;
use sta_vfs::Vfs;

const USAGE: &str = "usage: odf_references <root> <odf> [--mod <name> | --loose-root]";

#[derive(Debug, PartialEq, Eq)]
struct Failure {
    code: i32,
    message: String,
}

impl Failure {
    fn usage() -> Self {
        Self {
            code: 2,
            message: USAGE.into(),
        }
    }

    fn operation(message: impl Into<String>) -> Self {
        Self {
            code: 1,
            message: message.into(),
        }
    }
}

enum Mode {
    Install(Option<String>),
    Loose,
}

struct Request {
    root: PathBuf,
    odf: String,
    mode: Mode,
}

fn parse(args: &[String]) -> Result<Option<Request>, Failure> {
    if args.len() == 1 && args[0] == "--help" {
        return Ok(None);
    }
    let [root, odf, tail @ ..] = args else {
        return Err(Failure::usage());
    };
    if root.is_empty() || odf.is_empty() || root.starts_with("--") || odf.starts_with("--") {
        return Err(Failure::usage());
    }
    let mode = match tail {
        [] => Mode::Install(None),
        [flag] if flag == "--loose-root" => Mode::Loose,
        [flag, name] if flag == "--mod" && !name.is_empty() && !name.starts_with("--") => {
            Mode::Install(Some(name.clone()))
        }
        _ => return Err(Failure::usage()),
    };
    Ok(Some(Request {
        root: PathBuf::from(root),
        odf: odf.clone(),
        mode,
    }))
}

fn escape(value: &str) -> String {
    value.chars().flat_map(char::escape_default).collect()
}

/// The same u32-suffix test and value cleanup used by the content unit reader.
fn numbered(odf: &Odf, prefix: &str, kind: u8) -> Vec<(u8, u32, String, String)> {
    odf.keys()
        .filter_map(|key| {
            let canonical = key.to_ascii_lowercase();
            let number = canonical.strip_prefix(prefix)?.parse::<u32>().ok()?;
            let reference = odf.raw(key)?.trim().trim_matches('"').trim();
            if reference.is_empty() || reference.eq_ignore_ascii_case("null") {
                return None;
            }
            Some((kind, number, canonical, reference.to_owned()))
        })
        .collect()
}

fn references(odf: &Odf) -> Vec<(String, String)> {
    let mut links = numbered(odf, "weapon", 0);
    links.extend(numbered(odf, "builditem", 1));
    if let Some(reference) = odf.str("ordName").map(str::trim).filter(|value| !value.is_empty()) {
        links.push((2, 0, "ordname".into(), reference.into()));
    }
    links.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));
    links
        .into_iter()
        .map(|(_, _, key, reference)| (key, reference))
        .collect()
}

fn inspect(vfs: &Vfs, requested: &str) -> Result<String, Failure> {
    let (index, warnings) = OdfIndex::build(vfs);
    let mut report = String::new();
    for warning in warnings {
        writeln!(
            report,
            "warning\tfile={}\tmessage={}",
            escape(&warning.file),
            escape(&warning.message)
        )
        .unwrap();
    }
    let source = index
        .get(requested)
        .ok_or_else(|| Failure::operation(format!("{report}ODF not indexed: {}", escape(requested))))?;
    // Use the index winner's filename verbatim: trimming a caller's basename
    // could resolve a different file and disagree with the reported source.
    let selected = source.path.rsplit('/').next().unwrap_or(&source.path);
    let resolved =
        sta_odf::resolve_layered(selected, &mut |name, version| index.load(vfs, name, version)).map_err(|error| {
            Failure::operation(format!(
                "{report}ODF {}: {}",
                escape(requested),
                escape(&error.to_string())
            ))
        })?;
    let layer = vfs.layers()[source.layer].name();
    writeln!(
        report,
        "source\todf={}\tlayer={}\tpath={}",
        escape(requested),
        escape(layer),
        escape(&source.path)
    )
    .unwrap();
    for (key, reference) in references(&resolved) {
        if let Some(target) = index.get(&reference) {
            writeln!(
                report,
                "link\tkey={}\tref={}\tstatus=indexed\tlayer={}\tpath={}",
                escape(&key),
                escape(&reference),
                escape(vfs.layers()[target.layer].name()),
                escape(&target.path)
            )
            .unwrap();
        } else {
            writeln!(
                report,
                "link\tkey={}\tref={}\tstatus=missing",
                escape(&key),
                escape(&reference)
            )
            .unwrap();
        }
    }
    Ok(report)
}

fn run(args: &[String]) -> Result<String, Failure> {
    let Some(request) = parse(args)? else {
        return Ok(format!("{USAGE}\n"));
    };
    let vfs = match request.mode {
        Mode::Install(name) => Vfs::mount_install(Path::new(&request.root), name.as_deref()),
        Mode::Loose => {
            let mut vfs = Vfs::default();
            vfs.push_dir("root".into(), request.root.clone()).map(|()| vfs)
        }
    }
    .map_err(|error| {
        Failure::operation(format!(
            "{}: {}",
            escape(&request.root.to_string_lossy()),
            escape(&error.to_string())
        ))
    })?;
    inspect(&vfs, &request.odf)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(report) => print!("{report}"),
        Err(error) => {
            eprintln!("{}", error.message);
            std::process::exit(error.code);
        }
    }
}

#[cfg(test)]
#[path = "odf_references/tests.rs"]
mod tests;
