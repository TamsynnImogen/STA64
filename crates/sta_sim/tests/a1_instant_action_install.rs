//! Install-gated (`STA64_TEST_A1_INSTALL`, an Armada I root; skipped when
//! unset): an Instant Action start on stock Armada I gives every seat the
//! header's start. Armada I's race ODFs author no `normal*` stock, so the seat
//! takes `cfgStartingDilithium` and `cfgStartingCrew` from `RTS_CFG.h`, and
//! `cfgMaxOfficers` is the officer cap; latinum, metal and biomatter stay
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
    let cap = db.rules().officer_limit(0).expect("cfgMaxOfficers");
    assert!(dilithium > 0 && crew > 0 && cap > 0);
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
            assert_eq!(p.officers[1], Fixed::from_int(cap), "{race} seat {} officer cap", id.0);
        }
        seen += 1;
    }
    assert!(seen > 0, "no playable race");
}
