//! Public Sim regressions for map-confinement state and birth timing.

use std::sync::Arc;

use sta_content::{
    ConfinementInitial, ConfinementOverrides, ContentDb, MapDef, MapObject, MoveStats, Profile,
    UnitDef,
};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

fn n(value: i32) -> Fixed {
    Fixed::from_int(value)
}

fn at(x: i32, y: i32, z: i32) -> Vec3 {
    Vec3 {
        x: n(x),
        y: n(y),
        z: n(z),
    }
}

fn content(
    profile: Profile,
    class: &str,
    overrides: ConfinementOverrides,
    mobile: bool,
) -> Arc<ContentDb> {
    let mut def = UnitDef::new("subject", class);
    def.confinement = overrides;
    def.hull = n(100);
    if mobile {
        def.movement = MoveStats::from_f32(120.0, 2.0, 2.0, 2.0, 1.0);
    }
    Arc::new(
        ContentDb::builder()
            .profile(profile)
            .unit(def)
            .build()
            .unwrap(),
    )
}

fn setup(content: Arc<ContentDb>, bounds: (Vec3, Vec3), placed: &[Vec3]) -> GameSetup {
    let mut map = MapDef::empty();
    map.bounds = bounds;
    map.objects = placed
        .iter()
        .copied()
        .map(|pos| MapObject {
            odf: "subject".into(),
            owner: Some(0),
            pos,
            heading: (Fixed::ZERO, Fixed::ONE),
            label: None,
        })
        .collect();
    GameSetup {
        seed: 29,
        map: Arc::new(map),
        content,
        players: vec![PlayerSetup {
            id: PlayerId(0),
            team: TeamId(0),
            race: "synthetic".into(),
            control: Control::Idle,
        }],
        options: GameOptions::default(),
    }
}

fn bounds() -> (Vec3, Vec3) {
    (at(-500, -10, -500), at(500, 10, 500))
}

fn spawn(sim: &mut Sim, pos: Vec3) -> EntityId {
    let key = sim.content().unwrap().key("subject").unwrap();
    sim.spawn_def(key, PlayerId(0), pos, (Fixed::ZERO, Fixed::ONE))
        .unwrap()
}

fn edit(player: PlayerId, seq: u32, id: EntityId, enabled: bool) -> Command {
    Command {
        player,
        seq,
        kind: CommandKind::Edit(Edit::ForceOntoMap { id, enabled }),
    }
}

fn rejected(events: &[SimEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, SimEvent::Rejected { .. }))
        .count()
}

#[test]
fn a2_and_fo_runtime_birth_waits_for_map_completion() {
    for profile in [Profile::Sta2Classic, Profile::Fo4] {
        let db = content(profile, "craft", ConfinementOverrides::default(), false);
        let mut runtime = Sim::from_setup(&setup(db.clone(), bounds(), &[]));
        let id = spawn(&mut runtime, at(450, 0, 0));
        assert_eq!(runtime.force_onto_map(id), Some(true));
        assert_eq!(runtime.unit(id).unwrap().pos.x, n(450));

        let placed = Sim::from_setup(&setup(db, bounds(), &[at(450, 0, 0)]));
        assert_eq!(placed.force_onto_map(EntityId(1)), Some(true));
        assert_eq!(
            placed.unit(EntityId(1)).unwrap().pos.x,
            n(400) - Fixed::from_bits(429496736)
        );
    }
}

#[test]
fn raw_xz_edges_are_strict_and_initialization_ignores_y() {
    let db = content(
        Profile::Fo4,
        "craft",
        ConfinementOverrides::default(),
        false,
    );
    let mut sim = Sim::from_setup(&setup(db, bounds(), &[]));
    for (pos, expected) in [
        (at(-500, 0, 0), false),
        (at(500, 0, 0), false),
        (at(0, 0, -500), false),
        (at(0, 0, 500), false),
        (at(0, 100, 0), true),
        (at(499, -100, 499), true),
    ] {
        let id = spawn(&mut sim, pos);
        assert_eq!(sim.force_onto_map(id), Some(expected), "{pos:?}");
        assert_eq!(sim.unit(id).unwrap().pos, pos, "runtime birth {pos:?}");
    }
}

#[test]
fn a1_padded_edges_and_rearm_differ_from_a2_sticky_false() {
    let a1 = content(
        Profile::Sta1Classic,
        "craft",
        ConfinementOverrides::default(),
        true,
    );
    let mut sim = Sim::from_setup(&setup(a1, bounds(), &[]));
    let lower_edge = spawn(&mut sim, at(-400, 0, 0));
    let upper_edge = spawn(&mut sim, at(400, 0, 0));
    assert_eq!(sim.force_onto_map(lower_edge), Some(true));
    assert_eq!(sim.force_onto_map(upper_edge), Some(true));
    let a1 = content(
        Profile::Sta1Classic,
        "craft",
        ConfinementOverrides::default(),
        true,
    );
    let mut sim = Sim::from_setup(&setup(a1, bounds(), &[]));
    let key = sim.content().unwrap().key("subject").unwrap();
    let entrant = sim
        .spawn_def(key, PlayerId(0), at(-450, 0, 0), (Fixed::ONE, Fixed::ZERO))
        .unwrap();
    assert_eq!(sim.force_onto_map(entrant), Some(false));
    let move_in = Command {
        player: PlayerId(0),
        seq: 1,
        kind: CommandKind::Order(Order::Move {
            units: vec![entrant],
            to: at(0, 0, 0),
            queue: false,
        }),
    };
    assert_eq!(rejected(&sim.step(&[move_in]).events), 0);
    for _ in 0..600 {
        if sim.force_onto_map(entrant) == Some(true) {
            break;
        }
        sim.step(&[]);
    }
    assert_eq!(sim.force_onto_map(entrant), Some(true));

    let a2 = content(
        Profile::Sta2Classic,
        "craft",
        ConfinementOverrides::default(),
        true,
    );
    let mut sim = Sim::from_setup(&setup(a2, bounds(), &[]));
    let key = sim.content().unwrap().key("subject").unwrap();
    let entrant = sim
        .spawn_def(key, PlayerId(0), at(-500, 0, 0), (Fixed::ONE, Fixed::ZERO))
        .unwrap();
    assert_eq!(sim.force_onto_map(entrant), Some(false));
    let move_in = Command {
        player: PlayerId(0),
        seq: 1,
        kind: CommandKind::Order(Order::Move {
            units: vec![entrant],
            to: at(0, 0, 0),
            queue: false,
        }),
    };
    assert_eq!(rejected(&sim.step(&[move_in]).events), 0);
    for _ in 0..600 {
        sim.step(&[]);
    }
    assert!(sim.unit(entrant).unwrap().pos.x > n(-400));
    assert_eq!(sim.force_onto_map(entrant), Some(false));
}

#[test]
fn terrain_bypasses_projection_and_immobile_station_ticks() {
    let terrain = content(
        Profile::Sta1Classic,
        "nebula_obj",
        ConfinementOverrides::default(),
        false,
    );
    let mut sim = Sim::from_setup(&setup(terrain, bounds(), &[at(600, 30, 0)]));
    let id = EntityId(1);
    assert_eq!(sim.force_onto_map(id), Some(true));
    assert_eq!(sim.unit(id).unwrap().pos, at(600, 30, 0));
    sim.step(&[]);
    assert_eq!(sim.unit(id).unwrap().pos, at(600, 30, 0));

    let station = content(
        Profile::Fo4,
        "starbase",
        ConfinementOverrides::default(),
        false,
    );
    let mut sim = Sim::from_setup(&setup(station, bounds(), &[]));
    let id = spawn(&mut sim, at(450, 0, 0));
    assert_eq!(sim.unit(id).unwrap().pos.x, n(450));
    sim.step(&[]);
    assert_eq!(
        sim.unit(id).unwrap().pos.x,
        n(400) - Fixed::from_bits(429496736)
    );
}

#[test]
fn setter_checks_privilege_availability_and_preserves_position() {
    let db = content(
        Profile::Fo4,
        "constplaceholder",
        ConfinementOverrides::default(),
        false,
    );
    let mut sim = Sim::from_setup(&setup(db, bounds(), &[]));
    let id = spawn(&mut sim, at(0, 0, 0));
    assert_eq!(sim.force_onto_map(id), Some(true));
    let before = sim.unit(id).unwrap().pos;
    assert_eq!(
        rejected(&sim.step(&[edit(PlayerId(0), 1, id, false)]).events),
        1
    );
    assert_eq!(sim.force_onto_map(id), Some(true));
    assert_eq!(
        rejected(&sim.step(&[edit(PlayerId::SCRIPT, 1, id, false)]).events),
        0
    );
    assert_eq!(sim.force_onto_map(id), Some(false));
    assert_eq!(sim.unit(id).unwrap().pos, before);
    assert_eq!(
        rejected(&sim.step(&[edit(PlayerId::BENCH, 1, id, false)]).events),
        0
    );
    assert_eq!(
        rejected(&sim.step(&[edit(PlayerId::BENCH, 2, id, true)]).events),
        0
    );
    assert_eq!(sim.force_onto_map(id), Some(true));
    assert_eq!(
        rejected(
            &sim.step(&[edit(PlayerId::SCRIPT, 2, EntityId::NONE, false)])
                .events
        ),
        1
    );
    assert!(sim.despawn(id, None));
    assert_eq!(sim.force_onto_map(id), None);
    assert_eq!(
        rejected(&sim.step(&[edit(PlayerId::SCRIPT, 3, id, false)]).events),
        1
    );
}

#[test]
fn unusable_and_unsupported_policies_keep_actors_without_rows() {
    let db = content(
        Profile::Fo4,
        "craft",
        ConfinementOverrides::default(),
        false,
    );
    for bad in [
        (at(-500, 0, -500), at(-400, 10, -400)),
        (at(500, -10, -500), at(-500, 10, 500)),
    ] {
        let (mut sim, warnings) =
            Sim::from_setup_with_warnings(&setup(db.clone(), bad, &[at(0, 0, 0)]));
        assert!(
            warnings
                .iter()
                .any(|w| w.file == "subject.odf"
                    && w.message.contains("map confinement unavailable")),
            "{warnings:?}"
        );
        assert!(sim.unit(EntityId(1)).is_some());
        assert_eq!(sim.force_onto_map(EntityId(1)), None);
        assert_eq!(
            rejected(
                &sim.step(&[edit(PlayerId::SCRIPT, 1, EntityId(1), false)])
                    .events
            ),
            1
        );
    }

    let unsupported = content(
        Profile::Fo4,
        "unknown_factory",
        ConfinementOverrides {
            capability: Some(true),
            ..Default::default()
        },
        false,
    );
    let (mut sim, warnings) =
        Sim::from_setup_with_warnings(&setup(unsupported, bounds(), &[at(0, 0, 0)]));
    assert!(
        warnings
            .iter()
            .any(|w| w.message.contains("no verified spatial policy"))
    );
    assert_eq!(sim.force_onto_map(EntityId(1)), None);
    assert_eq!(
        rejected(
            &sim.step(&[edit(PlayerId::SCRIPT, 1, EntityId(1), false)])
                .events
        ),
        1
    );
}

#[test]
fn enabled_empty_bounds_hash_and_exact_false_snapshot() {
    let enabled = content(
        Profile::Fo4,
        "craft",
        ConfinementOverrides::default(),
        false,
    );
    let a = Sim::from_setup(&setup(enabled.clone(), bounds(), &[]));
    let b = Sim::from_setup(&setup(
        enabled.clone(),
        (at(-600, -10, -500), at(500, 10, 500)),
        &[],
    ));
    assert_ne!(a.digest(), b.digest());
    let native = content(
        Profile::Native,
        "craft",
        ConfinementOverrides::default(),
        false,
    );
    let a = Sim::from_setup(&setup(native.clone(), bounds(), &[]));
    let b = Sim::from_setup(&setup(native, (at(-600, -10, -500), at(500, 10, 500)), &[]));
    assert_eq!(a.digest(), b.digest());

    let mut sim = Sim::from_setup(&setup(enabled.clone(), bounds(), &[]));
    let id = spawn(&mut sim, at(0, 0, 0));
    assert_eq!(
        rejected(&sim.step(&[edit(PlayerId::SCRIPT, 1, id, false)]).events),
        0
    );
    assert_eq!(sim.force_onto_map(id), Some(false));
    let bytes = sim.snapshot();
    let loaded = Sim::from_snapshot(&bytes, Some(enabled)).unwrap();
    assert_eq!(loaded.force_onto_map(id), Some(false));
    assert_eq!(loaded.snapshot(), bytes);
    assert_eq!(loaded.digest(), sim.digest());
}

#[test]
fn native_solid_actor_does_not_collide_with_its_own_boundary_projection() {
    let mut def = UnitDef::new("subject", "planet");
    def.hull = n(100);
    def.radius = n(100);
    def.confinement = ConfinementOverrides {
        capability: Some(true),
        initial: Some(ConfinementInitial::Disabled),
        ..Default::default()
    };
    let db = Arc::new(ContentDb::builder().unit(def).build().unwrap());
    let mut sim = Sim::from_setup(&setup(db, bounds(), &[]));
    let id = spawn(&mut sim, at(450, 0, 0));
    assert_eq!(sim.force_onto_map(id), Some(false));
    assert_eq!(sim.unit(id).unwrap().pos, at(450, 0, 0));
    assert_eq!(
        rejected(&sim.step(&[edit(PlayerId::SCRIPT, 1, id, true)]).events),
        0
    );
    assert_eq!(
        sim.unit(id).unwrap().pos.x,
        n(400) - Fixed::from_bits(429496736)
    );
    assert_eq!(sim.force_onto_map(id), Some(true));
}
