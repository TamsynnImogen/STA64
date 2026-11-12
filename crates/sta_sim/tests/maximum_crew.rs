use std::sync::Arc;

use sta_content::{ContentDb, Features, MapDef, MapObject, Overrides, Profile, UnitDef};
use sta_core::{Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Order};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::world::Entity;
use sta_sim::{Sim, SimEvent, WorldSource};

const HEADING: (Fixed, Fixed) = (Fixed::ZERO, Fixed::ONE);

fn n(value: i32) -> Fixed {
    Fixed::from_int(value)
}

fn object(name: &str) -> MapObject {
    MapObject {
        odf: name.into(),
        owner: Some(0),
        pos: Vec3::ZERO,
        heading: HEADING,
        label: None,
    }
}

fn setup(content: Arc<ContentDb>, objects: Vec<MapObject>) -> GameSetup {
    let mut map = MapDef::empty();
    map.objects = objects;
    GameSetup {
        seed: 67,
        map: Arc::new(map),
        content,
        players: (0..2)
            .map(|id| PlayerSetup {
                id: PlayerId(id),
                team: TeamId(id),
                race: "fx".into(),
                control: Control::Idle,
            })
            .collect(),
        options: GameOptions::default(),
    }
}

fn classic(maximum: i32) -> Arc<ContentDb> {
    let text = format!("classLabel = \"craft\"\ncrewCost = 150\nmaximumCrew = {maximum}\n");
    let (db, warnings) = sta_content::odf::load_with(
        Profile::Sta2Classic,
        &Overrides::default(),
        &["ship".into()],
        &mut |_, _| Some(text.clone()),
    )
    .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    Arc::new(db)
}

#[test]
fn constructor_keeps_independent_start_and_capacity_with_legacy_fallback() {
    let mut def = UnitDef::new("ship", "craft");
    def.crew = n(150);
    let features = Features::default();
    assert_eq!(
        Entity::from_def(&def, &features, PlayerId(0), Vec3::ZERO, HEADING).crew,
        Some([n(150), n(150)])
    );
    for maximum in [0, 1500] {
        def.maximum_crew = Some(n(maximum));
        assert_eq!(
            Entity::from_def(&def, &features, PlayerId(0), Vec3::ZERO, HEADING).crew,
            Some([n(150), n(maximum)])
        );
    }
    let fake = sta_content::fake::db();
    let cruiser = fake.unit(fake.key("fx_cruiser").unwrap()).unwrap();
    assert_eq!(cruiser.maximum_crew, None);
    assert_eq!(
        Entity::from_def(cruiser, fake.features(), PlayerId(0), Vec3::ZERO, HEADING).crew,
        Some([cruiser.crew, cruiser.crew])
    );
}

#[test]
fn loaded_map_and_script_spawns_show_exact_crew_and_snapshot_never_refills() {
    for maximum in [0, 1500] {
        let db = classic(maximum);
        let mut sim = Sim::from_setup(&setup(db.clone(), vec![object("ship")]));
        let mapped = sim.view().units[0].id;
        let expected = Some([n(150), n(maximum)]);
        assert_eq!(sim.unit(mapped).unwrap().crew, expected);
        assert_eq!(sim.view().unit(mapped).unwrap().crew, expected);
        let scripted = sim
            .spawn_def(db.key("ship").unwrap(), PlayerId(0), Vec3::ZERO, HEADING)
            .unwrap();
        assert_eq!(sim.unit(scripted).unwrap().crew, expected);
        assert_eq!(sim.view().unit(scripted).unwrap().crew, expected);
        for _ in 0..3 {
            sim.step(&[]);
        }
        assert_eq!(sim.unit(mapped).unwrap().crew, expected);
        assert_eq!(sim.unit(scripted).unwrap().crew, expected);
        let bytes = sim.snapshot();
        let mut restored = Sim::from_snapshot(&bytes, Some(db.clone())).unwrap();
        assert_eq!(restored.snapshot(), bytes);
        assert_eq!(restored.digest(), sim.digest());
        assert_eq!(restored.view().unit(mapped).unwrap().crew, expected);
        assert_eq!(restored.view().unit(scripted).unwrap().crew, expected);
        restored.step(&[]);
        assert_eq!(restored.unit(mapped).unwrap().crew, expected);
        assert_eq!(restored.unit(scripted).unwrap().crew, expected);
        let other = if maximum == 0 { 1500 } else { 0 };
        assert!(Sim::from_snapshot(&bytes, Some(classic(other))).is_err());
    }
}

#[test]
fn completed_yard_production_uses_product_start_and_capacity() {
    let fake = sta_content::fake::db();
    let mut builder = ContentDb::builder()
        .profile(fake.profile())
        .features(fake.features().clone());
    for weapon in fake.weapons() {
        builder = builder.weapon(weapon.clone());
    }
    for source in fake.units() {
        let mut def = source.clone();
        if def.odf == "fx_cruiser" {
            def.crew = n(150);
            def.maximum_crew = Some(n(1500));
            def.build.ticks = 2;
            def.build.cost.clear();
            def.build.officers = Fixed::ZERO;
        }
        builder = builder.unit(def);
    }
    let db = Arc::new(builder.build().unwrap());
    let mut sim = Sim::from_setup(&setup(db.clone(), vec![object("fx_yard")]));
    let yard = sim.view().units[0].id;
    let mut events = sim
        .step(&[Command {
            player: PlayerId(0),
            seq: 1,
            kind: CommandKind::Order(Order::Build {
                builder: yard,
                what: db.key("fx_cruiser").unwrap(),
                at: None,
            }),
        }])
        .events;
    for _ in 0..5 {
        events.extend(sim.step(&[]).events);
    }
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, SimEvent::Rejected { .. })),
        "{events:?}"
    );
    let id = events
        .iter()
        .find_map(|event| match event {
            SimEvent::Built { id, .. } => Some(*id),
            _ => None,
        })
        .expect("yard completed the ship");
    assert_eq!(sim.unit(id).unwrap().crew, Some([n(150), n(1500)]));
    assert_eq!(sim.view().unit(id).unwrap().crew, Some([n(150), n(1500)]));
}
