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
    let mut c = Counts { units: 0, sane: 0, protection: 0, crew_hit: 0, hull_hit: 0, sys_hit: 0, sys_loss: 0, ls_loss: 0, recrew: 0, variance: 0, modifiers: 0, thresholds: 0, cfg_protection: None, facing: (None, None) };
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
            c.thresholds += usize::from(nz(l.threshold));
        }
    }
    let cfg = &db.rules().cfg;
    c.cfg_protection = cfg.shield_damage_protection.map(|f| f.to_f32());
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
        ("FO 3.2.7", "Star Trek Armada II Fleet Operations", None),
        ("DW", "Star Trek Armada II Dominion War", None),
        ("Roots", ROOTS, None),
    ];
    for (label, dir, modname) in cases {
        let Some((profile, c)) = load(dir, modname) else { continue };
        eprintln!(
            "{label} ({profile}): units {} sane {} shieldProtection {} crewHit {} hullHit {} sysHit {} sysCrewLoss {} lifeSupportLoss {} recrewRate {} ordnance: variance {} modifiers {} thresholds {}; cfg protection {:?} facing {:?}",
            c.units, c.sane, c.protection, c.crew_hit, c.hull_hit, c.sys_hit, c.sys_loss, c.ls_loss, c.recrew, c.variance, c.modifiers, c.thresholds, c.cfg_protection, c.facing,
        );
        assert!(c.units > 0, "{label}");
        match label {
            "A1" => assert!(c.sane == 0 && c.crew_hit == 0, "A1 has neither saneCrewLossMethod nor crewHitPercent"),
            "A2" | "Classic" => {
                assert_eq!(c.cfg_protection, Some(0.8), "{label}");
                assert_eq!(c.facing, (Some(0.2), Some(3)), "{label}");
                assert!(c.sane >= 20 && c.crew_hit >= 15 && c.protection >= 5 && c.variance > 20 && c.modifiers > 40, "{label}");
            }
            "Classic Enhanced" => assert_eq!(c.cfg_protection, Some(0.9)),
            "FO 3.2.7" | "DW" | "Roots" => {
                assert_eq!(c.cfg_protection, Some(1.0), "{label}");
                assert!(c.crew_hit > 100, "{label}: crewHitPercent is set on FO units");
            }
            _ => {}
        }
    }
}
