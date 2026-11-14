//! Make an Armada II random map (CON-49) and write it as a BZN.
//!
//! `cargo run -p sta_content --example random_map -- <seed> <players> [out.bzn] [install root] [mod] [set]`
//!
//! With an install the objects' sizes come from its content and the table is
//! its `random_map.json5` (CON-50; the stock table when it has none); without
//! one the generator's stock sizes and table stand in. `-` skips an argument.

use std::path::Path;

use sta_content::random_map::{self, ContentClasses, StockSizes, Table};
use sta_content::{ContentDb, OdfIndex};
use sta_vfs::Vfs;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(seed), Some(players)) = (args.first().and_then(|s| s.parse::<u32>().ok()), args.get(1).and_then(|s| s.parse::<u8>().ok())) else {
        eprintln!("usage: random_map <seed> <players 1-8> [out.bzn] [install root] [mod] [set]");
        std::process::exit(2);
    };
    let arg = |i: usize| args.get(i).map(String::as_str).filter(|a| *a != "-");
    let set = arg(5);
    let map = match arg(3) {
        Some(root) => {
            let vfs = Vfs::mount_install(Path::new(root), arg(4)).expect("mount the install");
            let (index, _) = OdfIndex::build(&vfs);
            let (db, _) = ContentDb::load(&vfs, &index).expect("load the content");
            let mut lookup = |name: &str| sta_odf::resolve_layered(&name.to_ascii_lowercase(), &mut |n, v| index.load(&vfs, n, v)).ok().map(std::sync::Arc::new);
            let (table, mut warnings) = Table::load(&vfs);
            warnings.extend(table.check(&mut |odf| lookup(odf).is_some()));
            for w in warnings {
                eprintln!("warning: {}: {}", w.file, w.message);
            }
            println!("sets: {}", table.sets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", "));
            random_map::generate_with(seed, players, &table, set, &mut ContentClasses { db: &db, odfs: &mut lookup })
        }
        None => random_map::generate(seed, players, &mut StockSizes),
    };
    println!("{}", map.description);
    println!("seed {seed}: {:.0} units across, {} start locations, {} objects, {} paths", map.size[0], map.start_count, map.objects.len(), map.paths.len());
    let mut counts = std::collections::BTreeMap::new();
    for o in &map.objects {
        *counts.entry(o.odf.as_str()).or_insert(0usize) += 1;
    }
    for (odf, n) in counts {
        println!("  {n:4} {odf}");
    }
    if let Some(out) = arg(2) {
        std::fs::write(out, sta_bzn::write::write(&map).expect("write the map")).expect("write the file");
        println!("wrote {out}");
    }
}
