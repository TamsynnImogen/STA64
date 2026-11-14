//! Prints one JSON line per converted mission: do its endings work (`sta_mission::ending`)?
//! Driven by `tools/campaign_readiness.py --converted a1|a2 --ending`.
//!
//! `ending_check --install <dir> --converted <dir> --game a1|a2 [--min-tick N] [--cap N] [--forced-only] [--only a2_fed01]`
//! `--converted` is the output directory of `tools/dsl_to_lua.py` / `tools/a1_to_lua.py`; it is
//! mounted as a layer over the install, so its `missions/<map>.lua` resolve.

use std::path::Path;
use std::sync::Arc;

use sta_mission::campaign::{CampaignBook, VfsHost};
use sta_mission::cheat::CheatGame;
use sta_mission::ending::{probe_book, EndingOptions};

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut install, mut converted, mut game, mut only) = (None, None, None::<String>, None::<String>);
    let mut forced_only = false;
    let (mut min_tick, mut cap) = (None::<u64>, None::<u64>);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--forced-only" => forced_only = true,
            "--install" => install = args.next(),
            "--converted" => converted = args.next(),
            "--game" => game = args.next(),
            "--only" => only = args.next(),
            "--min-tick" => min_tick = args.next().and_then(|v| v.parse().ok()),
            "--cap" => cap = args.next().and_then(|v| v.parse().ok()),
            other => panic!("unknown argument {other}"),
        }
    }
    let (install, converted) = (install.expect("--install DIR"), converted.expect("--converted DIR"));
    let game = match game.as_deref() {
        Some("a1") => CheatGame::A1,
        Some("a2") => CheatGame::A2,
        _ => panic!("--game a1|a2"),
    };
    let mut vfs = sta_vfs::Vfs::mount_install(Path::new(&install), None).expect("mount install");
    vfs.push_dir("converted".into(), converted.into()).expect("mount converted");
    let book = CampaignBook::load_vfs(&vfs);
    let vfs = Arc::new(vfs);
    let host = VfsHost::load(vfs.clone()).expect("content loads");
    let mut opts = EndingOptions::new(game);
    opts.forced_only = forced_only;
    if let Some(v) = min_tick {
        opts.min_tick = v;
    }
    if let Some(v) = cap {
        opts.cap = v;
    }
    for p in probe_book(&host, Some(&vfs), &book, &opts, only.as_deref()) {
        println!("{}", p.to_json());
    }
}
