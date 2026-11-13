//! Why the menu cannot start a stock Armada II mission, and what makes it able to:
//! install-gated (`STA64_TEST_A2_STOCK_INSTALL`; needs `python3` and `objdump`).
//!
//! Without a conversion every campaign mission of the stock install is blocked as
//! "legacy DLL only" (its rules are a compiled DLL STA64 never loads, D2), so the
//! menu lists it as "cannot run yet". After `tools/convert_install_missions.py` has
//! written the user's conversion and the game has mounted it
//! ([`sta_mission::converted::mount`]), the stock campaigns list as startable.
//! The conversion goes under the target directory (never the repository).

use std::path::{Path, PathBuf};

use sta_mission::campaign::*;
use sta_vfs::Vfs;

#[test]
fn stock_a2_campaign_is_blocked_until_converted_then_startable() {
    let Ok(install) = std::env::var("STA64_TEST_A2_STOCK_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A2_STOCK_INSTALL to a stock Armada II root");
        return;
    };
    let install = Path::new(&install);
    let mut vfs = Vfs::mount_install(install, None).expect("mount install");

    // The cause: no Lua anywhere, so every mission with only DLL rules is blocked.
    let before = CampaignBook::load_vfs(&vfs);
    let total: usize = before.campaigns.iter().map(|c| c.missions.len()).sum();
    let blocked = before.blocked();
    let legacy = blocked.iter().filter(|b| b.reason.starts_with("legacy DLL only")).count();
    eprintln!("before: {total} missions, {} blocked, {legacy} legacy DLL only", blocked.len());
    assert!(legacy >= 30, "the stock campaigns need a conversion: {legacy} of {total}");

    // The one-command conversion, into a data dir of its own.
    let data: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR")).join("converted_menu_data");
    let _ = std::fs::remove_dir_all(&data);
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let run = std::process::Command::new("python3")
        .arg("-I")
        .arg(repo.join("tools/convert_install_missions.py"))
        .arg("--install")
        .arg(install)
        .env("STA64_DATA_DIR", &data)
        .output()
        .expect("run python3 tools/convert_install_missions.py");
    assert!(run.status.success(), "{}\n{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr));
    let dir = sta_mission::converted::dir_in(&data, install, None);
    assert!(dir.join("missions").is_dir(), "the tool wrote where the game looks: {}", dir.display());

    // What the game does at start-up.
    assert!(sta_mission::converted::mount(&mut vfs, &dir));
    let after = CampaignBook::load_vfs(&vfs);
    let blocked_after = after.blocked();
    eprintln!("after: {} blocked of {total}", blocked_after.len());
    for b in &blocked_after {
        eprintln!("  still blocked: {} {} {}", b.campaign, b.bzn, b.reason);
    }
    for id in ["federation", "klingon", "borg"] {
        let c = after.campaign(id).unwrap_or_else(|| panic!("{id} campaign"));
        assert!(!c.missions.is_empty(), "{id}");
        assert!(
            !blocked_after.iter().any(|b| b.campaign == id && b.index == 0),
            "{id} mission 1 starts: {:?}",
            blocked_after.iter().find(|b| b.campaign == id && b.index == 0)
        );
    }
    assert!(blocked_after.is_empty(), "every stock mission is startable after conversion (measured: 36 of 36 with LUA-11/14/15 merged)");
    let _ = std::fs::remove_dir_all(&data);
}

/// Armada I through the same command (`tools/a1_to_lua.py`), install-gated on
/// `STA64_TEST_A1_INSTALL`. Prints how many missions the book can start after the conversion.
#[test]
fn a1_install_converts_through_the_same_command_and_lists() {
    let Ok(install) = std::env::var("STA64_TEST_A1_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A1_INSTALL to an Armada I root");
        return;
    };
    let install = Path::new(&install);
    let mut vfs = Vfs::mount_install(install, None).expect("mount install");
    let before = CampaignBook::load_vfs(&vfs);
    let total: usize = before.campaigns.iter().map(|c| c.missions.len()).sum();
    let data: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR")).join("converted_menu_a1_data");
    let _ = std::fs::remove_dir_all(&data);
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let run = std::process::Command::new("python3")
        .arg("-I")
        .arg(repo.join("tools/convert_install_missions.py"))
        .arg("--install")
        .arg(install)
        .env("STA64_DATA_DIR", &data)
        .output()
        .expect("run the converter");
    assert!(run.status.success(), "{}\n{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr));
    let dir = sta_mission::converted::dir_in(&data, install, None);
    assert!(sta_mission::converted::mount(&mut vfs, &dir), "{}", dir.display());
    let after = CampaignBook::load_vfs(&vfs);
    let blocked = after.blocked();
    eprintln!("A1: {total} missions, {} blocked after conversion", blocked.len());
    for b in &blocked {
        eprintln!("  blocked: {} {} {}", b.campaign, b.bzn, b.reason);
    }
    assert!(blocked.len() < before.blocked().len(), "the conversion unblocks some A1 missions");
    let _ = std::fs::remove_dir_all(&data);
}
