//! LUA-17: stock campaign missions played to the end by the playthrough driver
//! (`sta_mission::playthrough`), install-gated.
//!
//! Each test converts one mission from the install named by its variable with
//! the repository's converter into the target directory (never into the
//! repository), mounts the output over the install and plays it with a plan
//! from `tests/playthrough/` (a scripted player that gives orders through the
//! normal command interface). It checks the outcome and the order of the
//! briefings (the objective panels the script shows). Needs `python3` and `objdump`; skipped when the
//! variable is not set. Prints the timeline's summary on failure.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sta_mission::campaign::{CampaignBook, VfsHost};
use sta_mission::playthrough::{Config, Playthrough, play};

fn convert(tool: &str, install: &str, only: &str) -> PathBuf {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("playthrough_{only}"));
    let _ = std::fs::remove_dir_all(&out);
    let run = std::process::Command::new("python3")
        .arg("-I")
        .arg(repo.join("tools").join(tool))
        .args(["--install", install, "--only", only, "--out"])
        .arg(&out)
        .output()
        .expect("run python3");
    assert!(run.status.success(), "{}\n{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr));
    out
}

fn play_mission(install: &str, converted: &Path, stem: &str, plan: &str, ticks: u64) -> Playthrough {
    let mut vfs = sta_vfs::Vfs::mount_install(Path::new(install), None).expect("mount install");
    vfs.push_dir("converted".into(), converted.to_path_buf()).expect("mount converted");
    let book = CampaignBook::load_vfs(&vfs);
    let (campaign, index) = book
        .campaigns
        .iter()
        .flat_map(|c| c.missions.iter().map(move |m| (c, m)))
        .find(|(_, m)| m.stem().eq_ignore_ascii_case(stem))
        .map(|(c, m)| (c.id.clone(), m.index))
        .unwrap_or_else(|| panic!("no campaign mission {stem}"));
    let host = VfsHost::load(Arc::new(vfs)).expect("content");
    let plan = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/playthrough").join(plan)).expect("plan");
    play(&host, &book, &campaign, index, &Config { ticks, plan: Some(plan), seed: 1 }).expect("starts")
}

fn dump(p: &Playthrough) -> String {
    p.events
        .iter()
        .filter(|e| !matches!(e.kind, "call" | "camera" | "spawned" | "order" | "step" | "present"))
        .map(|e| format!("{:8.1} {:<12} {}", e.secs(), e.kind, e.what.lines().next().unwrap_or("")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Armada I, Federation 1 "Premonitions": build a starbase and a mining station, destroy the
/// Dominion, then the Borg that follow the Premonition through the gate; the Enterprise survives.
#[test]
fn a1_federation_1_is_won_by_a_competent_player() {
    let Ok(install) = std::env::var("STA64_TEST_A1_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A1_INSTALL to an Armada I root");
        return;
    };
    let out = convert("a1_to_lua.py", &install, "fed1");
    let p = play_mission(&install, &out, "fed1", "a1_fed1.lua", 30 * 60 * 40);
    let log = dump(&p);
    assert!(p.outcome.is_some_and(|o| o.won), "not won:\n{log}");
    // Briefings in order: the opening's, the Dominion attack's, the Premonition's.
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

/// The lose condition of the same mission: the Enterprise is lost.
#[test]
fn a1_federation_1_is_lost_when_the_enterprise_is_destroyed() {
    let Ok(install) = std::env::var("STA64_TEST_A1_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A1_INSTALL to an Armada I root");
        return;
    };
    let out = convert("a1_to_lua.py", &install, "fed1");
    let p = play_mission(&install, &out, "fed1", "a1_fed1_lose.lua", 30 * 60 * 20);
    let log = dump(&p);
    assert!(p.outcome.is_some_and(|o| !o.won), "not lost:\n{log}");
}

/// Armada II, Federation 1 "Invasion": the intro props belong to team 8 and the script sets
/// `SetRelation(1, 8, 2)` and `SetRelation(8, 1, 2)` for the opening (SIM-63). With the relation
/// honoured, nothing of the player's seat is destroyed in the opening (before the fix a team-8
/// starbase and freighter destroyed five ships and the Enterprise inside the intro).
#[test]
fn a2_federation_1_opening_destroys_none_of_the_players_ships() {
    let Ok(install) = std::env::var("STA64_TEST_A2_STOCK_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A2_STOCK_INSTALL to an Armada II (stock) root");
        return;
    };
    let out = convert("dsl_to_lua.py", &install, "a2_fed01");
    let p = play_mission(&install, &out, "a2_fed01", "idle.lua", 30 * 70);
    let log = dump(&p);
    let lost: Vec<_> = p.of("destroyed").filter(|e| e.what.starts_with("p0 ")).map(|e| e.what.clone()).collect();
    assert!(lost.is_empty(), "the player's ships were destroyed in the opening: {lost:?}\n{log}");
    assert!(p.outcome.is_none(), "the opening ended the mission:\n{log}");
}
