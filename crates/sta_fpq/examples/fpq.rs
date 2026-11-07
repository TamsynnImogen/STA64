//! `cargo run -p sta_fpq --example fpq -- <archive.fpq> [list | extract <out-dir>]`

use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(archive) = args.first() else {
        eprintln!("usage: fpq <archive.fpq> [list | extract <out-dir>]");
        std::process::exit(2);
    };
    let fpq = sta_fpq::Fpq::open(Path::new(archive)).unwrap_or_else(|e| {
        eprintln!("{archive}: {e}");
        std::process::exit(1);
    });
    match args.get(1).map(String::as_str).unwrap_or("list") {
        "list" => {
            for i in 0..fpq.len() {
                let e = fpq.entry(i);
                println!("{:10} {:10} {} {}", e.size, e.stored_size, e.compression, fpq.path(i).unwrap());
            }
        }
        "extract" => {
            let out = Path::new(args.get(2).expect("extract needs an output directory"));
            for i in 0..fpq.len() {
                let rel = fpq.path(i).unwrap();
                let dest = out.join(&rel);
                std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
                std::fs::write(&dest, fpq.read(i).unwrap()).unwrap();
            }
            eprintln!("extracted {} files to {}", fpq.len(), out.display());
        }
        other => {
            eprintln!("unknown command {other}");
            std::process::exit(2);
        }
    }
}
