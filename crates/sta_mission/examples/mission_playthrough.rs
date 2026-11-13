//! Plays one campaign mission headless with a scripted player and prints its
//! timeline as JSON lines (`sta_mission::playthrough`, LUA-17). Driven by
//! `tools/mission_playthrough.py`.
//!
//! `mission_playthrough --install <dir> [--mod NAME] [--converted <dir>] --mission <bzn stem>
//!  [--plan FILE.lua] [--ticks N] [--seed N]`
//! `--converted` is a directory holding `missions/<map>.lua` (the converter's output), mounted
//! over the install. No window, no audio. Prints names, numbers and the script's requests only.

use std::path::Path;
use std::sync::Arc;

use sta_mission::campaign::{CampaignBook, VfsHost};
use sta_mission::playthrough::{Config, play};

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut install, mut modname, mut converted, mut mission, mut plan) = (None, None, None, None::<String>, None);
    let mut cfg = Config { ticks: 30 * 60 * 30, plan: None, seed: 1 };
    while let Some(a) = args.next() {
        let v = args.next().unwrap_or_default();
        match a.as_str() {
            "--install" => install = Some(v),
            "--mod" => modname = Some(v),
            "--converted" => converted = Some(v),
            "--mission" => mission = Some(v),
            "--plan" => plan = Some(v),
            "--ticks" => cfg.ticks = v.parse().expect("--ticks N"),
            "--seed" => cfg.seed = v.parse().expect("--seed N"),
            other => panic!("unknown argument {other}"),
        }
    }
    let mut vfs = sta_vfs::Vfs::mount_install(Path::new(&install.expect("--install DIR")), modname.as_deref()).expect("mount install");
    if let Some(c) = converted {
        vfs.push_dir("converted".into(), c.into()).expect("mount converted");
    }
    cfg.plan = plan.map(|p| std::fs::read_to_string(p).expect("plan file"));
    let book = CampaignBook::load_vfs(&vfs);
    let mission = mission.expect("--mission STEM").to_ascii_lowercase();
    let (campaign, index) = book
        .campaigns
        .iter()
        .flat_map(|c| c.missions.iter().map(move |m| (c, m)))
        .find(|(_, m)| m.stem().to_ascii_lowercase() == mission)
        .map(|(c, m)| (c.id.clone(), m.index))
        .unwrap_or_else(|| panic!("no campaign mission {mission}"));
    let host = VfsHost::load(Arc::new(vfs)).expect("content loads");
    match play(&host, &book, &campaign, index, &cfg) {
        Ok(p) => {
            for e in &p.events {
                println!("{}", e.to_json());
            }
        }
        Err(e) => {
            eprintln!("cannot start {mission}: {e}");
            std::process::exit(2);
        }
    }
}
