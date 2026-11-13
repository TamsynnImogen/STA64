//! Prints one JSON line per campaign mission: did its script resolve, load and
//! run, and which API names it hit were unbound or unsupported
//! (`sta_mission::readiness`). Driven by `tools/campaign_readiness.py`.
//!
//! `campaign_readiness --a1 <dir> --a2 <dir> --roots <dir> [--ticks N]`
//! Each root is optional; an install with campaigns is probed once per line
//! labelled `install`: `a1`, `a2`, `roots/base` and `roots/<mod>` for every
//! mod in the Roots install's `Data/Mods` (`--mod NAME` limits those).

use std::path::Path;

fn emit(label: &str, probes: Result<Vec<sta_mission::readiness::MissionProbe>, String>) {
    match probes {
        Ok(v) => {
            for p in v {
                let j = p.to_json();
                println!("{{\"install\":\"{label}\",{}", &j[1..]);
            }
        }
        Err(e) => println!("{{\"install\":\"{label}\",\"error\":{:?}}}", e),
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut a1, mut a2, mut roots, mut ticks) = (None, None, None, 300u64);
    let mut mods: Vec<String> = Vec::new();
    while let Some(a) = args.next() {
        let v = args.next().unwrap_or_default();
        match a.as_str() {
            "--a1" => a1 = Some(v),
            "--a2" => a2 = Some(v),
            "--roots" => roots = Some(v),
            "--mod" => mods.push(v),
            "--ticks" => ticks = v.parse().expect("--ticks N"),
            other => panic!("unknown argument {other}"),
        }
    }
    use sta_mission::readiness::probe_install;
    if let Some(p) = a1 {
        emit("a1", probe_install(Path::new(&p), None, ticks));
    }
    if let Some(p) = a2 {
        emit("a2", probe_install(Path::new(&p), None, ticks));
    }
    if let Some(p) = roots {
        let root = Path::new(&p);
        let mut names: Vec<String> = std::fs::read_dir(root.join("Data/Mods"))
            .map(|rd| rd.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        names.sort();
        if !mods.is_empty() {
            names.retain(|n| mods.contains(n));
        }
        emit("roots/base", probe_install(root, None, ticks));
        for n in names {
            emit(&format!("roots/{n}"), probe_install(root, Some(&n), ticks));
        }
    }
}
