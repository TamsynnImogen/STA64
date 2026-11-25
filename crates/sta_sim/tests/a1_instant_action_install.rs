//! Install-gated (`STA64_TEST_A1_INSTALL`, an Armada I root; skipped when
//! unset): an Instant Action start on stock Armada I gives every seat the
//! header's start. Armada I's race ODFs author no `normal*` stock, so the seat
//! takes `cfgStartingDilithium` and `cfgStartingCrew` from `RTS_CFG.h`, and
//! `cfgMaxOfficers` supplies the initial officer allowance; latinum, metal and biomatter stay
//! empty. Keys read from the install; the amounts are not measured on a
//! running Armada I.

use std::path::Path;
use std::sync::Arc;

use sta_content::{ContentDb, OdfIndex, Profile, Races};
use sta_core::{Fixed, PlayerId, TeamId};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, WorldSource};
use sta_vfs::Vfs;

#[test]
fn every_armada_one_seat_starts_with_the_headers_dilithium_crew_and_officer_cap() {
    let Ok(root) = std::env::var("STA64_TEST_A1_INSTALL") else { return };
    let vfs = Vfs::mount_install(Path::new(&root), None).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    assert_eq!(db.profile(), Profile::Sta1Classic, "STA64_TEST_A1_INSTALL must name an Armada I install");
    let db = Arc::new(db);
    let (races, _) = Races::load(&vfs, &index);
    let header = db.rules().legacy_start();
    let (dilithium, crew) = (header.dilithium.expect("cfgStartingDilithium"), header.crew.expect("cfgStartingCrew"));
    let cap = db.a1_starting_officers().expect("cfgMaxOfficers");
    assert!(dilithium > 0 && crew > 0);
    let mut seen = 0;
    for race in sta_content::race::skirmish_races(&races, &db) {
        let def = races.get_playable(&race).or_else(|| races.get(&race)).unwrap();
        assert!(def.stock.iter().all(|n| *n == 0) && def.normal_crew.is_none(), "{race} authors its own start; the header is not the source");
        let seat = |id: u8, control: Control| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: race.clone(), control };
        let setup = GameSetup {
            seed: 3,
            map: Arc::new(sta_content::fake::two_bases()),
            content: db.clone(),
            players: vec![seat(0, Control::Human), seat(1, Control::Ai("test".into()))],
            options: GameOptions::default(),
        };
        let (mut sim, _) = Sim::from_setup_with_starts(&setup, &races);
        for _ in 0..sta_core::TICK_HZ {
            sim.step(&[]);
        }
        let view = sim.view();
        for id in [PlayerId(0), PlayerId(1)] {
            let p = view.player(id).unwrap();
            let stock: Vec<i64> = p.resources.iter().map(|f| f.to_bits() / Fixed::ONE.to_bits()).collect();
            assert_eq!(stock, [dilithium, 0, 0, 0], "{race} seat {} stock", id.0);
            assert_eq!(p.crew, Some(Fixed::from_int(crew as i32)), "{race} seat {} crew", id.0);
            let base_gain: i32 = view.units.iter().filter(|unit| unit.owner == id)
                .filter_map(|unit| db.unit(unit.def)?.a1_base_officers)
                .map(|base| base.gain).sum();
            assert_eq!(p.officers[1], Fixed::from_int(cap + base_gain), "{race} seat {} officer cap", id.0);
        }
        seen += 1;
    }
    assert!(seen > 0, "no playable race");
}

/// A1Compat fills the missing Armada I race starts in the current Roots
/// install. The source has no SHOWMETHEMONEY_* overrides, so its documented
/// built-in is 10,000 per resource and crew; Lots is 1.5x normal.
#[test]
fn a1_compat_instant_action_starts_with_compatibility_defaults_at_tick_zero() {
    let Ok(root) = std::env::var("STA64_TEST_A1_COMPAT_INSTALL") else { return };
    let active_mod = std::env::var("STA64_TEST_A1_MOD").unwrap_or_else(|_| "STA1 Classic".into());
    let vfs = Vfs::mount_install(Path::new(&root), Some(&active_mod)).unwrap();
    assert!(sta_content::profile::a1_compat(&vfs), "{active_mod:?} must require A1Compat");
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let db = Arc::new(db);
    let (races, _) = Races::load(&vfs, &index);
    let playable = sta_content::race::skirmish_races(&races, &db);
    assert!(!playable.is_empty(), "{active_mod:?} has no playable skirmish races");

    for (mode, amount) in [
        (sta_content::StartingResources::Normal, 10_000),
        (sta_content::StartingResources::Lots, 15_000),
    ] {
        for race in &playable {
            let seat = |id: u8, control: Control| PlayerSetup {
                id: PlayerId(id),
                team: TeamId(id),
                race: race.clone(),
                control,
            };
            let mut options = GameOptions::default();
            options.starting_resources = mode;
            let setup = GameSetup {
                seed: 3,
                map: Arc::new(sta_content::fake::two_bases()),
                content: db.clone(),
                players: vec![seat(0, Control::Human), seat(1, Control::Ai("test".into()))],
                options,
            };
            let (sim, _) = Sim::from_setup_with_starts(&setup, &races);
            let view = sim.view();
            assert_eq!(view.tick, 0);
            let baseline = db.a1_starting_officers().unwrap_or(0);
            for id in [PlayerId(0), PlayerId(1)] {
                let player = view.player(id).unwrap();
                let stock: Vec<i64> = player.resources.iter().map(|f| f.to_bits() / Fixed::ONE.to_bits()).collect();
                assert_eq!(stock, [amount, 0, amount, 0], "{race} {mode:?} seat {} stock", id.0);
                assert_eq!(player.crew, Some(Fixed::from_int(amount as i32)), "{race} {mode:?} seat {} crew", id.0);
                let living = view.units.iter().filter(|unit| unit.owner == id)
                    .filter_map(|unit| db.unit(unit.def))
                    .fold(Fixed::ZERO, |sum, unit| sum + unit.build.officers.max(Fixed::ZERO));
                let base_gain: i32 = view.units.iter().filter(|unit| unit.owner == id)
                    .filter_map(|unit| db.unit(unit.def)?.a1_base_officers)
                    .map(|base| base.gain).sum();
                let expected = [living, Fixed::from_int(baseline + base_gain)];
                assert_eq!(player.officers, expected, "{race} {mode:?} seat {} officers", id.0);
            }
        }
    }
}

/// Exercises the normal content loader for the Federation special slots in a
/// real Armada I archive. The test ships and weapons come from that install.
#[test]
fn federation_specials_load_from_the_armada_one_install() {
    use sta_content::A1SpecialKind as K;
    use sta_sim::specials::{SpecialKind, specials_indexed};

    let Ok(root) = std::env::var("STA64_TEST_A1_INSTALL") else { return };
    let vfs = Vfs::mount_install(Path::new(&root), None).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    assert_eq!(db.profile(), Profile::Sta1Classic);
    for (ship, weapon, kind, rail) in [
        ("fdestroy", "gamine", K::Mines, 1),
        ("fcruise1", "gchainp", K::ChainPulsar, 2),
        ("fcruise2", "gwcover", K::EngineOverload, 3),
        ("fbattle", "gcorbom", K::Reflector, 4),
        ("fscout", "gdetectf", K::DetectCloak, 0),
        ("fsensor", "gdetectf", K::DetectCloak, 0),
        ("fspecial", "gmanheim", K::Gemini, 4),
        ("fspecial", "gshieldh", K::ShieldDisruptor, 2),
        ("fspecial", "gfedrep", K::EngineeringTeam, 3),
        ("fspecial", "gpdlaser", K::PointDefense, 1),
        ("fsuperbl", "gtempor", K::Stasis, 5),
    ] {
        let def = db.unit(db.key(ship).unwrap()).unwrap();
        let key = db.key(weapon).unwrap();
        let (_, special, w) = specials_indexed(&db, def)
            .find(|(_, _, w)| w.key == key)
            .unwrap_or_else(|| panic!("{ship} lacks {weapon}"));
        assert_eq!(w.a1_special.unwrap().kind, kind);
        assert_eq!(w.raw.i64("buttonSlot"), Some(rail));
        assert!(w.energy_cost > Fixed::ZERO, "{weapon}");
        assert_eq!(special, if kind == K::PointDefense { SpecialKind::PointDefence } else { SpecialKind::A1(kind) });
    }
}
