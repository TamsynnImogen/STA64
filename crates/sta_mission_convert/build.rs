//! Takes a digest of this crate's source files (`src/**`) into `STA_MISSION_CONVERT_SRC_DIGEST`, the
//! identity the conversion cache's keys carry: a changed converter is a new key, without anyone
//! having to raise a version number and without git. FNV-1a over the sorted relative paths and
//! file bytes (line endings are part of it: another checkout is another identity, harmless).

use std::fs;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn fnv(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h ^= u64::from(*b);
        *h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=build.rs");
    let src = Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir")).join("src");
    let mut files = Vec::new();
    walk(&src, &mut files);
    files.sort();
    // two lanes with different seeds: 128 bits
    let (mut a, mut b) = (0xcbf2_9ce4_8422_2325u64, 0x8422_2325_cbf2_9ce4u64);
    for f in &files {
        let rel = f.strip_prefix(&src).unwrap_or(f).to_string_lossy().replace('\\', "/");
        let data = fs::read(f).unwrap_or_default();
        for h in [&mut a, &mut b] {
            fnv(h, rel.as_bytes());
            fnv(h, &[0]);
            fnv(h, &(data.len() as u64).to_le_bytes());
            fnv(h, &data);
        }
    }
    println!("cargo:rustc-env=STA_MISSION_CONVERT_SRC_DIGEST={a:016x}{b:016x}");
}
