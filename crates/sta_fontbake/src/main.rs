//! `cargo run --release -p sta_fontbake -- <font-dir> <out-dir>`: bakes Noto Sans
//! Condensed Medium and SemiBold into `font-regular.bin` and `font-bold.bin`.
//! The shipped atlases live in `crates/sta64/assets/ui` (see its `NOTICE.md`).

use std::path::PathBuf;

fn main() {
    let mut a = std::env::args().skip(1);
    let (Some(dir), Some(out)) = (a.next().map(PathBuf::from), a.next().map(PathBuf::from)) else {
        eprintln!("usage: sta_fontbake <dir with NotoSans-Condensed{{Medium,SemiBold}}.ttf> <out dir>");
        std::process::exit(2);
    };
    if let Err(e) = sta_fontbake::bake_fonts(&dir, &out) {
        eprintln!("sta_fontbake: {e}");
        std::process::exit(1);
    }
}
