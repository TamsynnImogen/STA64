//! Make an Armada II random map (CON-49) and write it as a BZN.
//!
//! `cargo run -p sta_content --example random_map -- <seed> <players> [out.bzn] [install root] [mod]`
//!
//! With an install the objects' sizes come from its content; without one the
//! generator's stock sizes stand in.

use std::path::Path;

use sta_content::random_map::{self, ContentClasses, StockSizes};
use sta_content::{ContentDb, OdfIndex};
use sta_vfs::Vfs;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(seed), Some(players)) = (args.first().and_then(|s| s.parse::<u32>().ok()), args.get(1).and_then(|s| s.parse::<u8>().ok())) else {
        eprintln!("usage: random_map <seed> <players 1-8> [out.bzn] [install root] [mod]");
        std::process::exit(2);
    };
    let map = match args.get(3) {
        Some(root) => {
            let vfs = Vfs::mount_install(Path::new(root), args.get(4).map(String::as_str)).expect("mount the install");
            let (index, _) = OdfIndex::build(&vfs);
            let (db, _) = ContentDb::load(&vfs, &index).expect("load the content");
            let mut lookup = |name: &str| sta_odf::resolve_layered(&name.to_ascii_lowercase(), &mut |n, v| index.load(&vfs, n, v)).ok().map(std::sync::Arc::new);
            random_map::generate(seed, players, &mut ContentClasses { db: &db, odfs: &mut lookup })
        }
        None => random_map::generate(seed, players, &mut StockSizes),
    };
    println!("seed {seed}: {:.0} units across, {} start locations, {} objects, {} paths", map.size[0], map.start_count, map.objects.len(), map.paths.len());
    let mut counts = std::collections::BTreeMap::new();
    for o in &map.objects {
        *counts.entry(o.odf.as_str()).or_insert(0usize) += 1;
    }
    for (odf, n) in counts {
        println!("  {n:4} {odf}");
    }
    if let Some(out) = args.get(2) {
        std::fs::write(out, sta_bzn::write::write(&map).expect("write the map")).expect("write the file");
        println!("wrote {out}");
    }
}
