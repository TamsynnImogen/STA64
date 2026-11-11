//! Inspect explicit model texture references through the shared asset loaders.
//! `cargo run -p sta_assets --example model_textures -- [--deny-missing] <install> <model> [mod]`
//! `cargo run -p sta_assets --example model_textures -- --root <asset-root> <model>`

use std::path::PathBuf;

use sta_assets::{find_model, find_texture, load_model, model::Model};
use sta_vfs::Vfs;

const USAGE: &str = "usage: model_textures [--deny-missing] <install> <model> [mod]\n       model_textures [--deny-missing] --root <asset-root> <model>\nReports references exposed by the shared model loader. Found paths are lookup only; textures are not decoded.";

#[derive(Debug, PartialEq, Eq)]
struct Options {
    root: PathBuf,
    model: String,
    active_mod: Option<String>,
    loose_root: bool,
    deny_missing: bool,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Option<Options>, String> {
    let mut positional = Vec::new();
    let mut loose_root = None;
    let mut deny_missing = false;
    let mut options = true;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if options {
            match arg.as_str() {
                "--" => {
                    options = false;
                    continue;
                }
                "--help" => return Ok(None),
                "--deny-missing" => {
                    if deny_missing {
                        return Err("--deny-missing specified twice".into());
                    }
                    deny_missing = true;
                    continue;
                }
                "--root" => {
                    if loose_root.is_some() {
                        return Err("--root specified twice".into());
                    }
                    let root = args.next().ok_or("--root needs an asset directory")?;
                    loose_root = Some(PathBuf::from(root));
                    continue;
                }
                _ if arg.starts_with('-') => return Err(format!("unknown option {arg:?}")),
                _ => {}
            }
        }
        positional.push(arg);
    }
    let (root, model, active_mod, loose_root) = match loose_root {
        Some(root) if positional.len() == 1 => (root, positional.remove(0), None, true),
        Some(_) => return Err("--root requires exactly one model name".into()),
        None if positional.len() == 2 || positional.len() == 3 => {
            let active_mod = (positional.len() == 3).then(|| positional.remove(2));
            (
                PathBuf::from(positional.remove(0)),
                positional.remove(0),
                active_mod,
                false,
            )
        }
        None => return Err("expected <install> <model> [mod]".into()),
    };
    Ok(Some(Options {
        root,
        model,
        active_mod,
        loose_root,
        deny_missing,
    }))
}

#[derive(Debug)]
struct Report {
    output: String,
    missing: usize,
}

fn report(vfs: &Vfs, path: &str, model: &Model) -> Report {
    use std::fmt::Write;

    let mut output = format!("model {path:?} format={:?} version={}\n", model.format, model.version);
    for warning in &model.warnings {
        writeln!(output, "warning {warning:?}").unwrap();
    }
    let mut missing = 0;
    for (node_index, node) in model.nodes.iter().enumerate() {
        let Some(mesh) = node.mesh() else { continue };
        for (group_index, group) in mesh.groups.iter().enumerate() {
            writeln!(
                output,
                "node={node_index} name={:?} group={group_index} material={:?}",
                node.name, group.material
            )
            .unwrap();
            let maps = &group.maps;
            for (role, reference) in [
                ("diffuse", &maps.diffuse),
                ("height", &maps.height),
                ("bump", &maps.bump),
                ("specular", &maps.specular),
                ("glow", &maps.glow),
                ("detail", &maps.detail),
            ] {
                let Some(reference) = reference else { continue };
                // An empty map in Model is still an occurrence, but never a lookup.
                let found = (!reference.is_empty()).then(|| find_texture(vfs, reference)).flatten();
                match found.and_then(|(path, format)| vfs.resolve(&path).map(|(layer, _)| (path, format, layer))) {
                    Some((texture_path, format, layer_index)) => {
                        let layer_name = vfs.layers()[layer_index].name();
                        writeln!(
                            output,
                            "  {role} ref={reference:?} path={texture_path:?} format={format:?} layer={layer_index} layer_name={layer_name:?}"
                        )
                        .unwrap();
                    }
                    None => {
                        missing += 1;
                        writeln!(output, "  {role} ref={reference:?} missing").unwrap();
                    }
                }
            }
        }
    }
    Report { output, missing }
}

fn run(options: &Options) -> Result<Report, String> {
    let vfs = if options.loose_root {
        let mut vfs = Vfs::default();
        vfs.push_dir("root".into(), options.root.clone())
            .map_err(|e| format!("mount {:?}: {e}", options.root))?;
        vfs
    } else {
        Vfs::mount_install(&options.root, options.active_mod.as_deref())
            .map_err(|e| format!("mount {:?}: {e}", options.root))?
    };
    let (path, format) =
        find_model(&vfs, &options.model).ok_or_else(|| format!("model {:?} not found", options.model))?;
    let model = load_model(&vfs, &path, format).map_err(|e| format!("load model {path:?}: {e}"))?;
    Ok(report(&vfs, &path, &model))
}

fn main() {
    let args = std::env::args_os()
        .skip(1)
        .map(|arg| arg.into_string().map_err(|_| "arguments must be UTF-8".to_owned()))
        .collect::<Result<Vec<_>, _>>();
    let options = match args.and_then(parse_args) {
        Ok(Some(options)) => options,
        Ok(None) => {
            println!("{USAGE}");
            return;
        }
        Err(error) => {
            eprintln!("{error}\n{USAGE}");
            std::process::exit(2);
        }
    };
    match run(&options) {
        Ok(report) => {
            print!("{}", report.output);
            if options.deny_missing && report.missing > 0 {
                std::process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
#[path = "model_textures/tests.rs"]
mod tests;
