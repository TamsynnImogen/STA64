//! `cargo run -p sta_content --example odf_index -- <install> [mod]`
//!
//! Builds the ODF index for an install (a folder with `Data/`, optionally a
//! mod under `Data/Mods`) or a bare game folder (`odf/` at its root, as the
//! Armada II retail install). Prints counts, where winners come from, and the
//! same-root duplicates.

use std::collections::BTreeMap;
use std::path::Path;

use sta_content::OdfIndex;
use sta_vfs::Vfs;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: odf_index <install> [mod]");
        std::process::exit(2);
    };
    let root = Path::new(root);
    let vfs = match Vfs::mount_install(root, args.get(1).map(String::as_str)) {
        Ok(v) => v,
        Err(_) => {
            let mut v = Vfs::default();
            v.push_dir("root".into(), root.to_path_buf()).unwrap_or_else(|e| {
                eprintln!("{}: {e}", root.display());
                std::process::exit(1);
            });
            v
        }
    };
    let (index, warnings) = OdfIndex::build(&vfs);

    let files: usize = index.iter().map(|(s, _)| index.versions(s).len()).sum();
    println!("layers: {}", vfs.layers().iter().map(|l| l.name()).collect::<Vec<_>>().join(" > "));
    println!("{} ODF names, {files} files", index.len());

    let mut by_layer: BTreeMap<usize, usize> = BTreeMap::new();
    let mut by_folder: BTreeMap<String, usize> = BTreeMap::new();
    for (_, e) in index.iter() {
        *by_layer.entry(e.layer).or_default() += 1;
        let folder = e.path.rsplit_once('/').map(|(d, _)| d).unwrap_or("").to_owned();
        *by_folder.entry(folder).or_default() += 1;
    }
    println!("winners by layer:");
    for (l, n) in &by_layer {
        println!("  {:>6}  {}", n, vfs.layers()[*l].name());
    }
    println!("winners by folder:");
    for (f, n) in &by_folder {
        println!("  {n:>6}  {f}/");
    }

    let overridden = index.duplicates().filter(|(_, all)| all[0].layer != all[1].layer).count();
    let moved = index
        .duplicates()
        .filter(|(_, all)| all[0].layer != all[1].layer && odf_rel(&all[0].path) != odf_rel(&all[1].path))
        .collect::<Vec<_>>();
    println!("{overridden} names overridden from a lower root, {} of them from a different folder", moved.len());
    for (stem, all) in moved.iter().take(10) {
        let chain: Vec<String> = all.iter().map(|e| format!("{}:{}", vfs.layers()[e.layer].name(), e.path)).collect();
        println!("  {stem}: {}", chain.join(" > "));
    }
    println!("{} same-root duplicates", warnings.len());
    for w in warnings.iter().take(15) {
        println!("  {w}");
    }
}

/// Path below `odf/`, ignoring the VFS's archive-stem prefix (`odf/odf/...`).
fn odf_rel(path: &str) -> &str {
    let mut rel = path;
    while let Some(r) = rel.strip_prefix("odf/") {
        rel = r;
    }
    rel
}
