//! SIM-39, install-gated: every installed profile's content carries the
//! crew-casualty keys the sim reads (counts of definitions with a non-default
//! value are printed with `--nocapture`). Absent installs are skipped.

use std::path::Path;

use sta_content::{ContentDb, OdfIndex};
use sta_vfs::Vfs;

const HEROIC: &str = "/home/cedric/Games/Heroic";
const ROOTS: &str = "Star Trek Armada II Fleet Operations Roots";

struct Counts {
    units: usize,
    sane: usize,
    protection: usize,
    crew_hit: usize,
    hull_hit: usize,
    sys_hit: usize,
    sys_loss: usize,
    ls_loss: usize,
    recrew: usize,
    variance: usize,
    modifiers: usize,
    hull_mod: usize,
    hull_law: Vec<String>,
    shield_mod: usize,
    shield_law: usize,
    crew_per_sec: sta_core::Fixed,
    thresholds: usize,
    cfg_protection: Option<f32>,
    facing: (Option<f32>, Option<i64>),
}

fn load(dir: &str, modname: Option<&str>) -> Option<(String, Counts)> {
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
    let (db, _) = ContentDb::load(&vfs, &index).ok()?;
    let nz = |f: sta_core::Fixed| f.to_bits() != 0;
    let mut c = Counts { units: 0, sane: 0, protection: 0, crew_hit: 0, hull_hit: 0, sys_hit: 0, sys_loss: 0, ls_loss: 0, recrew: 0, variance: 0, modifiers: 0, hull_mod: 0, hull_law: Vec::new(), shield_mod: 0, shield_law: 0, crew_per_sec: sta_core::Fixed::ZERO, thresholds: 0, cfg_protection: None, facing: (None, None) };
    for u in db.units() {
        c.units += 1;
        let k = &u.casualty;
        c.sane += usize::from(u.sane_crew_loss);
        c.protection += usize::from(k.shield_protection.is_some());
        c.crew_hit += usize::from(nz(k.crew_hit_percent));
        c.hull_hit += usize::from(k.hull_hit_percent.is_some());
        c.sys_hit += usize::from(k.system_hit_percent.iter().any(|f| nz(*f)));
        c.sys_loss += usize::from(k.system_crew_loss.iter().any(|f| nz(*f)));
        c.ls_loss += usize::from(nz(k.life_support_loss));
        c.recrew += usize::from(k.recrew_rate.is_some());
    }
    for w in db.weapons() {
        c.variance += usize::from(nz(w.damage_variance));
        if let Some(l) = w.crew_loss {
            c.modifiers += usize::from(nz(l.shield) || nz(l.hull));
            if nz(l.hull) && nz(w.damage) {
                c.hull_law.push(w.odf.clone());
            }
            c.hull_mod += usize::from(nz(l.hull));
            c.shield_mod += usize::from(nz(l.shield));
            // The shield crew law fires only with a threshold and a modifier.
            c.shield_law += usize::from(nz(l.threshold) && nz(l.shield));
            c.thresholds += usize::from(nz(l.threshold));
        }
    }
    let cfg = &db.rules().cfg;
    c.cfg_protection = cfg.shield_damage_protection.map(|f| f.to_f32());
    c.crew_per_sec = cfg.yard_crew_per_sec();
    c.facing = (cfg.facing_weight.map(|f| f.to_f32()), cfg.facing_constant);
    Some((format!("{:?}", db.profile()), c))
}

#[test]
fn every_installed_profile_loads_the_casualty_keys() {
    let cases = [
        ("A1", "Star Trek Armada", None),
        ("A2", "Star Trek Armada II (Stock)", None),
        ("Classic", ROOTS, Some("STA2 Classic Mod")),
        ("Classic Enhanced", ROOTS, Some("STA2 Classic Mod Enhanced Gameplay")),
        ("FO 4.0", ROOTS, Some("Fleet Ops 4.0")),
        ("FO 3.2.7", "Star Trek Armada II Fleet Operations", None),
        ("DW", "Star Trek Armada II Dominion War", None),
        ("Roots", ROOTS, None),
    ];
    for (label, dir, modname) in cases {
        let Some((profile, c)) = load(dir, modname) else { continue };
        eprintln!(
            "{label} ({profile}): units {} sane {} shieldProtection {} crewHit {} hullHit {} sysHit {} sysCrewLoss {} lifeSupportLoss {} recrewRate {} ordnance: variance {} modifiers {} thresholds {} (hull-mod {} shield-mod {} shield-law {}; damaging hull-law {:?}); cfg protection {:?} facing {:?}",
            c.units, c.sane, c.protection, c.crew_hit, c.hull_hit, c.sys_hit, c.sys_loss, c.ls_loss, c.recrew, c.variance, c.modifiers, c.thresholds, c.hull_mod, c.shield_mod, c.shield_law, c.hull_law, c.cfg_protection, c.facing,
        );
        assert!(c.units > 0, "{label}");
        let want = match label {
            "A1" => "Sta1Classic",
            "A2" => "Sta2Classic",
            "FO 3.2.7" | "DW" => "Fo3",
            _ => "Fo4",
        };
        assert_eq!(profile, want, "{label}: detected profile");
        match label {
            "A1" => assert!(c.sane == 0 && c.crew_hit == 0, "A1 has neither saneCrewLossMethod nor crewHitPercent"),
            "A2" | "Classic" => {
                assert_eq!(c.cfg_protection, Some(0.8), "{label}");
                assert_eq!(c.facing, (Some(0.2), Some(3)), "{label}");
                assert_eq!(c.crew_per_sec, sta_core::Fixed::from_int(20), "{label}");
                assert!(c.shield_law > 0 && c.hull_mod > 0, "{label}: the laws have data to act on");
                assert!(c.sane >= 20 && c.crew_hit >= 15 && c.protection >= 5 && c.variance > 20 && c.modifiers > 40, "{label}");
            }
            "Classic Enhanced" => {
                assert_eq!(c.cfg_protection, Some(0.9));
                assert_eq!(c.crew_per_sec, sta_core::Fixed::from_int(30));
            }
            "FO 3.2.7" | "DW" | "Roots" | "FO 4.0" => {
                // PR #652: the stock laws run, but FO/Roots data zeroes their keys.
                // The shield law needs `shieldProtection` below 1; FO data has none.
                assert!(c.protection == 0 && c.cfg_protection == Some(1.0), "{label}: shield bleed-through is possible");
                // The hull-location law has a few damaging ordnance with a modifier
                // (found 2026-11-13: `fed_magnana`, `kli_overload` in FO 3.2.7 / 4.0, plus
                // `card_ion_storm`, `card_overload`, `gchainp` in Dominion War); any other
                // would add a percent-of-crew loss to each crew-location hit.
                let extra: Vec<_> = c.hull_law.iter().filter(|n| !["fed_magnana", "kli_overload", "card_ion_storm", "card_overload", "gchainp"].contains(&n.as_str())).collect();
                assert!(extra.is_empty(), "{label}: ordnance with a hull crew modifier: {extra:?}");
                assert_eq!(c.crew_per_sec, sta_core::Fixed::from_int(20), "{label}");
                assert_eq!(c.cfg_protection, Some(1.0), "{label}");
                assert!(c.crew_hit > 100, "{label}: crewHitPercent is set on FO units");
            }
            _ => {}
        }
    }
}
