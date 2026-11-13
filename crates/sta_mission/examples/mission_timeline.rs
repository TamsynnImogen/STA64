//! Prints the timeline of one campaign mission run headless: the presentation requests the
//! script raises, destroyed units, script errors, unsupported calls and the end.
//!
//! `mission_timeline --install <dir> [--mod NAME] --campaign <id> --index <n> [--ticks N]
//!  [--dump-script PATH] [--probe LUA_FILE]` (the chunk returns a string; run at a few early ticks)
//! No window, no audio, no player input. Names and counts only; nothing is printed of the script.

use std::path::Path;
use std::sync::Arc;

use sta_mission::campaign::{CampaignBook, CampaignRunner, MissionScript, VfsHost};
use sta_sim::{SimEvent, WorldSource};
use std::collections::HashMap;

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut install, mut modname, mut campaign, mut index, mut ticks, mut dump, mut probe) = (None, None, None, 0usize, 6000u64, None::<String>, None::<String>);
    while let Some(a) = args.next() {
        let v = args.next().unwrap_or_default();
        match a.as_str() {
            "--install" => install = Some(v),
            "--mod" => modname = Some(v),
            "--campaign" => campaign = Some(v),
            "--index" => index = v.parse().expect("--index N"),
            "--ticks" => ticks = v.parse().expect("--ticks N"),
            "--dump-script" => dump = Some(v),
            "--probe" => probe = Some(std::fs::read_to_string(&v).expect("probe file")),
            other => panic!("unknown argument {other}"),
        }
    }
    let vfs = sta_vfs::Vfs::mount_install(Path::new(&install.expect("--install DIR")), modname.as_deref()).expect("mount");
    let book = CampaignBook::load_vfs(&vfs);
    let campaign = campaign.expect("--campaign ID");
    let m = &book.campaigns.iter().find(|c| c.id == campaign).expect("campaign").missions[index];
    println!("mission: {:?} {} map_present={} script={:?} movies={:?} ends={:?}", m.title, m.bzn, m.map_present, m.script, m.movies, m.end_calls);
    println!("blocked(): {:?}", book.blocked());
    if let (Some(path), MissionScript::Lua { path: lua }) = (&dump, &m.script)
        && let Some(Ok(b)) = vfs.read(lua)
    {
        std::fs::write(path, b).expect("write script");
    }
    let host = VfsHost::load(Arc::new(vfs)).expect("content");
    let mut open = book.clone();
    open.give_all_missions = true;
    let mut progress = sta_mission::campaign::CampaignProgress::new(&open);
    progress.apply_give_all(&open);
    let mut runner = CampaignRunner::with_progress(open, progress);
    let a = runner.start(&host, &campaign, index, 1).expect("starts");
    let mut owners: HashMap<sta_core::EntityId, (sta_core::PlayerId, String)> = HashMap::new();
    let mut hits: std::collections::BTreeMap<(u8, u8), u32> = Default::default();
    let mut errs = 0;
    let mut logn = 0;
    for _ in 0..ticks {
        let o = a.tick();
        let t = a.sim().tick;
        for u in &a.sim().view().units {
            owners.entry(u.id).or_insert((u.owner, format!("{:?}", u.def)));
        }
        for e in a.last_events() {
            match e {
                SimEvent::Present(p) => println!("t={t} PRESENT {p:?}"),
                SimEvent::Destroyed { id, by } => {
                    let of = |i: &sta_core::EntityId| owners.get(i).map_or("?".to_owned(), |(p, d)| format!("p{} {d}", p.0));
                    println!("t={t} DESTROYED {id:?} [{}] by {:?} [{}]", of(id), by, by.as_ref().map_or("-".into(), of));
                }
                SimEvent::Hit { target, by, .. } => {
                    if let (Some(a), Some(b)) = (owners.get(by), owners.get(target)) {
                        *hits.entry((a.0 .0, b.0 .0)).or_default() += 1;
                    }
                }
                SimEvent::PlayerEliminated { player } => println!("t={t} ELIMINATED {player:?}"),
                SimEvent::Spawned { id } => println!("t={t} spawned {id:?}"),
                _ => {}
            }
        }
        let e = a.runtime().errors();
        while errs < e.len() {
            println!("t={t} ERROR {}", e[errs].lines().next().unwrap_or(""));
            errs += 1;
        }
        let l = a.runtime().log();
        while logn < l.len() {
            println!("t={t} LOG {}", l[logn]);
            logn += 1;
        }
        if let Some(chunk) = &probe
            && [1u64, 2, 3, 5, 10, 31, 90].contains(&t)
        {
            match a.runtime().eval::<String>(chunk) {
                Ok(r) => println!("t={t} PROBE {r}"),
                Err(e) => println!("t={t} PROBE error {e}"),
            }
        }
        if let Some(o) = o {
            println!("t={t} END won={} delay={}", o.won, o.delay);
            break;
        }
    }
    println!("final tick {}", a.sim().tick);
    println!("hits (shooter player, target player) -> count: {hits:?}");
    println!("unsupported: {:?}", a.runtime().unsupported());
    println!("objectives: {:?}", a.runtime().objectives());
}
