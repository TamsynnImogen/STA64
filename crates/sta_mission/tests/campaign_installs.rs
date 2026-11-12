//! Installed campaigns (LUA-08), install-gated. Each test reads one game's
//! campaigns from its own files, plays mission 1 of its first open campaign
//! to a win and checks that mission 2 is selected, round-trips the progress
//! and prints the missions that cannot run yet. No game file is bundled.
//!
//! - `STA64_TEST_A1_INSTALL`: Armada I.
//! - `STA64_TEST_A2_STOCK_INSTALL`: stock Armada II.
//! - `STA64_TEST_ROOTS_INSTALL`: Fleet Ops Roots; the base and every mod in
//!   its `Data/Mods`.
//! - `STA64_TEST_FO327_INSTALL`, `STA64_TEST_DW_INSTALL`: Fleet Ops 3.2.7 and
//!   Dominion War, read and reported only (not v1 lineages, D5).
//!
//! A mission whose script cannot run yet (legacy DLL only) is played with a
//! stand-in script on its real map that ends it with `Mission:finish(true)`
//! after one second, which exercises the campaign flow on the real mission
//! order; the output says so. A real Lua mission runs as installed; if its own
//! win is not reached within the horizon (no player input is simulated), the
//! test ends it through the same `Mission:finish(true, 0)` call its script
//! uses, and says so.

use std::path::Path;
use std::sync::Arc;

use sta_mission::campaign::*;
use sta_sim::setup::GameSetup;
use sta_vfs::Vfs;

const HORIZON: u64 = 300;

const STAND_IN: &str = "MMM.register(function(t) if t >= 1 then Mission:finish(true, 0) end end)\n";

/// Real maps and content, a stand-in script where the mission cannot run.
struct StandIn<'a>(&'a VfsHost);

impl MissionHost for StandIn<'_> {
    fn game_setup(&self, c: &Campaign, m: &CampaignMission, seed: u64) -> Result<GameSetup, String> {
        self.0.game_setup(c, m, seed)
    }

    fn script(&self, _c: &Campaign, _m: &CampaignMission) -> Result<(String, ScriptReader), String> {
        Ok(("missions/__stand_in.lua".into(), Arc::new(|p| (p == "missions/__stand_in.lua").then(|| STAND_IN.as_bytes().to_vec()))))
    }

    fn seat(&self) -> (sta_core::PlayerId, sta_mission::api::PlayerNumbering) {
        self.0.seat()
    }
}

fn report(game: &str, book: &CampaignBook) {
    eprintln!("[{game}] layout {:?}, GIVE_ALL_MISSIONS {}, digest {:016x}", book.layout, book.give_all_missions, book.digest());
    for w in &book.warnings {
        eprintln!("[{game}]   warning: {w}");
    }
    for c in &book.campaigns {
        eprintln!(
            "[{game}]   {} ({}): {} missions, requires {:?}, locked text {:?}",
            c.id,
            c.name.as_deref().unwrap_or("-"),
            c.missions.len(),
            c.requires,
            c.locked_text
        );
        for m in &c.missions {
            let script = match &m.script {
                MissionScript::Lua { path } => format!("lua {path}"),
                MissionScript::LegacyDll { files } => format!("dll {}", files.join("+")),
                MissionScript::None => "none".into(),
            };
            eprintln!(
                "[{game}]     {:>2} {} {:?}: {script}; briefing {:?}; movies {}; ends {:?}",
                m.index + 1,
                m.bzn,
                m.title.as_deref().unwrap_or("-"),
                m.briefing.as_ref().map(|b| b.title.as_str()),
                m.movies.len(),
                m.end_calls
            );
        }
    }
    let blocked = book.blocked();
    let total: usize = book.campaigns.iter().map(|c| c.missions.len()).sum();
    eprintln!("[{game}] cannot run yet: {} of {total}", blocked.len());
    for b in &blocked {
        eprintln!("[{game}]   {} #{} {}: {}", b.campaign, b.index + 1, b.bzn, b.reason);
    }
}

/// Play mission 1 of the first open campaign with two or more missions.
fn play_first(game: &str, vfs: Vfs) -> CampaignBook {
    let book = CampaignBook::load_vfs(&vfs);
    report(game, &book);
    let vfs = Arc::new(vfs);
    let host = VfsHost::load(vfs).expect("installed content loads");
    // A new player's progress without GIVE_ALL_MISSIONS: only open campaigns.
    let mut fresh = book.clone();
    fresh.give_all_missions = false;
    let mut runner = CampaignRunner::new(fresh);
    let campaign = runner
        .open_campaigns()
        .into_iter()
        .find(|c| runner.book.campaign(c).is_some_and(|c| c.missions.len() >= 2))
        .expect("an open campaign with two missions");
    let first = runner.book.campaign(&campaign).unwrap().missions[0].bzn.clone();
    let how = match runner.start(&host, &campaign, 0, 1) {
        Ok(a) => match a.run(HORIZON) {
            Some(o) => format!("installed script ended it itself (won {})", o.won),
            None => {
                let errors = a.runtime().errors().len();
                a.runtime().eval::<()>("Mission:finish(true, 0)").expect("script's own win call");
                format!("installed script ran {HORIZON} ticks ({errors} errors), win ended through Mission:finish(true, 0)")
            }
        },
        Err(StartError::Blocked(why)) => {
            eprintln!("[{game}] {campaign} #1 {first} blocked: {why}");
            let a = runner.start(&StandIn(&host), &campaign, 0, 1).expect("stand-in starts on the real map");
            a.run(HORIZON).expect("stand-in wins");
            "stand-in script on the real map".into()
        }
        Err(e) => panic!("[{game}] {campaign} #1: {e}"),
    };
    let tick = runner.active().unwrap().sim().tick;
    let a = runner.active_mut().unwrap();
    let outcome = a.check_outcome(tick).expect("mission ended");
    assert!(outcome.won, "[{game}] mission 1 ended as a loss: {outcome:?}");
    let next = runner.finish(outcome).unwrap();
    let second = runner.book.campaign(&campaign).unwrap().missions[1].bzn.clone();
    eprintln!("[{game}] played {campaign} #1 {first} ({how}); next: {next:?} = {second}");
    assert_eq!(next, Transition::Next { campaign: campaign.clone(), mission: 1 });
    assert_eq!(runner.progress.available(&runner.book, &campaign), 0..2);
    let text = runner.progress.save();
    let back = CampaignProgress::restore_for(&runner.book, &text).expect("progress restores");
    assert_eq!(back, runner.progress);
    assert_eq!(back.save(), text);
    book
}

fn counts(book: &CampaignBook) -> Vec<usize> {
    book.campaigns.iter().map(|c| c.missions.len()).collect()
}

fn mount(var: &str, active_mod: Option<&str>) -> Option<Vfs> {
    let root = std::env::var(var).ok()?;
    Some(Vfs::mount_install(Path::new(&root), active_mod).expect("mount install"))
}

#[test]
fn armada1_campaigns() {
    let Some(vfs) = mount("STA64_TEST_A1_INSTALL", None) else { return };
    let book = play_first("A1", vfs);
    assert_eq!(book.layout, ShellLayout::Armada1);
    assert!(counts(&book).iter().all(|&n| n >= 2), "{:?}", counts(&book));
    assert_eq!(book.campaigns[1].requires.as_deref(), Some("federation"));
}

#[test]
fn armada2_stock_campaigns() {
    let Some(vfs) = mount("STA64_TEST_A2_STOCK_INSTALL", None) else { return };
    let book = play_first("A2", vfs);
    assert_eq!(book.layout, ShellLayout::Armada2);
    assert_eq!(book.campaigns.len(), 4);
    assert!(counts(&book).iter().all(|&n| n >= 2), "{:?}", counts(&book));
    assert_eq!(book.campaign("borg").unwrap().requires.as_deref(), Some("klingon"));
}

#[test]
fn roots_campaigns_base_and_mods() {
    let Ok(root) = std::env::var("STA64_TEST_ROOTS_INSTALL") else { return };
    let mods_dir = Path::new(&root).join("Data/Mods");
    let mut mods: Vec<Option<String>> = vec![None];
    if let Ok(rd) = std::fs::read_dir(&mods_dir) {
        let mut names: Vec<String> = rd.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        mods.extend(names.into_iter().map(Some));
    }
    for m in mods {
        let game = format!("Roots/{}", m.as_deref().unwrap_or("base"));
        let vfs = Vfs::mount_install(Path::new(&root), m.as_deref()).expect("mount");
        let book = CampaignBook::load_vfs(&vfs);
        if book.campaigns.iter().all(|c| c.missions.len() < 2) {
            report(&game, &book);
            eprintln!("[{game}] no campaign with two missions: nothing to play");
            continue;
        }
        play_first(&game, vfs);
    }
}

/// Not v1 lineages (D5): read and reported only.
#[test]
fn fo327_and_dominion_war_report() {
    for (game, var) in [("FO 3.2.7", "STA64_TEST_FO327_INSTALL"), ("Dominion War", "STA64_TEST_DW_INSTALL")] {
        let Some(vfs) = mount(var, None) else { continue };
        let book = CampaignBook::load_vfs(&vfs);
        report(game, &book);
    }
}
