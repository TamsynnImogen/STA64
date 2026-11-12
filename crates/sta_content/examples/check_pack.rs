//! Coverage report and `why` for an install or a mod (CON-34, D7). Read-only.
//!
//! ```text
//! cargo run -p sta_content --example check_pack -- <root> [<mod> | --all]
//! cargo run -p sta_content --example check_pack -- <root> <mod> --why <file> [<key>]
//! ```
//!
//! `<root>` is an install (with `Data`). Without a mod the `Data` layer is
//! checked; `--all` checks `Data` and every mod under `Data/Mods`, each
//! mounted by its own rules (`standalone`, `ignoreParentMaps`). `--why`
//! names the layer that supplies a file (an ODF stem, a path, `RTS_CFG.h`)
//! and, with a key, the file and line that set the key.

use std::path::Path;

use sta_content::coverage;
use sta_vfs::Vfs;

fn mount(root: &Path, m: Option<&str>) -> Result<Vfs, String> {
    Vfs::mount_install(root, m).map_err(|e| format!("{}: {e}", m.unwrap_or("Data")))
}

fn run(args: &[String]) -> Result<(), String> {
    let usage = "usage: check_pack <root> [<mod> | --all] [--why <file> [<key>]]";
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
