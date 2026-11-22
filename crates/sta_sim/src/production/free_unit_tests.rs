//! The free unit of a mining station ([`super::super::free_unit`]), on authored data.

use super::*;
use sta_content::{LaunchPoint, StationMining};

const STATION_CREW: i32 = 200;
const FREE_CREW: i32 = 50;

/// `fx_station` with `free` as its `freighterName` (dock node 30 ahead, facing +x)
/// and crews set; `fx_miner` costs `FREE_CREW`.
fn content_free(free: Option<&str>, dock: bool) -> Arc<ContentDb> {
    let base = content(true);
    let mut b = ContentDb::builder().profile(base.profile()).features(base.features().clone());
    for w in base.weapons() {
        b = b.weapon(w.clone());
    }
    for u in base.units() {
        let mut u = u.clone();
        if u.odf.eq_ignore_ascii_case("fx_station") {
            u.free_unit = free.map(str::to_owned);
            u.crew = Fixed::from_int(STATION_CREW);
            u.maximum_crew = Some(Fixed::from_int(STATION_CREW));
            if dock {
                u.dock = Some(LaunchPoint {
                    offset: (Fixed::ZERO, Fixed::from_int(30)),
                    height: Fixed::ZERO,
                    dir: (Fixed::ONE, Fixed::ZERO),
                    clear: Fixed::from_int(60),
                    node: Some("dock".into()),
                });
            }
        }
        if u.odf.eq_ignore_ascii_case("fx_miner") {
            u.crew = Fixed::from_int(FREE_CREW);
            u.maximum_crew = Some(Fixed::from_int(FREE_CREW));
        }
        b = b.unit(u);
    }
    Arc::new(b.build().unwrap())
}

fn game_with(content: Arc<ContentDb>) -> Sim {
    let seat = |id| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control: Control::Human };
    Sim::from_setup(&GameSetup {
        seed: 3,
        map: Arc::new(sta_content::MapDef::empty()),
        content,
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    })
}

/// Build the station with a rig and step until it completes; returns (sim, station).
fn built_station(content: Arc<ContentDb>) -> (Sim, EntityId) {
    let mut sim = game_with(content);
    let rig = spawn(&mut sim, "fx_rig", v(0, 0));
    fund(&mut sim, 1500);
    assert!(build(&mut sim, rig, "fx_station", Some(v(10, 0))));
    let station = EntityId(rig.0 + 1);
    for _ in 0..1799 {
        sim.step(&[]);
        assert!(sim.unit(station).unwrap().progress.is_some());
    }
    (sim, station)
}

fn miners(sim: &Sim) -> Vec<EntityId> {
    let k = key(sim, "fx_miner");
    sim.units.iter().filter(|e| e.def == k).map(|e| e.id).collect()
}

fn content_with_mining_mask(mask: [bool; 4]) -> Arc<ContentDb> {
    let base = content_free(Some("fx_miner"), true);
    let mut b = ContentDb::builder().profile(base.profile()).features(base.features().clone());
    for w in base.weapons() {
        b = b.weapon(w.clone());
    }
    for u in base.units() {
        let mut u = u.clone();
        if u.odf == "fx_station" {
            u.station_mining = Some(StationMining { multiplier: None, can_handle: Some(mask) });
        }
        b = b.unit(u);
    }
    let mut metal = UnitDef::new("fx_metal_moon", "planet");
    metal.resource = Some(2);
    Arc::new(b.unit(metal).build().unwrap())
}

#[test]
fn free_freighter_seeks_nearest_resource_the_station_accepts() {
    let (mut sim, station) = built_station(content_with_mining_mask([false, false, true, false]));
    let db = sim.content().unwrap().clone();
    let heading = (Fixed::ZERO, Fixed::ONE);
    let near_dilithium = sim.spawn_def(db.key("fx_moon").unwrap(), PlayerId(255), v(100, 0), heading).unwrap();
    let far_metal = sim.spawn_def(db.key("fx_metal_moon").unwrap(), PlayerId(255), v(500, 0), heading).unwrap();
    assert!(near_dilithium < far_metal);
    sim.step(&[]);
    let miner = miners(&sim)[0];
    assert!(sim.economy.is_mining(miner));
    assert!(sim.economy.to_rec().miners.iter().any(|row| row.0 == miner.0 && row.1 == far_metal.0));
    assert_eq!(sim.unit(station).unwrap().progress, None);
}

#[test]
fn free_freighter_stays_idle_for_an_authored_empty_resource_mask() {
    let (mut sim, _) = built_station(content_with_mining_mask([false; 4]));
    let db = sim.content().unwrap().clone();
    sim.spawn_def(db.key("fx_moon").unwrap(), PlayerId(255), v(100, 0), (Fixed::ZERO, Fixed::ONE)).unwrap();
    sim.step(&[]);
    assert!(!sim.economy.is_mining(miners(&sim)[0]));
}

#[test]
fn completed_station_gets_exactly_one_free_unit_for_its_owner_at_the_dock() {
    let (mut sim, station) = built_station(content_free(Some("fx_miner"), true));
    assert!(miners(&sim).is_empty());
    let out = sim.step(&[]);
    assert_eq!(sim.unit(station).unwrap().progress, None);
    let m = miners(&sim);
    assert_eq!(m.len(), 1);
    let u = sim.unit(m[0]).unwrap();
    assert_eq!(u.owner, P0);
    // Dock node (0, 30) in the station's frame, station heading (0, 1) at (10, 0).
    assert_eq!(u.pos, v(10, 30));
    assert_eq!(u.heading, (Fixed::ONE, Fixed::ZERO));
    assert!(out.events.contains(&SimEvent::Spawned { id: m[0] }));
    // Its crew leaves the station's; nothing is charged.
    assert_eq!(sim.unit(station).unwrap().crew.unwrap()[0], Fixed::from_int(STATION_CREW - FREE_CREW));
    assert_eq!(sim.unit(m[0]).unwrap().crew.unwrap()[0], Fixed::from_int(FREE_CREW));
    assert_eq!(dilithium(&sim), Fixed::from_int(300));
    // Nothing more later.
    for _ in 0..50 {
        sim.step(&[]);
    }
    assert_eq!(miners(&sim).len(), 1);
}

#[test]
fn no_key_or_unknown_name_gives_nothing() {
    for free in [None, Some("no_such_unit")] {
        let (mut sim, _) = built_station(content_free(free, true));
        sim.step(&[]);
        assert!(miners(&sim).is_empty());
    }
}

#[test]
fn missing_dock_node_uses_the_marked_fallback_ahead_of_the_station() {
    let (mut sim, _) = built_station(content_free(Some("fx_miner"), false));
    sim.step(&[]);
    let m = miners(&sim);
    assert_eq!(m.len(), 1);
    let u = sim.unit(m[0]).unwrap();
    assert_eq!(u.pos, v(10, sta_content::launch::DOCK_FALLBACK_AHEAD));
}

#[test]
fn station_crew_stops_at_zero_when_lower_than_the_free_unit_cost() {
    let base = content_free(Some("fx_miner"), true);
    let mut b = ContentDb::builder().profile(base.profile()).features(base.features().clone());
    for w in base.weapons() {
        b = b.weapon(w.clone());
    }
    for u in base.units() {
        let mut u = u.clone();
        if u.odf.eq_ignore_ascii_case("fx_station") {
            u.crew = Fixed::from_int(10);
        }
        b = b.unit(u);
    }
    let (mut sim, station) = built_station(Arc::new(b.build().unwrap()));
    sim.step(&[]);
    assert_eq!(miners(&sim).len(), 1);
    assert_eq!(sim.unit(station).unwrap().crew.unwrap()[0], Fixed::ZERO);
}

#[test]
fn a_unit_completed_by_a_yard_queue_gives_no_free_unit_even_if_its_def_names_one() {
    let base = content_free(None, true);
    let mut b = ContentDb::builder().profile(base.profile()).features(base.features().clone());
    for w in base.weapons() {
        b = b.weapon(w.clone());
    }
    for u in base.units() {
        let mut u = u.clone();
        if u.odf.eq_ignore_ascii_case("fx_frigate") {
            u.free_unit = Some("fx_miner".into());
        }
        b = b.unit(u);
    }
    let mut sim = game_with(Arc::new(b.build().unwrap()));
    let yard = spawn(&mut sim, "fx_yard", v(0, 0));
    fund(&mut sim, 1500);
    assert!(build(&mut sim, yard, "fx_frigate", None));
    let mut log = Vec::new();
    run(&mut sim, 3000, &mut log);
    assert!(!built(&log).is_empty(), "the yard finished a frigate");
    assert!(miners(&sim).is_empty());
}

#[test]
fn a_spawned_station_gives_no_free_unit() {
    let mut sim = game_with(content_free(Some("fx_miner"), true));
    spawn(&mut sim, "fx_station", v(500, 500));
    for _ in 0..5 {
        sim.step(&[]);
    }
    assert!(miners(&sim).is_empty());
}

#[test]
fn free_unit_survives_snapshot_and_two_peers_agree() {
    let (mut sim, _) = built_station(content_free(Some("fx_miner"), true));
    let db = sim.content().unwrap().clone();
    let mut other = Sim::from_snapshot(&sim.snapshot(), Some(db.clone())).unwrap();
    assert_eq!(sim.digest(), other.digest());
    let a = sim.step(&[]);
    let b = other.step(&[]);
    assert_eq!(a, b);
    assert_eq!(sim.digest(), other.digest());
    // A save taken right after completion restores the freighter.
    let mut restored = Sim::from_snapshot(&sim.snapshot(), Some(db)).unwrap();
    assert_eq!(miners(&restored).len(), 1);
    assert_eq!(sim.digest(), restored.digest());
    for _ in 0..20 {
        assert_eq!(sim.step(&[]), restored.step(&[]));
    }
}
