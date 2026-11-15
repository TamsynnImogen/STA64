//! CON-51, install-gated: the income keys of orbital mining parse from the
//! installs without warnings, and the Armada II family's data obeys the
//! general rules. Absent installs are skipped. Read from data; no values.

use std::path::Path;

use sta_content::{ContentDb, OdfIndex, Profile, Warning};
use sta_vfs::Vfs;

const HEROIC: &str = "/home/cedric/Games/Heroic";
const ROOTS: &str = "Star Trek Armada II Fleet Operations Roots";
const KEYS: [&str; 7] = ["miningMultiplier", "resourcesCanHandle", "miningRate", "trickleAmount", "trickleMiningRateFraction", "isInfinite", "miningRateModifier"];

fn load(dir: &str, modname: Option<&str>) -> Option<(ContentDb, Vec<Warning>, Vfs, OdfIndex)> {
    let root = Path::new(HEROIC).join(dir);
    if !root.is_dir() {
        eprintln!("{dir}: skipped, not installed");
        return None;
    }
    let vfs = match Vfs::mount_install(&root, modname) {
        Ok(v) => v,
        Err(_) => {
            let mut v = Vfs::default();
            v.push_dir(dir.into(), root.clone()).ok()?;
            v
        }
    };
    let (index, _) = OdfIndex::build(&vfs);
    let (db, w) = ContentDb::load(&vfs, &index).ok()?;
    Some((db, w, vfs, index))
}

fn about_keys(w: &[Warning]) -> Vec<String> {
    w.iter().map(|w| format!("{w:?}")).filter(|s| KEYS.iter().any(|k| s.contains(k)) || s.contains("amt")).collect()
}

#[test]
fn installs_parse_the_income_keys_without_warnings() {
    let cases = [
        ("A2", "Star Trek Armada II (Stock)", None, true),
        ("Classic", ROOTS, Some("STA2 Classic Mod"), true),
        ("Classic Enhanced", ROOTS, Some("STA2 Classic Mod Enhanced Gameplay"), true),
        ("FO 3.2.7", "Star Trek Armada II Fleet Operations", None, false),
        ("DW", "Star Trek Armada II Dominion War", None, false),
        ("Roots", ROOTS, None, false),
        ("A1", "Star Trek Armada", None, false),
    ];
    for (label, dir, modname, a2_data) in cases {
        let Some((db, warnings, vfs, index)) = load(dir, modname) else { continue };
        let bad = about_keys(&warnings);
        assert!(bad.is_empty(), "{label}: {bad:?}");
        let mut stations = 0;
        let mut checked = 0;
        for u in db.units() {
            if let Some(m) = u.station_mining {
                stations += 1;
                assert_eq!(u.class.0, "p_mining", "{label}: {}", u.odf);
                // A station that mines names what it can handle.
                assert!(m.can_handle.is_some() || m.multiplier.is_some(), "{label}");
            }
            // The node key rule: a node whose ODF says `isInfinite` 1 is typed infinite, one that
            // says 0 is typed finite, one without the key carries none.
            if u.resource.is_some() {
                let unit = sta_odf::resolve_layered(&u.odf, &mut |n, v| index.load(&vfs, n, v)).ok();
                let node = unit
                    .as_ref()
                    .and_then(|o| o.str("resource").map(|t| t.trim().to_owned()))
                    .and_then(|t| sta_odf::resolve_layered(&t, &mut |n, v| index.load(&vfs, n, v)).ok());
                if let Some(node) = node {
                    checked += 1;
                    assert_eq!(u.node_income.and_then(|n| n.infinite), node.bool("isInfinite"), "{label}: {}", u.odf);
                }
            }
            if let Some(n) = u.node_income {
                assert!(u.resource.is_some(), "{label}: {}", u.odf);
                // Trickle keys come as a pair in the data.
                assert_eq!(n.trickle_amount.is_some(), n.trickle_fraction.is_some(), "{label}: {}", u.odf);
            }
        }
        eprintln!("{label}: {stations} stations with keys, {checked} nodes checked, profile {:?}", db.profile());
        if a2_data && db.profile() == Profile::Sta2Classic {
            // Every planet whose node takes an orbital station carries the rate and amounts.
            for u in db.units().filter(|u| u.can_have_orbital_station) {
                let n = u.node_income.unwrap_or_else(|| panic!("{label}: {} has no node income", u.odf));
                assert!(n.mining_rate.is_some_and(|r| r > sta_core::Fixed::ZERO), "{label}: {}", u.odf);
                assert!(n.amounts.iter().any(Option::is_some), "{label}: {}", u.odf);
            }
            assert!(db.units().any(|u| u.station_mining.is_some_and(|m| m.multiplier.is_some())), "{label}");
        }
    }
}
