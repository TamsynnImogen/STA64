//! Why the menu cannot start a stock mission, and what makes it able to: install-gated
//! (`STA64_TEST_A2_STOCK_INSTALL`, `STA64_TEST_A1_INSTALL`; each test skips without its variable).
//!
//! Without a conversion a campaign mission whose rules are a compiled DLL is blocked as "legacy
//! DLL only" (STA64 never loads the DLL, D2), so the menu lists it as "cannot run yet". After the
//! Rust converter (`sta_mission_convert`, the library the game calls; no Python, no `objdump`) has
//! written the player's conversion into the folder the game looks in
//! ([`sta_mission::converted::dir_in`]) and the game has mounted it
//! ([`sta_mission::converted::mount`]), those missions list as startable.
//!
//! Nothing here names or counts a mission of a game: what to expect is derived from the install
//! and from what the conversion reports, at run time. The conversion goes to a temporary folder
//! that is removed with the test.

#[path = "support/convert.rs"]
mod support;

use std::collections::BTreeSet;
use std::path::Path;

use sta_mission::campaign::*;
use sta_vfs::Vfs;

/// The `.bzn` names (lower case) of the missions the book blocks as needing a conversion.
fn legacy_blocked(book: &CampaignBook) -> BTreeSet<String> {
    book.blocked().iter().filter(|b| b.reason.starts_with("legacy DLL only")).map(|b| b.bzn.to_ascii_lowercase()).collect()
}

/// The missions of `waiting` the conversion wrote a script for: a script is written under the
/// stem of the map it belongs to.
fn covered(waiting: &BTreeSet<String>, written: &BTreeSet<String>) -> BTreeSet<String> {
    waiting.iter().filter(|bzn| written.contains(&format!("{}.lua", bzn.trim_end_matches(".bzn")))).cloned().collect()
}

/// Converts the install into the data folder's place for it, mounts that as the game does, and
/// checks the book before and after.
fn blocked_until_converted(install: &Path, tag: &str) {
    let mut vfs = Vfs::mount_install(install, None).expect("mount install");

    // The cause: no Lua for them, so the missions with only DLL rules are blocked.
    let waiting = legacy_blocked(&CampaignBook::load_vfs(&vfs));
    assert!(!waiting.is_empty(), "the install's campaigns have missions that need a conversion");

    // The conversion, into a data folder of its own, where the game looks for this install.
    let data = support::TempDir::new(tag);
    let dir = sta_mission::converted::dir_in(data.path(), install, None);
    let run = support::convert_into(install, &dir, &[], false);
    assert!(dir.join("missions").is_dir(), "the converter wrote where the game looks");
    let covered = covered(&waiting, &run.written());
    assert!(!covered.is_empty(), "the conversion wrote a script for at least one waiting mission");

    // What the game does at start-up.
    assert!(sta_mission::converted::mount(&mut vfs, &dir));
    let after = CampaignBook::load_vfs(&vfs);
    let still = legacy_blocked(&after);
    eprintln!("{} missions waited for a conversion, {} got a script, {} still wait", waiting.len(), covered.len(), still.len());
    let unexpected: Vec<&String> = covered.intersection(&still).collect();
    assert!(unexpected.is_empty(), "{} mission(s) with a converted script are still blocked as legacy DLL only", unexpected.len());
    assert_eq!(still, waiting.difference(&covered).cloned().collect(), "only the missions without a converted script still wait");
    // A campaign whose first mission got a script can be started.
    for c in &after.campaigns {
        let Some(first) = c.missions.first() else { continue };
        if covered.contains(&first.bzn.to_ascii_lowercase()) {
            assert!(!after.blocked().iter().any(|b| b.campaign == c.id && b.index == 0 && b.reason.starts_with("legacy DLL only")), "a campaign's first mission starts");
        }
    }
}

#[test]
fn stock_a2_campaign_is_blocked_until_converted_then_startable() {
    let Ok(install) = std::env::var("STA64_TEST_A2_STOCK_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A2_STOCK_INSTALL to a stock Armada II root");
        return;
    };
    blocked_until_converted(Path::new(&install), "menu-a2");
}

/// Armada I through the same converter, install-gated on `STA64_TEST_A1_INSTALL`.
#[test]
fn a1_install_converts_through_the_same_converter_and_lists() {
    let Ok(install) = std::env::var("STA64_TEST_A1_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A1_INSTALL to an Armada I root");
        return;
    };
    blocked_until_converted(Path::new(&install), "menu-a1");
}
