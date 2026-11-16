use super::*;
use sta_sim::command::{Edit, LocalRouter};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Fixed, Sim};

#[test]
fn active_production_is_cached_at_initial_load_advances_and_clears_on_stop() {
    let db = Arc::new(sta_content::fake::db());
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 42,
        map: Arc::new(sta_content::MapDef::empty()),
        content: db.clone(),
        players: vec![PlayerSetup {
            id: PlayerId(0),
            team: sta_sim::TeamId(0),
            race: "fx".into(),
            control: Control::Human,
        }],
        options: GameOptions::default(),
    });
    let step = |sim: &mut Sim, player, kind| {
        sim.step(&[Command {
            player,
            seq: 0,
            kind,
        }])
    };
    let out = step(
        &mut sim,
        PlayerId::BENCH,
        CommandKind::Edit(Edit::Spawn {
            odf: db.key("fx_yard").unwrap(),
            player: PlayerId(0),
            pos: SimVec3::ZERO,
            heading: (Fixed::ZERO, Fixed::ONE),
        }),
    );
    let yard = out.view.units[0].id;
    step(
        &mut sim,
        PlayerId::BENCH,
        CommandKind::Edit(Edit::Cargo(PlayerId(0), 0, Fixed::from_int(2000))),
    );
    step(
        &mut sim,
        PlayerId(0),
        CommandKind::Order(Order::Build {
            builder: yard,
            what: db.key("fx_cruiser").unwrap(),
            at: None,
        }),
    );
    let initial = sim.production_view();
    assert_eq!(initial.len(), 1);
    let mut link = SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0));
    assert_eq!(link.production(), initial);
    let tick = link.curr().tick;
    link.tick().unwrap();
    assert_eq!(link.curr().tick, tick + 1);
    assert!(link.production()[0].jobs[0].progress > initial[0].jobs[0].progress);
    link.submit(CommandKind::Order(Order::Stop { units: vec![yard] }));
    link.tick().unwrap();
    assert!(link.production().is_empty());
    assert!(link.curr().unit(yard).is_some());
}

#[test]
fn cargo_cache_initializes_refreshes_on_tick_and_clears_on_legacy_external_feed() {
    let db = Arc::new(sta_content::fake::db());
    let mut sim = Sim::from_setup(&GameSetup {
        seed: 42,
        map: Arc::new(sta_content::MapDef::empty()),
        content: db.clone(),
        players: vec![PlayerSetup {
            id: PlayerId(0),
            team: sta_sim::TeamId(0),
            race: "fx".into(),
            control: Control::Human,
        }],
        options: GameOptions::default(),
    });
    let spawn = |sim: &mut Sim, odf, player| {
        let out = sim.step(&[Command {
            player: PlayerId::BENCH,
            seq: 0,
            kind: CommandKind::Edit(Edit::Spawn {
                odf: db.key(odf).unwrap(),
                player,
                pos: SimVec3::ZERO,
                heading: (Fixed::ZERO, Fixed::ONE),
            }),
        }]);
        out.view.units.last().unwrap().id
    };
    let moon = spawn(&mut sim, "fx_moon", sta_sim::world::NEUTRAL);
    let miner = spawn(&mut sim, "fx_miner", PlayerId(0));
    let initial = sim.cargo_view();
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0].entity, miner);
    assert_eq!(initial[0].amount, Fixed::ZERO);
    assert_eq!(initial[0].capacity, Fixed::from_int(150));
    assert_eq!(initial[0].resource, None);

    let mut link = SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0));
    assert_eq!(link.cargo(), initial);
    link.submit(CommandKind::Order(Order::Mine { units: vec![miner], node: moon }));
    link.tick().unwrap();
    let loaded = link.cargo().to_vec();
    assert_eq!(loaded.len(), 1);
    assert!(loaded[0].amount > Fixed::ZERO);
    assert_eq!(loaded[0].resource, Some(0));
    // Legacy external feeds carry no economy rows; they must not retain a
    // previous tick's real cargo as though it matched the replacement view.
    let shown = link.real_curr().clone();
    link.feed(shown);
    assert!(link.cargo().is_empty());
    link.tick().unwrap();
    assert!(link.cargo()[0].amount >= loaded[0].amount);
    let actual = link.cargo().to_vec();
    let shown = link.real_curr().clone();
    link.feed_with_production(shown, Vec::new());
    assert!(link.cargo().is_empty());
    let shown = link.real_curr().clone();
    link.feed_with_production_and_cargo(shown, Vec::new(), actual.clone());
    assert_eq!(link.cargo(), actual);
}

#[test]
fn scripted_source_does_not_invent_cargo_for_a_miner_in_its_view() {
    let db = sta_content::fake::db();
    let source = sta_sim::fake::ScriptedSource::from_json5(
        r#"{units:[{id:1,def:"fx_miner",owner:0,hull:250,rails:[{at:0,pos:{x:0,y:0,z:0}}]}]}"#,
        &db,
    ).unwrap();
    let link = SimLink::new(Box::new(source), Box::new(LocalRouter::new()), PlayerId(0));
    assert_eq!(link.curr().units.len(), 1);
    assert!(link.cargo().is_empty());
}
