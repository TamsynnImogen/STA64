//! SIM-24, install-gated: on every installed Fleet Operations lineage, a unit
//! carrying `resourceweapon` hardpoints raises its owner's stock by exactly the
//! ODF amounts per second (time base: measured on the original Roots, see the
//! PR). Installs: the Heroic folders, plus `STA64_TEST_INSTALL`
//! (+ `STA64_TEST_MOD`). Absent or unmountable installs are skipped.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sta_content::{ContentDb, MapDef, MapObject, OdfIndex};
use sta_core::{Fixed, PlayerId, TeamId, Vec3};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, WorldSource};
use sta_vfs::Vfs;

const HEROIC: &str = "/home/cedric/Games/Heroic";

fn installs() -> Vec<(String, PathBuf, Option<String>)> {
    let roots = || Path::new(HEROIC).join("Star Trek Armada II Fleet Operations Roots");
    let mut v = vec![
        ("Roots".to_owned(), roots(), None),
        ("FO4".to_owned(), roots(), Some("Fleet Ops 4.0".to_owned())),
        ("FO".to_owned(), Path::new(HEROIC).join("Star Trek Armada II Fleet Operations"), None),
        ("A2".to_owned(), Path::new(HEROIC).join("Star Trek Armada II (Stock)"), None),
    ];
    if let Ok(root) = std::env::var("STA64_TEST_INSTALL") {
        v.push(("env".into(), PathBuf::from(root), std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty())));
    }
    v
}

fn seat(id: u8) -> PlayerSetup {
    PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "x".into(), control: Control::Human }
}

#[test]
fn installed_resource_weapons_pay_their_odf_amounts_per_second() {
    let mut carriers_checked = 0;
    for (label, root, active) in installs() {
        if !root.is_dir() {
            continue;
        }
        let Ok(vfs) = Vfs::mount_install(&root, active.as_deref()) else { continue };
        let (index, _) = OdfIndex::build(&vfs);
        let Ok((db, _)) = ContentDb::load(&vfs, &index) else { continue };
        let db = Arc::new(db);
        let carriers: Vec<_> = db
            .units()
            .filter(|u| {
                u.hardpoints.iter().any(|h| {
                    db.weapon(h.weapon.key).and_then(|w| w.resource).is_some_and(|r| !r.one_time && r.always_player)
                })
            })
            .collect();
        eprintln!("{label}: {} carrier units of always-active resource weapons", carriers.len());
        for u in carriers.iter() {
            let mut map = MapDef::empty();
            map.objects = vec![MapObject {
                odf: u.odf.clone(),
                owner: Some(0),
                pos: Vec3::ZERO,
                heading: (Fixed::ZERO, Fixed::ONE),
                label: None,
            }];
            let mut sim = Sim::from_setup(&GameSetup {
                seed: 1,
                map: Arc::new(map),
                content: Arc::clone(&db),
                players: vec![seat(0), seat(1)],
                options: GameOptions::default(),
            });
            // SIM-28: hidden carriers with a lifetime or an instant replacement end early and
            // may hand over to successors (chains), so only the other-seat check applies to them.
            let ends_early = u.life.is_some()
                || u.hardpoints.iter().filter_map(|h| db.weapon(h.weapon.key).and_then(|w| w.replace.as_ref())).any(|r| {
                    r.fires_for(sta_content::lifecycle::Owner::Player) && r.required_officers.is_none()
                });
            let mut want = [Fixed::ZERO; 4];
            let mut per_tick = 0i64;
            for h in &u.hardpoints {
                if let Some(r) = db.weapon(h.weapon.key).and_then(|w| w.resource).filter(|r| !r.one_time && r.always_player) {
                    for (w, a) in want.iter_mut().zip(r.add) {
                        *w = *w + Fixed::from_bits(a.to_bits() * 60);
                        per_tick += a.to_bits().abs() / 30 + 1;
                    }
                }
            }
            for _ in 0..60 * 30 {
                sim.step(&[]);
            }
            let got = sim.view().player(PlayerId(0)).map(|p| p.resources.clone()).unwrap_or_default();
            let got = |s: usize| got.get(s).copied().unwrap_or(Fixed::ZERO);
            // A starting stock is not set here, so negative adders clamp at zero.
            if !ends_early && want.iter().all(|w| *w >= Fixed::ZERO) {
                for (s, w) in want.iter().enumerate() {
                    let diff = (got(s).to_bits() - w.to_bits()).abs();
                    assert!(diff <= per_tick * 2, "{label} {} slot {s}: got {:?}, want {w:?}", u.odf, got(s));
                }
            }
            let other = sim.view().player(PlayerId(1)).map(|p| p.resources.clone()).unwrap_or_default();
            assert!(other.iter().all(|r| *r == Fixed::ZERO), "{label} {}: the other seat gained {other:?}", u.odf);
            carriers_checked += 1;
        }
    }
    eprintln!("{carriers_checked} carriers checked");
}
