use std::sync::Arc;

use sta_content::{ContentDb, MapDef, Overrides, Profile, UnitDef, odf::load_with};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Order};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::UnitFlags;
use sta_sim::world::EntityKind;
use sta_sim::{MoveStats, Sim, SimEvent, WorldSource, WorldView};

use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;

const CRAFT: &[&str] = &[
    "cargo_ship",
    "colony",
    "constructionrig",
    "craft",
    "evolver",
    "freighter",
    "hover",
    "minearray",
    "mining",
    "p_mining",
    "planet",
    "pod",
    "producer",
    "repairship",
    "research",
    "scavenger",
    "sensor",
    "shipyard",
    "starbase",
    "trading_station",
    "wingman",
];
const OTHER: &[&str] = &[
    "areaeffect_obj",
    "asteroidfield",
    "background_obj",
    "blackhole_obj",
    "constplaceholder",
    "i76building",
    "ionstorm_obj",
    "latinum",
    "nebula_obj",
    "planet_obj",
    "planet_shield",
    "scrap",
    "u_ball_obj",
    "upgrade",
    "wormhole_obj",
];

fn definition(name: &str, class: &str, station: bool) -> UnitDef {
    let mut def = UnitDef::new(name, class);
    def.is_starbase = station;
    def.race = "fx".into();
    def.hull = Fixed::from_int(100);
    def
}

fn scene(defs: Vec<UnitDef>) -> (Sim, Arc<ContentDb>, Vec<EntityId>) {
    let mut builder = ContentDb::builder();
    let names: Vec<_> = defs.iter().map(|def| def.odf.clone()).collect();
    for def in defs {
        builder = builder.unit(def);
    }
    let db = Arc::new(builder.build().unwrap());
    let setup = GameSetup {
        seed: 19,
        map: Arc::new(MapDef::empty()),
        content: db.clone(),
        players: vec![PlayerSetup {
            id: PlayerId(0),
            team: TeamId(0),
            race: "fx".into(),
            control: Control::Idle,
        }],
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let ids = names
        .iter()
        .enumerate()
        .map(|(n, name)| {
            sim.spawn_def(
                db.key(name).unwrap(),
                PlayerId(0),
                Vec3::from_f32([n as f32 * 100.0, 0.0, 0.0]),
                (Fixed::ZERO, Fixed::ONE),
            )
            .unwrap()
        })
        .collect();
    (sim, db, ids)
}

fn runtime(view: WorldView, content: Option<Arc<ContentDb>>) -> MissionRuntime {
    let rt = MissionRuntime::from_source_with(
        "station-query.lua",
        "",
        19,
        MissionConfig {
            content,
            view: Some(view),
            ..MissionConfig::default()
        },
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt
}

fn query(rt: &MissionRuntime, id: EntityId) -> bool {
    rt.eval(&format!("return A2.IsStation({})", id.0)).unwrap()
}

#[test]
fn canonical_native_families_use_resolved_flag_independent_of_actor_kind() {
    let labels = CRAFT.iter().chain(OTHER.iter()).copied();
    let defs: Vec<_> = labels
        .enumerate()
        .map(|(n, class)| definition(&format!("case_{n:02}"), class, true))
        .collect();
    let (sim, db, ids) = scene(defs);
    let initial_digest = sim.digest();
    let mut rt = runtime((*sim.view()).clone(), Some(db));
    for (n, id) in ids.into_iter().enumerate() {
        assert_eq!(query(&rt, id), n < CRAFT.len(), "label index {n}");
    }
    assert_eq!(sim.digest(), initial_digest);
    assert!(rt.take_commands().is_empty());
    assert!(rt.unsupported().is_empty());
}

#[test]
fn false_flag_wins_even_for_craft_and_unclassified_labels() {
    let mut defs = vec![
        definition("craft_false", "craft", false),
        definition("station_false", "starbase", false),
        definition("unknown_false", "buildyard", false),
        definition("pod_true", "pod", true),
        definition("mine_true", "minearray", true),
        definition("planet_true", "planet", true),
        definition("upgrade_true", "upgrade", true),
        definition("latinum_true", "latinum", true),
    ];
    let mut mixed = definition("mixed_case_true", "craft", true);
    mixed.class.0 = "CrAfT".into();
    defs.push(mixed);
    let (sim, db, ids) = scene(defs);
    for (index, kind) in [
        (0, EntityKind::Craft),
        (1, EntityKind::Station),
        (3, EntityKind::Prop),
        (4, EntityKind::Prop),
        (5, EntityKind::Resource),
        (7, EntityKind::Resource),
    ] {
        assert_eq!(sim.unit(ids[index]).unwrap().kind, kind);
    }
    let mut rt = runtime((*sim.view()).clone(), Some(db));
    for (id, expected) in ids
        .into_iter()
        .zip([false, false, false, true, true, true, false, false, true])
    {
        assert_eq!(query(&rt, id), expected, "id {}", id.0);
    }
    assert!(rt.take_commands().is_empty());
    assert!(rt.unsupported().is_empty());
}

#[test]
fn inherited_flag_and_child_zero_flow_through_loaded_content() {
    let stems = vec!["inherited".into(), "cleared".into()];
    let (db, warnings) = load_with(
        Profile::Native,
        &Overrides::default(),
        &stems,
        &mut |name, version| match (name.to_ascii_lowercase().as_str(), version) {
            ("inherited.odf", 0) => Some("#include \"parent.odf\"\n".into()),
            ("cleared.odf", 0) => Some("#include \"parent.odf\"\nis_starbase = 0\n".into()),
            ("parent.odf", 0) => {
                Some("classLabel = \"craft\"\nunitName = \"Parent\"\nis_starbase = 1\n".into())
            }
            _ => None,
        },
    )
    .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let inherited = db.unit(db.key("inherited").unwrap()).unwrap().clone();
    let cleared = db.unit(db.key("cleared").unwrap()).unwrap().clone();
    assert!(inherited.is_starbase);
    assert!(!cleared.is_starbase);
    let (sim, db, ids) = scene(vec![inherited, cleared]);
    let mut rt = runtime((*sim.view()).clone(), Some(db));
    assert!(query(&rt, ids[0]));
    assert!(!query(&rt, ids[1]));
    assert!(rt.take_commands().is_empty() && rt.unsupported().is_empty());
}

#[test]
fn construction_dying_missing_and_definitionless_views_obey_liveness_order() {
    let (mut sim, db, ids) = scene(vec![definition("station", "starbase", true)]);
    let id = ids[0];
    let definitionless = sim.spawn(
        0,
        Vec3::from_f32([100.0, 0.0, 0.0]),
        (Fixed::ZERO, Fixed::ONE),
        MoveStats::STATIC,
    );
    let base = (*sim.view()).clone();
    let mut rt = runtime(base.clone(), Some(db));
    assert!(query(&rt, id));
    assert!(!query(&rt, EntityId(99999)));
    assert!(
        rt.eval::<bool>(&format!("return A2.IsStation({})", definitionless.0))
            .is_err()
    );

    let mut commands = Vec::new();
    for progress in [Some(Fixed::ZERO), Some(Fixed::ONE)] {
        let mut view = base.clone();
        view.tick += 1;
        view.units.iter_mut().find(|u| u.id == id).unwrap().progress = progress;
        // The old snapshot remains active until think receives the new view.
        assert!(query(&rt, id));
        rt.think(&view, &[], &mut commands);
        assert!(!query(&rt, id));
        rt.think(&base, &[], &mut commands);
    }
    let mut dying = base.clone();
    dying.tick += 1;
    dying.units.iter_mut().find(|u| u.id == id).unwrap().flags = UnitFlags::DYING;
    rt.think(&dying, &[], &mut commands);
    assert!(!query(&rt, id));
    rt.think(&base, &[], &mut commands);
    assert!(query(&rt, id), "None progress represents finalized craft");
    assert!(commands.is_empty() && rt.take_commands().is_empty() && rt.unsupported().is_empty());
}

#[test]
fn real_rig_weld_is_false_until_sim_finalizes_its_station() {
    let mut rig = definition("rig", "constructionrig", false);
    rig.movement = MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
    let mut product = definition("product", "starbase", true);
    product.build.ticks = 3;
    let (mut sim, db, ids) = scene(vec![rig, product]);
    let rig = ids[0];
    let product_key = db.key("product").unwrap();
    let mut rt = runtime((*sim.view()).clone(), Some(db));
    let before = sim.digest();
    let out = sim.step(&[Command {
        player: PlayerId::SCRIPT,
        seq: 1,
        kind: CommandKind::Order(Order::Build {
            builder: rig,
            what: product_key,
            at: Some(Vec3::from_f32([10.0, 0.0, 0.0])),
        }),
    }]);
    assert!(
        !out.events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. }))
    );
    let proxy = out
        .events
        .iter()
        .find_map(|event| match event {
            SimEvent::Spawned { id } if !ids.contains(id) => Some(*id),
            _ => None,
        })
        .expect("the rig must deploy a represented construction proxy");
    assert_eq!(out.view.unit(proxy).unwrap().progress, Some(Fixed::ZERO));
    assert!(!query(&rt, proxy), "old snapshot has no proxy yet");
    let mut commands = Vec::new();
    rt.think(&out.view, &out.events, &mut commands);
    assert!(
        !query(&rt, proxy),
        "fresh zero-progress proxy is not finished"
    );
    for tick in 1..=3 {
        let out = sim.step(&[]);
        rt.think(&out.view, &out.events, &mut commands);
        if tick < 3 {
            assert!(out.view.unit(proxy).unwrap().progress.is_some());
            assert!(!query(&rt, proxy), "weld tick {tick}");
        } else {
            assert!(out.events.iter().any(
                |e| matches!(e, SimEvent::Built { builder, id } if *builder == rig && *id == proxy)
            ));
            assert_eq!(out.view.unit(proxy).unwrap().progress, None);
            let after_build = sim.digest();
            assert!(query(&rt, proxy));
            assert_eq!(sim.digest(), after_build, "query must not alter sim state");
        }
    }
    assert_ne!(
        sim.digest(),
        before,
        "the authorized Build changed sim state"
    );
    assert!(commands.is_empty() && rt.take_commands().is_empty() && rt.unsupported().is_empty());
}

#[test]
fn malformed_arguments_fail_before_state_lookup_and_leave_no_commands() {
    let (sim, db, ids) = scene(vec![definition("station", "starbase", true)]);
    let mut rt = runtime((*sim.view()).clone(), Some(db));
    for call in [
        "A2.IsStation()".to_owned(),
        format!("A2.IsStation({}, 1)", ids[0].0),
        "A2.IsStation(-1)".into(),
        "A2.IsStation(4294967296)".into(),
        "A2.IsStation(1.5)".into(),
        "A2.IsStation(0/0)".into(),
        "A2.IsStation(math.huge)".into(),
        "A2.IsStation(-math.huge)".into(),
        "A2.IsStation('1')".into(),
        "A2.IsStation(nil)".into(),
        "A2.IsStation(true)".into(),
    ] {
        assert!(rt.eval::<()>(&call).is_err(), "{call}");
        assert!(rt.take_commands().is_empty(), "{call}");
    }
    assert!(rt.unsupported().is_empty());
}

#[test]
fn unknown_true_class_reports_stable_count_even_when_error_is_caught() {
    let (sim, db, ids) = scene(vec![definition("unknown", "buildyard", true)]);
    let mut rt = runtime((*sim.view()).clone(), Some(db));
    for expected_count in [1, 2] {
        let (ok, error): (bool, String) = rt
            .eval(&format!(
                "local ok, error = pcall(A2.IsStation, {}); return ok, tostring(error)",
                ids[0].0
            ))
            .unwrap();
        assert!(!ok);
        assert!(
            error.contains("A2.IsStation") && error.contains("buildyard"),
            "{error}"
        );
        assert!(!error.contains(&format!("object {}", ids[0].0)), "{error}");
        assert_eq!(
            rt.unsupported(),
            [(
                "A2.IsStation: unclassified native class buildyard".to_owned(),
                expected_count
            )]
        );
    }
    assert!(rt.take_commands().is_empty());
}
