//! Starting strengths are shared spawn rules, never a HUD-only substitution.

use std::sync::Arc;

use sta_content::{ContentDb, Features, MapDef, MapObject, Overrides, Profile, UnitDef};
use sta_core::{Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
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

#[test]
fn entity_initialization_preserves_partial_zero_and_directional_strengths() {
    let mut def = UnitDef::new("partial", "craft");
    def.hull = n(200);
    def.shields.max = n(80);
    for directional in [false, true] {
        let mut features = Features::default();
        features.directional_shields = directional;
        let full = Entity::from_def(&def, &features, PlayerId(0), Vec3::ZERO, HEADING);
        assert_eq!(full.hull, [n(200); 2]);
        assert_eq!(full.shields.view(), [[n(80); 2]; 4]);
        for (hull, shield) in [(90, 20), (0, 0)] {
            let mut partial = def.clone();
            partial.initial_hull = Some(n(hull));
            partial.initial_shields = Some(n(shield));
            let unit = Entity::from_def(&partial, &features, PlayerId(0), Vec3::ZERO, HEADING);
            assert_eq!(unit.hull, [n(hull), n(200)]);
            assert_eq!(unit.shields.view(), [[n(shield), n(80)]; 4]);
            assert_eq!(
                unit.shields.now,
                if directional {
                    [n(shield); 4]
                } else {
                    [n(shield), n(0), n(0), n(0)]
                }
            );
        }
    }
}

fn classic(initial: i32) -> Arc<ContentDb> {
    let text = format!("classLabel = \"craft\"\nmaxHealth = 150\ncurHealth = {initial}\n");
    let (db, warnings) = sta_content::odf::load_with(
        Profile::Sta2Classic,
        &Overrides::default(),
        &["partial".into()],
        &mut |_, _| Some(text.clone()),
    )
    .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    Arc::new(db)
}

#[test]
fn loaded_health_fallback_reaches_map_script_view_and_save_without_refilling() {
    let db = classic(75);
    let game = setup(db.clone(), vec![object("partial")]);
    let mut sim = Sim::from_setup(&game);
    let first = sim.view().units[0].id;
    assert_eq!(sim.unit(first).unwrap().hull, [n(75), n(150)]);
    assert_eq!(
        sim.view().unit(first).unwrap().shields,
        [[n(75), n(150)]; 4]
    );

    let second = sim
        .spawn_def(db.key("partial").unwrap(), PlayerId(0), Vec3::ZERO, HEADING)
        .unwrap();
    assert_eq!(sim.unit(second).unwrap().hull, [n(75), n(150)]);
    assert_eq!(
        sim.unit(second).unwrap().shields.view(),
        [[n(75), n(150)]; 4]
    );
    sim.step(&[Command {
        player: PlayerId::SCRIPT,
        seq: 1,
        kind: CommandKind::Edit(Edit::Hull(first, n(40))),
    }]);
    let bytes = sim.snapshot();
    let restored = Sim::from_snapshot(&bytes, Some(db)).unwrap();
    assert_eq!(restored.unit(first).unwrap().hull, [n(40), n(150)]);
    assert_eq!(
        restored.unit(first).unwrap().shields.view(),
        [[n(75), n(150)]; 4]
    );
    assert_eq!(restored.snapshot(), bytes);
    assert_eq!(restored.digest(), sim.digest());
    assert!(
        Sim::from_snapshot(&bytes, Some(classic(74))).is_err(),
        "changed starting rules must fail content admission"
    );
}

#[test]
fn completed_yard_production_uses_authored_starting_pools() {
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
            def.initial_hull = Some(n(90));
            def.initial_shields = Some(n(20));
            def.shields.recharge = Fixed::ZERO;
            def.build.ticks = 2;
            def.build.cost.clear();
            def.build.officers = Fixed::ZERO;
        }
        builder = builder.unit(def);
    }
    let db = Arc::new(builder.build().unwrap());
    let mut sim = Sim::from_setup(&setup(db.clone(), vec![object("fx_yard")]));
    let yard = sim.view().units[0].id;
    let order = Command {
        player: PlayerId(0),
        seq: 1,
        kind: CommandKind::Order(Order::Build {
            builder: yard,
            what: db.key("fx_cruiser").unwrap(),
            at: None,
        }),
    };
    let mut events = sim.step(&[order]).events;
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
    let unit = sim.unit(id).unwrap();
    assert_eq!(unit.hull, [n(90), n(800)]);
    assert_eq!(unit.shields.view(), [[n(20), n(400)]; 4]);
    assert_eq!(sim.view().unit(id).unwrap().hull, unit.hull);
}

#[test]
fn installed_classic_constructor_has_live_health_derived_shields() {
    let Ok(root) = std::env::var("STA64_POOL_TEST_INSTALL") else {
        return;
    };
    let active_mod = std::env::var("STA64_POOL_TEST_MOD").unwrap_or_else(|_| "STA2 Classic".into());
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), Some(&active_mod)).unwrap();
    let (index, _) = sta_content::OdfIndex::build(&vfs);
    let (db, warnings) = ContentDb::load(&vfs, &index).unwrap();
    assert!(matches!(db.profile(), Profile::Sta2Classic | Profile::Fo4));
    let def = db
        .unit(db.key("fconst").expect("installed Classic constructor"))
        .unwrap();
    for key in ["maxShield", "maxShields", "curShield", "curShields"] {
        assert!(
            def.raw.raw(key).is_none(),
            "fixture now explicitly supplies {key}; update the installed-data check"
        );
    }
    assert!(def.hull > Fixed::ZERO);
    assert_eq!(def.shields.max, def.hull);
    assert_eq!(
        def.initial_shields.unwrap_or(def.shields.max),
        def.initial_hull.unwrap_or(def.hull)
    );
    let db = Arc::new(db);
    let sim = Sim::from_setup(&setup(db, vec![object("fconst")]));
    let view = sim.view();
    let unit = &view.units[0];
    assert_eq!(unit.shields, [unit.hull; 4]);
    eprintln!(
        "{active_mod} fconst: live hull {:?}, live shields {:?}; {} existing content warnings",
        unit.hull,
        unit.shields,
        warnings.len()
    );
}
