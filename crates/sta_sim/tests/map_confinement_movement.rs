//! Public simulation checks for confined command targets and obstacle steering.

use std::sync::Arc;

use sta_content::{
    ContentDb, MapDef, MapObject, MoveStats, Profile, Region, RegionKind, RegionShape, UnitDef,
};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::movement::FORM_LINE;
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

fn n(x: i32) -> Fixed {
    Fixed::from_int(x)
}
fn p(x: i32, z: i32) -> Vec3 {
    Vec3 {
        x: n(x),
        y: Fixed::ZERO,
        z: n(z),
    }
}
fn upper() -> Fixed {
    n(400) - Fixed::from_bits(429496736)
}

fn content(with_station: bool) -> Arc<ContentDb> {
    let mut craft = UnitDef::new("craft", "craft");
    craft.hull = n(100);
    craft.radius = n(100); // body radius 75
    craft.movement = MoveStats::from_f32(120.0, 2.0, 2.0, 2.0, 1.0);
    let mut builder = ContentDb::builder().profile(Profile::Fo4).unit(craft);
    if with_station {
        let mut station = UnitDef::new("station", "starbase");
        station.hull = n(100);
        station.radius = n(40); // solid radius 30
        builder = builder.unit(station);
    }
    Arc::new(builder.build().unwrap())
}

fn setup(db: Arc<ContentDb>, regions: Vec<Region>, objects: Vec<(&str, u8, Vec3)>) -> GameSetup {
    let mut map = MapDef::empty();
    map.bounds = (p(-500, -500), p(500, 500));
    map.regions = regions;
    map.objects = objects
        .into_iter()
        .map(|(odf, owner, pos)| MapObject {
            odf: odf.into(),
            owner: Some(owner),
            pos,
            heading: (Fixed::ONE, Fixed::ZERO),
            label: None,
        })
        .collect();
    GameSetup {
        seed: 97,
        map: Arc::new(map),
        content: db,
        players: [0, 1]
            .into_iter()
            .map(|id| PlayerSetup {
                id: PlayerId(id),
                team: TeamId(id),
                race: "synthetic".into(),
                control: Control::Idle,
            })
            .collect(),
        options: GameOptions::default(),
    }
}

fn spawn(sim: &mut Sim, owner: u8, at: Vec3) -> EntityId {
    let key = sim.content().unwrap().key("craft").unwrap();
    sim.spawn_def(key, PlayerId(owner), at, (Fixed::ONE, Fixed::ZERO))
        .unwrap()
}

fn mv(player: u8, seq: u32, ids: &[EntityId], to: Vec3, queue: bool) -> Command {
    Command {
        player: PlayerId(player),
        seq,
        kind: CommandKind::Order(Order::Move {
            units: ids.to_vec(),
            to,
            queue,
        }),
    }
}

fn force(seq: u32, id: EntityId, enabled: bool) -> Command {
    Command {
        player: PlayerId::SCRIPT,
        seq,
        kind: CommandKind::Edit(Edit::ForceOntoMap { id, enabled }),
    }
}

fn accepted(events: &[SimEvent]) {
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SimEvent::Rejected { .. })),
        "{events:?}"
    );
}

#[test]
fn final_flag_in_batch_governs_target_projection() {
    let db = content(false);
    let mut sim = Sim::from_setup(&setup(db, vec![], vec![]));
    let id = spawn(&mut sim, 0, p(0, 0));
    accepted(
        &sim.step(&[mv(0, 1, &[id], p(467, 0), false), force(1, id, false)])
            .events,
    );
    assert_eq!(sim.force_onto_map(id), Some(false));
    assert_eq!(sim.unit(id).unwrap().target, Some(p(467, 0)));

    accepted(
        &sim.step(&[mv(0, 2, &[id], p(467, 0), false), force(2, id, true)])
            .events,
    );
    assert_eq!(sim.force_onto_map(id), Some(true));
    assert_eq!(sim.unit(id).unwrap().target.unwrap().x, upper());

    accepted(
        &sim.step(&[
            mv(0, 3, &[id], p(467, 0), false),
            force(3, id, true),
            force(4, id, false),
        ])
        .events,
    );
    assert_eq!(sim.force_onto_map(id), Some(false));
    assert_eq!(sim.unit(id).unwrap().target, Some(p(467, 0)));
}

#[test]
fn active_formation_and_queued_waypoints_are_confined() {
    let mut sim = Sim::from_setup(&setup(content(false), vec![], vec![]));
    let a = spawn(&mut sim, 0, p(-100, -100));
    let b = spawn(&mut sim, 0, p(100, -100));
    let formation = Command {
        player: PlayerId(0),
        seq: 1,
        kind: CommandKind::Order(Order::Formation {
            units: vec![a, b],
            kind: FORM_LINE,
        }),
    };
    accepted(&sim.step(&[formation]).events);
    accepted(&sim.step(&[mv(0, 2, &[a, b], p(467, 467), false)]).events);
    for id in [a, b] {
        let target = sim.unit(id).unwrap().target.unwrap();
        assert!(
            target.x <= n(400) && target.z <= n(400),
            "{id:?}: {target:?}"
        );
        assert!(target.x >= n(-400) && target.z >= n(-400));
    }

    // A queued second leg must be projected before it becomes the current leg.
    let mut sim = Sim::from_setup(&setup(content(false), vec![], vec![]));
    let id = spawn(&mut sim, 0, p(0, 0));
    accepted(&sim.step(&[mv(0, 1, &[id], p(250, 0), false)]).events);
    accepted(&sim.step(&[mv(0, 2, &[id], p(467, 0), true)]).events);
    let mut second = false;
    for _ in 0..900 {
        sim.step(&[]);
        if sim.unit(id).unwrap().target.is_some_and(|t| t.x > n(250)) {
            second = true;
            assert_eq!(sim.unit(id).unwrap().target.unwrap().x, upper());
            break;
        }
    }
    assert!(second, "queued leg never became active");
}

#[test]
fn station_solid_keeps_own_station_exemption() {
    let db = content(true);
    let mut own = Sim::from_setup(&setup(db.clone(), vec![], vec![("station", 0, p(0, 0))]));
    let own_craft = spawn(&mut own, 0, p(-300, 0));
    accepted(&own.step(&[mv(0, 1, &[own_craft], p(0, 0), false)]).events);
    let mut enemy = Sim::from_setup(&setup(db, vec![], vec![("station", 1, p(0, 0))]));
    let enemy_craft = spawn(&mut enemy, 0, p(-300, 0));
    accepted(
        &enemy
            .step(&[mv(0, 1, &[enemy_craft], p(0, 0), false)])
            .events,
    );
    for _ in 0..450 {
        own.step(&[]);
        enemy.step(&[]);
    }
    assert!((own.unit(own_craft).unwrap().pos.x).abs() < n(40));
    let away = enemy.unit(enemy_craft).unwrap().pos;
    assert!(
        away.x * away.x + away.z * away.z >= n(105) * n(105),
        "{away:?}"
    );
}

#[test]
fn border_sphere_contact_stays_inside_and_clear() {
    let db = content(true);
    let mut sim = Sim::from_setup(&setup(db, vec![], vec![("station", 1, p(350, 0))]));
    let id = spawn(&mut sim, 0, p(350, -300));
    accepted(&sim.step(&[mv(0, 1, &[id], p(467, 0), false)]).events);
    let first = sim.unit(id).unwrap().pos;
    for _ in 0..450 {
        sim.step(&[]);
    }
    let at = sim.unit(id).unwrap().pos;
    assert!(at.z > first.z, "no legal inward progress: {at:?}");
    assert!(
        at.x >= n(-400) && at.x <= n(400) && at.z >= n(-400) && at.z <= n(400),
        "{at:?}"
    );
    let dx = at.x - n(350);
    assert!(
        dx * dx + at.z * at.z >= n(105) * n(105),
        "clipped into blocker: {at:?}"
    );
}

#[test]
fn blocked_preferred_detour_uses_inward_side() {
    // Traveling left makes the old preferred tangent point above the upper
    // Z comparison limit. The alternate tangent remains inside the map.
    let db = content(true);
    let mut sim = Sim::from_setup(&setup(db, vec![], vec![("station", 1, p(0, 350))]));
    let key = sim.content().unwrap().key("craft").unwrap();
    let id = sim
        .spawn_def(key, PlayerId(0), p(300, 350), (-Fixed::ONE, Fixed::ZERO))
        .unwrap();
    accepted(&sim.step(&[mv(0, 1, &[id], p(-300, 350), false)]).events);
    let mut crossed = false;
    for _ in 0..600 {
        sim.step(&[]);
        let at = sim.unit(id).unwrap().pos;
        assert!(at.z <= n(400), "escaped volume while detouring: {at:?}");
        if at.x < n(-120) {
            crossed = true;
            assert!(at.z < n(350), "did not take inward side: {at:?}");
            break;
        }
    }
    assert!(crossed, "alternate detour did not carry craft past blocker");
}

#[test]
fn fully_occluded_space_holds_and_snapshot_keeps_logical_target() {
    let wall = Region {
        kind: RegionKind::AsteroidField,
        odf: "synthetic_wall".into(),
        shape: RegionShape::Box {
            min: p(-400, -400),
            max: p(400, 400),
        },
    };
    let db = content(false);
    let mut sim = Sim::from_setup(&setup(db.clone(), vec![wall], vec![]));
    let id = spawn(&mut sim, 0, p(-400, 0));
    accepted(&sim.step(&[mv(0, 1, &[id], p(300, 0), false)]).events);
    for _ in 0..60 {
        assert_eq!(sim.unit(id).unwrap().target, Some(p(300, 0)));
        sim.step(&[]);
    }
    let saved = sim.snapshot();
    let mut loaded = Sim::from_snapshot(&saved, Some(db)).unwrap();
    assert_eq!(loaded.unit(id).unwrap().target, Some(p(300, 0)));
    assert_eq!(loaded.digest(), sim.digest());
    for _ in 0..30 {
        sim.step(&[]);
        loaded.step(&[]);
    }
    assert_eq!(loaded.digest(), sim.digest());
    assert_eq!(loaded.unit(id).unwrap().target, Some(p(300, 0)));
}

fn walled_in() -> (Sim, EntityId, Arc<ContentDb>) {
    let wall = Region {
        kind: RegionKind::AsteroidField,
        odf: "synthetic_wall".into(),
        shape: RegionShape::Box {
            min: p(-400, -400),
            max: p(400, 400),
        },
    };
    let db = content(false);
    let mut sim = Sim::from_setup(&setup(db.clone(), vec![wall], vec![]));
    let id = spawn(&mut sim, 0, p(-400, 0));
    (sim, id, db)
}

fn blocked(events: &[SimEvent]) -> usize {
    events.iter().filter(|e| matches!(e, SimEvent::OrderBlocked { .. })).count()
}

#[test]
fn an_order_into_a_wall_is_reported_at_once_even_from_rest_and_ends_after_two_minutes() {
    let (mut sim, id, _) = walled_in();
    let out = sim.step(&[mv(0, 1, &[id], p(300, 0), false)]);
    accepted(&out.events);
    let mut reports = blocked(&out.events);
    for _ in 0..90 {
        reports += blocked(&sim.step(&[]).events);
    }
    assert_eq!(reports, 1, "reported once, the first tick the craft cannot go");
    assert_eq!(sim.unit(id).unwrap().target, Some(p(300, 0)), "SIM-22 keeps the order while it holds");
    // Two minutes (30 Hz) after the order with no progress the watch ends it.
    for _ in 0..3700 {
        reports += blocked(&sim.step(&[]).events);
    }
    assert_eq!(reports, 1, "no second report for the same order");
    assert_eq!(sim.unit(id).unwrap().target, None, "the stuck order ended");
    // A new order starts afresh.
    sim.step(&[mv(0, 2, &[id], p(-300, 300), false)]);
    assert!(sim.unit(id).unwrap().target.is_some());
}

#[test]
fn a_restored_snapshot_times_a_stuck_order_out_on_the_same_tick() {
    let (mut sim, id, db) = walled_in();
    sim.step(&[mv(0, 1, &[id], p(300, 0), false)]);
    for _ in 0..1000 {
        sim.step(&[]);
    }
    let mut loaded = Sim::from_snapshot(&sim.snapshot(), Some(db)).unwrap();
    for tick in 0..2700 {
        let a = sim.step(&[]);
        let b = loaded.step(&[]);
        assert_eq!(sim.unit(id).unwrap().target, loaded.unit(id).unwrap().target, "tick {tick}");
        assert_eq!(a.events, b.events, "tick {tick}");
    }
    assert_eq!(sim.unit(id).unwrap().target, None);
}
