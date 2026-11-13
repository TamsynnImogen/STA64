//! Prints one JSON line per converted mission: how far it got in N headless
//! ticks (`sta_mission::progress`). Driven by `tools/campaign_readiness.py --converted`.
//!
//! `converted_progress --install <dir> --converted <dir> [--ticks N] [--standin] [--only a2_fed01]`
//! `--converted` is the output directory of `tools/dsl_to_lua.py`; it is
//! mounted as a layer over the install, so its `missions/<map>.lua` resolve.

use std::path::Path;
use std::sync::Arc;

use sta_mission::campaign::{CampaignBook, VfsHost};

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut install, mut converted, mut ticks, mut standin, mut only) = (None, None, 9000u64, false, None::<String>);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--standin" => standin = true,
            "--install" => install = args.next(),
            "--converted" => converted = args.next(),
            "--ticks" => ticks = args.next().and_then(|v| v.parse().ok()).expect("--ticks N"),
            "--only" => only = args.next(),
            other => panic!("unknown argument {other}"),
        }
    }
    let (install, converted) = (install.expect("--install DIR"), converted.expect("--converted DIR"));
    let mut vfs = sta_vfs::Vfs::mount_install(Path::new(&install), None).expect("mount install");
    vfs.push_dir("converted".into(), converted.into()).expect("mount converted");
    let book = CampaignBook::load_vfs(&vfs);
    let host = VfsHost::load(Arc::new(vfs)).expect("content loads");
    for p in sta_mission::progress::probe_book(&host, &book, ticks, standin, only.as_deref()) {
        println!("{}", p.to_json());
    }
}
