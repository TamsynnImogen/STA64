//! LUA-17: stock campaign missions played to the end by the playthrough driver
//! (`sta_mission::playthrough`), install-gated.
//!
//! Each test converts from the install named by its variable with the Rust converter
//! (`sta_mission_convert`, the library the game calls; no Python, no `objdump`) into a temporary
//! folder that is removed with the test, mounts the output over the install and plays a mission
//! with a plan from `tests/playthrough/` (a scripted player that gives orders through the normal
//! command interface). It checks the outcome and what the script shows on the way. Skipped when
//! the variable is not set. Prints the timeline's summary on failure.

#[path = "support/convert.rs"]
mod support;

use std::path::Path;
use std::sync::Arc;

use sta_mission::campaign::{CampaignBook, VfsHost};
use sta_mission::playthrough::{Config, Playthrough, play};

/// The install with `conv` mounted over it: its book and host.
fn mounted(install: &str, conv: &support::Conversion) -> (CampaignBook, VfsHost) {
    let mut vfs = sta_vfs::Vfs::mount_install(Path::new(install), None).expect("mount install");
    vfs.push_dir("converted".into(), conv.dir()).expect("mount converted");
    let book = CampaignBook::load_vfs(&vfs);
    (book, VfsHost::load(Arc::new(vfs)).expect("content"))
}

fn run_plan(host: &VfsHost, book: &CampaignBook, campaign: &str, index: usize, plan: &str, ticks: u64) -> Playthrough {
    let plan = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/playthrough").join(plan)).expect("plan");
    play(host, book, campaign, index, &Config { ticks, plan: Some(plan), seed: 1 }).expect("starts")
}

/// Converts only the mission of the map `stem` and plays it (the plan is written for that map).
fn play_mission(install: &str, stem: &str, plan: &str, ticks: u64) -> Playthrough {
    let conv = support::convert(Path::new(install), "playthrough", &[stem], false);
    let (book, host) = mounted(install, &conv);
    let (campaign, index) = book
        .campaigns
        .iter()
        .flat_map(|c| c.missions.iter().map(move |m| (c, m)))
        .find(|(_, m)| m.stem().eq_ignore_ascii_case(stem))
        .map(|(c, m)| (c.id.clone(), m.index))
        .unwrap_or_else(|| panic!("no campaign mission for that map"));
    run_plan(&host, &book, &campaign, index, plan, ticks)
}

fn dump(p: &Playthrough) -> String {
    p.events
        .iter()
        .filter(|e| !matches!(e.kind, "call" | "camera" | "spawned" | "order" | "step" | "present"))
        .map(|e| format!("{:8.1} {:<12} {}", e.secs(), e.kind, e.what.lines().next().unwrap_or("")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// An Armada I campaign mission played to a win by its plan: the briefings come in their order,
/// control is released before the first objective is shown, and the script raises no error.
#[test]
fn a1_federation_1_is_won_by_a_competent_player() {
    let Ok(install) = std::env::var("STA64_TEST_A1_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A1_INSTALL to an Armada I root");
        return;
    };
    let p = play_mission(&install, "fed1", "a1_fed1.lua", 30 * 60 * 40);
    let log = dump(&p);
    assert!(p.outcome.is_some_and(|o| o.won), "not won:\n{log}");
    // The mission's three briefings, in their order.
    let briefings: Vec<_> = p.of("briefing").map(|e| e.what.to_ascii_lowercase()).collect();
    let want = ["f1ob1.txt", "f1ob2.txt", "f1ob3.txt"];
    let mut at = 0;
    for b in &briefings {
        if at < want.len() && b.ends_with(want[at]) {
            at += 1;
        }
    }
    assert_eq!(at, want.len(), "briefings {briefings:?}\n{log}");
    // Control is released (the opening cineractive ends) before the first objective is shown.
    let released = p.of("present").find(|e| e.what.contains("Cineractive { on: false")).map(|e| e.tick);
    let first = p.of("objective").next().map(|e| e.tick);
    assert!(released.is_some() && released <= first, "opening:\n{log}");
    assert!(p.of("error").next().is_none(), "script errors:\n{log}");
}

/// A second Armada I campaign mission. Rule under test: orders for the units of a seat that has
/// been eliminated still reach the sim (units of that seat first seen after the elimination act),
/// and the mission's end rule can still hold afterwards (the win).
#[test]
fn a1_federation_2_waves_act_and_the_mission_is_won() {
    let Ok(install) = std::env::var("STA64_TEST_A1_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A1_INSTALL to an Armada I root");
        return;
    };
    let p = play_mission(&install, "fed2", "a1_fed2.lua", 30 * 60 * 30);
    let log = dump(&p);
    // The first wave's seat is eliminated; ships built for it afterwards still move.
    assert!(p.of("eliminated").next().is_some(), "no seat was eliminated:\n{log}");
    // Only enemy units first seen after the tick of the elimination count, so the first wave
    // cannot satisfy the check.
    let eliminated_at = p.of("eliminated").next().map(|e| e.tick).unwrap();
    let late: std::collections::HashSet<&str> = p
        .of("plan")
        .filter(|e| e.tick > eliminated_at)
        .filter_map(|e| e.what.strip_prefix("first "))
        .collect();
    let acted = p.of("plan").filter_map(|e| e.what.strip_prefix("acted ")).filter(|id| late.contains(id)).count();
    assert!(acted >= 3, "only {acted} enemy units first seen after the elimination acted:\n{log}");
    assert!(p.outcome.is_some_and(|o| o.won), "not won:\n{log}");
}

/// The lose condition of the first of those missions, reached by its losing plan.
#[test]
fn a1_federation_1_is_lost_by_its_losing_plan() {
    let Ok(install) = std::env::var("STA64_TEST_A1_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A1_INSTALL to an Armada I root");
        return;
    };
    let p = play_mission(&install, "fed1", "a1_fed1_lose.lua", 30 * 60 * 20);
    let log = dump(&p);
    assert!(p.outcome.is_some_and(|o| !o.won), "not lost:\n{log}");
}

/// Armada II, the first mission of the first story campaign (STA64's campaign slot `federation`,
/// its first entry; which map that is comes from the install). Its opening sets the relation
/// between the player's team and another team in both directions (SIM-63). Rule under test: with
/// the relation honoured, nothing of the player's seat is destroyed while the opening runs, and
/// the opening does not end the mission.
#[test]
fn a2_first_story_mission_opening_destroys_none_of_the_players_ships() {
    let Ok(install) = std::env::var("STA64_TEST_A2_STOCK_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A2_STOCK_INSTALL to an Armada II (stock) root");
        return;
    };
    let conv = support::convert(Path::new(&install), "playthrough-a2", &[], false);
    let (book, host) = mounted(&install, &conv);
    let first = book.campaign("federation").and_then(|c| c.missions.first()).expect("the campaign has a first mission");
    assert!(conv.run.written().contains(&format!("{}.lua", first.stem().to_ascii_lowercase())), "its script was converted");
    let p = run_plan(&host, &book, "federation", 0, "idle.lua", 30 * 70);
    let log = dump(&p);
    // the opening really ran: the script changed a relation and showed something
    assert!(p.of("present").next().is_some(), "the opening presented nothing:\n{log}");
    let lost: Vec<_> = p.of("destroyed").filter(|e| e.what.starts_with("p0 ")).map(|e| e.what.clone()).collect();
    assert!(lost.is_empty(), "the player's ships were destroyed in the opening: {lost:?}\n{log}");
    assert!(p.outcome.is_none(), "the opening ended the mission:\n{log}");
}
