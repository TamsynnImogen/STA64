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
