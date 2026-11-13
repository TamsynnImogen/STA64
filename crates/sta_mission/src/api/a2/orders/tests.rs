//! Tests of the order, AI and team calls: authored scenes on fake content
//! (two players), no game files.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, UnitDef, fake};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::WorldView;
use sta_sim::setup::Controller;
use sta_sim::{Sim, SimEvent, WorldSource};

use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;

struct Scene {
    sim: Sim,
    view: WorldView,
    db: Arc<ContentDb>,
    /// Cruisers of player 0 and 1, a player-0 station, a far player-1 frigate.
    ids: [EntityId; 4],
}

fn setup(db: &Arc<ContentDb>) -> GameSetup {
    let seat = |n| PlayerSetup { id: PlayerId(n), team: TeamId(n), race: "fx".into(), control: Control::Idle };
    GameSetup {
        seed: 3,
        map: Arc::new(MapDef::empty()),
        content: db.clone(),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    }
}

fn spawn_cmd(db: &ContentDb, seq: u32, odf: &str, player: u8, at: [f32; 3]) -> Command {
    Command {
        player: PlayerId::SCRIPT,
        seq,
        kind: CommandKind::Edit(Edit::Spawn {
            odf: db.key(odf).expect("fake odf"),
            player: PlayerId(player),
            pos: Vec3::from_f32(at),
            heading: (Fixed::ZERO, Fixed::ONE),
        }),
    }
}

fn scene_with(db: Arc<ContentDb>, odfs: [&str; 4], at: [[f32; 3]; 4]) -> Scene {
    let mut sim = Sim::from_setup(&setup(&db));
    let players = [0, 1, 0, 1];
    let cmds: Vec<Command> =
        (0..4).map(|i| spawn_cmd(&db, i as u32 + 1, odfs[i], players[i], at[i])).collect();
    let out = sim.step(&cmds);
    let find = |p: [f32; 3]| out.view.units.iter().find(|u| u.pos == Vec3::from_f32(p)).expect("scene unit").id;
    let ids = [find(at[0]), find(at[1]), find(at[2]), find(at[3])];
    Scene { sim, view: (*out.view).clone(), db, ids }
}

fn scene() -> Scene {
    scene_with(
        Arc::new(fake::db()),
        ["fx_cruiser", "fx_cruiser", "fx_station", "fx_frigate"],
        [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [300.0, 0.0, 0.0], [5000.0, 0.0, 0.0]],
    )
}

const PRELUDE: &str = r#"
    A2.paths.P = { {x=10,y=0,z=0}, {x=20,y=0,z=0}, {x=30,y=0,z=0} }
    A2.paths.Q = { {x=500,y=0,z=0} }
    A2.paths.F = { {x=2000,y=0,z=0}, {x=2500,y=0,z=0} }
"#;

fn config(s: &Scene) -> MissionConfig {
    MissionConfig { content: Some(s.db.clone()), view: Some(s.view.clone()), ..MissionConfig::default() }
}

fn runtime(s: &Scene) -> MissionRuntime {
    let rt = MissionRuntime::from_source_with("orders.lua", "", 3, config(s));
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt.eval::<()>(&format!(
        "a, b, s, far = {}, {}, {}, {}; {PRELUDE}",
        s.ids[0].0, s.ids[1].0, s.ids[2].0, s.ids[3].0
    ))
    .unwrap();
    rt
}

fn run(s: &Scene, script: &str) -> (MissionRuntime, Vec<Command>) {
    let mut rt = runtime(s);
    rt.eval::<()>(script).unwrap_or_else(|e| panic!("{script}: {e}"));
    let commands = rt.take_commands();
    (rt, commands)
}

fn fails(s: &Scene, script: &str) -> (MissionRuntime, String) {
    let mut rt = runtime(s);
    let err = rt.eval::<()>(script).expect_err(script).to_string();
    assert!(rt.take_commands().is_empty(), "{script} queued commands before failing");
    (rt, err)
}

fn counted(rt: &MissionRuntime, what: &str) -> bool {
    rt.unsupported().iter().any(|(k, _)| k.starts_with(what))
}

fn orders(commands: &[Command]) -> Vec<(PlayerId, Order)> {
    commands
        .iter()
        .map(|c| match &c.kind {
            CommandKind::Order(o) => (c.player, o.clone()),
            other => panic!("not an order: {other:?}"),
        })
        .collect()
}

fn at(x: i32) -> Vec3 {
    Vec3 { x: Fixed::from_int(x), y: Fixed::ZERO, z: Fixed::ZERO }
}

#[test]
fn goto_a_path_moves_through_its_points_in_order_as_the_owner() {
    let s = scene();
    let (_, commands) = run(&s, "A2.Goto(a, 'P', false, 1)");
    let a = s.ids[0];
    assert_eq!(
        orders(&commands),
        vec![
            (PlayerId(0), Order::Move { units: vec![a], to: at(10), queue: false }),
            (PlayerId(0), Order::Move { units: vec![a], to: at(20), queue: true }),
            (PlayerId(0), Order::Move { units: vec![a], to: at(30), queue: true }),
        ]
    );
}

#[test]
fn goto_an_object_moves_to_its_position_and_ignores_the_trailing_int() {
    let s = scene();
    let (rt, commands) = run(&s, "A2.Goto(a, b, false, 1); A2.Goto(a, s, false, 99)");
    let a = s.ids[0];
    assert_eq!(
        orders(&commands),
        vec![
            (PlayerId(0), Order::Move { units: vec![a], to: at(100), queue: false }),
            (PlayerId(0), Order::Move { units: vec![a], to: at(300), queue: false }),
        ]
    );
    assert!(rt.unsupported().is_empty());
}

#[test]
fn goto_with_the_warp_flag_is_a_warp_move_on_the_first_leg() {
    let s = scene();
    let a = s.ids[0];
    let (_, commands) = run(&s, "A2.Goto(a, 'P', true, 1); A2.Goto(a, b, true, 1)");
    assert_eq!(
        orders(&commands),
        vec![
            (PlayerId(0), Order::MoveWarp { units: vec![a], to: at(10), queue: false }),
            (PlayerId(0), Order::Move { units: vec![a], to: at(20), queue: true }),
            (PlayerId(0), Order::Move { units: vec![a], to: at(30), queue: true }),
            (PlayerId(0), Order::MoveWarp { units: vec![a], to: at(100), queue: false }),
        ]
    );
}

#[test]
fn goto_a_path_the_host_did_not_fill_is_counted() {
    let s = scene();
    let (rt, err) = fails(&s, "A2.Goto(a, 'NOPE', false, 1)");
    assert!(err.contains("A2.paths"), "{err}");
    assert!(counted(&rt, "A2.Goto: path 'NOPE'"));
}

#[test]
fn malformed_calls_fail_before_anything_is_queued() {
    let s = scene();
    for script in [
        "A2.Goto(a, 'P', false)",
        "A2.Goto(a, 'P', 0, 1)",
        "A2.Goto('a', 'P', false, 1)",
        "A2.Goto(a, 'P', false, 1.5)",
        "A2.Stop(a)",
        "A2.Stop(a, 1, 2)",
        "A2.Attack(a, b)",
        "A2.AI_Attack(a, 'b')",
        "A2.Mine(a)",
        "A2.SetImpulseSpeed(a, 'fast')",
        "A2.SetImpulseSpeed(a, 0/0)",
        "A2.SetRedAlert()",
        "A2.GetNearestEnemy(a)",
        "A2.BuildObject('fx_frigate', 1, 'P')",
    ] {
        fails(&s, script);
    }
    let (_, err) = fails(&s, "A2.SetImpulseSpeed(a, -1)");
    assert!(err.contains("non-negative"), "{err}");
}

#[test]
fn an_absent_or_dying_unit_is_left_alone_like_the_engine_does() {
    let mut s = scene();
    let dying = s.ids[1];
    s.view.units.iter_mut().find(|u| u.id == dying).unwrap().flags = sta_sim::view::UnitFlags::DYING;
    let (rt, commands) = run(
        &s,
        "A2.Goto(9999, 'P', false, 1); A2.Goto(0, b, false, 1); A2.Goto(a, 9999, false, 1); \
         A2.Stop(9999, 1); A2.Attack(a, b, 1); A2.Attack(b, a, 1); A2.AI_Attack(9999, a); \
         A2.Mine(9999, a); A2.SetImpulseSpeed(9999, 5); A2.ResetImpulseSpeed(9999); \
         A2.SetRedAlert(9999); A2.SetGreenAlert(b); assert(A2.GetImpulseSpeed(9999) == 0); \
         assert(A2.GetNearestEnemy(9999, 100) == 0)",
    );
    assert!(commands.is_empty(), "{commands:?}");
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
}

#[test]
fn stop_attack_and_ai_attack_are_owner_orders_and_ignore_the_extra_int() {
    let s = scene();
    let (a, b) = (s.ids[0], s.ids[1]);
    let (_, commands) = run(&s, "A2.Stop(a, 1); A2.Attack(a, b, 1); A2.AI_Attack(a, b); A2.Attack(b, a, 77)");
    assert_eq!(
        orders(&commands),
        vec![
            (PlayerId(0), Order::Stop { units: vec![a] }),
            (PlayerId(0), Order::Attack { units: vec![a], target: b }),
            (PlayerId(0), Order::Attack { units: vec![a], target: b }),
            (PlayerId(1), Order::Attack { units: vec![b], target: a }),
        ]
    );
}

#[test]
fn ai_attack_without_a_target_or_toward_a_point_and_the_scripted_orders_are_orders() {
    let s = scene();
    let a = s.ids[0];
    let (rt, commands) = run(&s, "A2.AI_Attack(a); A2.AI_Attack(a, {x=1,y=2,z=3}); A2.Patrol(a, 'P'); A2.SearchAndDestroy(a); A2.Scout(a)");
    assert_eq!(
        orders(&commands),
        vec![
            (PlayerId(0), Order::AttackAny { units: vec![a] }),
            (PlayerId(0), Order::AttackPoint { units: vec![a], at: Vec3::from_f32([1.0, 2.0, 3.0]) }),
            (PlayerId(0), Order::Patrol { units: vec![a], points: vec![at(10), at(20), at(30)] }),
            (PlayerId(0), Order::SearchAndDestroy { units: vec![a] }),
            (PlayerId(0), Order::Scout { units: vec![a] }),
        ]
    );
    assert!(rt.unsupported().is_empty());
    // A dead or absent unit does nothing, no error.
    let (rt, commands) = run(&s, "A2.Patrol(9999, 'P'); A2.Scout(9999); A2.SearchAndDestroy(0)");
    assert!(commands.is_empty() && rt.unsupported().is_empty());
}

#[test]
fn mine_orders_the_node_and_an_absent_node_needs_one_in_the_view() {
    let s = scene();
    let a = s.ids[0];
    let (_, commands) = run(&s, "A2.Mine(a, s)");
    assert_eq!(orders(&commands), vec![(PlayerId(0), Order::Mine { units: vec![a], node: s.ids[2] })]);
    // No resource in this scene: Mine without a target is counted, not guessed.
    let (rt, _) = fails(&s, "A2.Mine(a, 0)");
    assert!(counted(&rt, "A2.Mine: no mineable"));
}

#[test]
fn impulse_speed_is_a_typed_edit_and_the_getter_follows_it() {
    let s = scene();
    let a = s.ids[0];
    let class = s.db.unit(s.db.key("fx_cruiser").unwrap()).unwrap().movement.max_speed.to_f32();
    let (mut rt, commands) = run(&s, "A2.SetImpulseSpeed(a, 28); assert(A2.GetImpulseSpeed(a) == 28.0)");
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].player, PlayerId::SCRIPT);
    assert_eq!(commands[0].kind, CommandKind::Edit(Edit::MaxSpeed(a, Some(Fixed::from_int(28)))));
    rt.eval::<()>("A2.ResetImpulseSpeed(a)").unwrap();
    let reset = rt.take_commands();
    assert_eq!(reset[0].kind, CommandKind::Edit(Edit::MaxSpeed(a, None)));
    let got: f64 = rt.eval("return A2.GetImpulseSpeed(a)").unwrap();
    assert_eq!(got, f64::from(class));
}

#[test]
fn the_sim_rejecting_an_impulse_or_alert_edit_is_reported_and_counted() {
    let mut s = scene();
    let mut rt = runtime(&s);
    rt.eval::<()>("A2.SetImpulseSpeed(a, 28); A2.SetRedAlert(a); A2.SetGreenAlert(b)").unwrap();
    // The host submits them, the sim answers; the runtime reports on a later view.
    driven(&mut s, &mut rt, 5);
    let counted_edits: Vec<_> = rt.unsupported().into_iter().map(|(k, _)| k).collect();
    for label in ["A2.SetImpulseSpeed", "A2.SetRedAlert", "A2.SetGreenAlert"] {
        let wanted = format!("{label}: simulation rejected edit");
        assert!(counted_edits.contains(&wanted), "{wanted} in {counted_edits:?}");
    }
}

#[test]
fn calls_with_no_seam_validate_then_raise_and_are_counted() {
    let s = scene();
    for (script, key) in [
        ("A2.Special_Attack(a, 'gquanteffect')", "A2.Special_Attack"),
        ("A2.Special_Attack(a, b, 'gtracbm')", "A2.Special_Attack"),
        ("A2.SetCommandMoveAutonomyLow(a)", "A2.SetCommandMoveAutonomyLow"),
        ("A2.GetMainEnemyBaseLocation(1)", "A2.GetMainEnemyBaseLocation"),
        ("A2.AttackMainEnemyBase(a, true)", "A2.AttackMainEnemyBase"),
        ("A2.GetAllyStationAttacks(3, 1)", "A2.GetAllyStationAttacks"),
    ] {
        let (rt, err) = fails(&s, script);
        assert!(err.contains("is not available"), "{script}: {err}");
        assert!(counted(&rt, key), "{script}: {:?}", rt.unsupported());
    }
    // A malformed call is an argument error, not a gap.
    let (rt, err) = fails(&s, "A2.Scout()");
    let _ = &rt;
    assert!(err.contains("exactly 1"), "{err}");
    assert!(rt.unsupported().is_empty());
}

fn driven(s: &mut Scene, rt: &mut MissionRuntime, ticks: u32) {
    let mut view = Arc::new(s.view.clone());
    let mut events: Vec<SimEvent> = Vec::new();
    for _ in 0..ticks {
        let mut out = Vec::new();
        rt.think(&view, &events, &mut out);
        let o = s.sim.step(&out);
        view = o.view;
        events = o.events;
    }
    s.view = (*view).clone();
}

#[test]
fn get_nearest_enemy_uses_the_xz_square_the_relation_and_visibility() {
    let s = scene();
    let (b, far) = (s.ids[1], s.ids[3]);
    let _ = far;
    run(&s, "assert(A2.GetNearestEnemy(a, 150) == b); assert(A2.GetNearestEnemy(a, 99) == 0)");
    // The station is an ally and the far frigate is outside the square until it is wide enough.
    run(&s, "assert(A2.GetNearestEnemy(a, 400) == b); assert(A2.GetNearestEnemy(s, 6000) == b)");
    // Height is ignored: a candidate 1000 above the unit is still inside the square.
    let mut high = scene();
    high.view.units.iter_mut().find(|u| u.id == b).unwrap().pos.y = Fixed::from_int(1000);
    run(&high, "assert(A2.GetNearestEnemy(a, 150) == b)");
    // Nearest first: move the far frigate (also an enemy) closer than the cruiser.
    let mut near = scene();
    near.view.units.iter_mut().find(|u| u.id == far).unwrap().pos = at(40);
    run(&near, &format!("assert(A2.GetNearestEnemy(a, 150) == {})", far.0));
    // Visibility: with data saying player 1 cannot see the unit, it is not found.
    let mut blind = scene();
    blind.view.visibility = vec![(PlayerId(0), vec![blind.ids[0], b]), (PlayerId(1), vec![b])];
    run(&blind, "assert(A2.GetNearestEnemy(a, 150) == 0)");
    let mut seen = scene();
    seen.view.visibility = vec![(PlayerId(0), vec![seen.ids[0]]), (PlayerId(1), vec![b, seen.ids[0]])];
    run(&seen, "assert(A2.GetNearestEnemy(a, 150) == b)");
}

fn radius_db(radius: i32) -> Arc<ContentDb> {
    let mut ship = UnitDef::new("t_ship", "craft");
    ship.race = "fx".into();
    ship.model = "t_ship".into();
    ship.hull = Fixed::from_int(100);
    ship.radius = Fixed::from_int(radius);
    Arc::new(ContentDb::builder().unit(ship).build().expect("one-unit content"))
}

#[test]
fn get_attack_location_is_the_point_on_the_bounding_sphere_toward_the_other_object() {
    let s = scene_with(
        radius_db(10),
        ["t_ship", "t_ship", "t_ship", "t_ship"],
        [[0.0, 0.0, 0.0], [0.0, 0.0, 100.0], [-100.0, 0.0, 0.0], [5000.0, 0.0, 0.0]],
    );
    let (a, b) = (s.ids[0], s.ids[1]);
    let rt = runtime(&s);
    let toward: (f64, f64, f64) =
        rt.eval(&format!("local v = A2.GetAttackLocation({}, {}); return v.x, v.y, v.z", a.0, b.0)).unwrap();
    assert_eq!(toward, (0.0, 0.0, 10.0));
    // No second object: one radius along +x.
    let alone: (f64, f64, f64) =
        rt.eval(&format!("local v = A2.GetAttackLocation({}, 0); return v.x, v.y, v.z", a.0)).unwrap();
    assert_eq!(alone, (10.0, 0.0, 0.0));
    let up: (f64, f64, f64) =
        rt.eval(&format!("local v = A2.GetAttackLocation({}, {}); return v.x, v.y, v.z", a.0, s.ids[2].0)).unwrap();
    assert_eq!(up, (-10.0, 0.0, 0.0));
    // No first object, or no radius in the content: counted, never guessed.
    let err = rt.eval::<()>("A2.GetAttackLocation(0, 0)").unwrap_err().to_string();
    assert!(err.contains("absent"), "{err}");
    let plain = scene();
    let (rt, err) = fails(&plain, "A2.GetAttackLocation(a, b)");
    assert!(err.contains("bounding radius"), "{err}");
    assert!(counted(&rt, "A2.GetAttackLocation: the unit's bounding radius"));
}

#[test]
fn command_calls_map_the_supported_commands_to_orders() {
    let s = scene();
    let (a, b) = (s.ids[0], s.ids[1]);
    // 3 STOP, 4 GO, 6 ATTACK, 21 MINE, 25 BUILD, 35 END_TRADE (values read from the executable).
    let (_, commands) = run(
        &s,
        "A2.SetCommandWhatWhoVO(a, 6, b, false); A2.SetCommandWhatWhoVO(a, 4, b, true); \
         A2.SetCommandWhatWhoVO(a, 21, s, false); \
         A2.SetCommandWhatWhoWhereVO(a, 25, 'fx_station.odf', {x=7,y=0,z=9}, false)",
    );
    let build = sta_sim::command::Order::Build {
        builder: a,
        what: s.db.key("fx_station").unwrap(),
        at: Some(Vec3 { x: Fixed::from_int(7), y: Fixed::ZERO, z: Fixed::from_int(9) }),
    };
    assert_eq!(
        orders(&commands),
        vec![
            (PlayerId(0), Order::Attack { units: vec![a], target: b }),
            (PlayerId(0), Order::Move { units: vec![a], to: at(100), queue: false }),
            (PlayerId(0), Order::Mine { units: vec![a], node: s.ids[2] }),
            (PlayerId(0), build),
        ]
    );
    // An absent object means nothing happens (the engine needs both).
    let (rt, commands) = run(&s, "A2.SetCommandWhatWhoVO(a, 46, 0, false); A2.SetCommandWhatWhoParamVO(a, 37, 0, 4, false)");
    assert!(commands.is_empty() && rt.unsupported().is_empty());
    // Other commands have no order.
    let (rt, _) = fails(&s, "A2.SetCommandWhatWhoVO(a, 35, b, false)");
    assert!(counted(&rt, "A2.SetCommandWhatWhoVO: command 35"));
    let (rt, _) = fails(&s, "A2.SetCommandWhatWhoParamVO(a, 37, b, 4, false)");
    assert!(counted(&rt, "A2.SetCommandWhatWhoParamVO: command 37"));
    let (rt, _) = fails(&s, "A2.SetCommandWhatWhoWhereVO(a, 4, 'fx_station', {x=1,y=0,z=1}, false)");
    assert!(counted(&rt, "A2.SetCommandWhatWhoWhereVO: GO at a position"));
    let (_, err) = fails(&s, "A2.SetCommandWhatWhoVO(a, 57, b, false)");
    assert!(err.contains("AiCommand"), "{err}");
    let (_, err) = fails(&s, "A2.SetCommandWhatWhoWhereVO(a, 25, 'no_such', {x=1,y=0,z=1}, false)");
    assert!(err.contains("unknown ODF"), "{err}");
}

#[test]
fn other_a2_calls_still_work_and_keep_their_errors() {
    let s = scene();
    let (rt, commands) = run(&s, "assert(A2.GetRealTeam(a) == 0 and A2.GetRealTeam(b) == 1)");
    assert!(commands.is_empty());
    let err = rt.eval::<()>("A2.GetRealTeam(9999)").unwrap_err().to_string();
    assert!(err.contains("A2.GetRealTeam") && err.contains("not alive"), "{err}");
    // An integer that is not a pending handle is passed through untouched.
    rt.eval::<()>("assert(not A2.Exists(0x7ffffff0))").unwrap();
}

fn edits(commands: &[Command]) -> Vec<Edit> {
    commands
        .iter()
        .map(|c| match &c.kind {
            CommandKind::Edit(e) => {
                assert_eq!(c.player, PlayerId::SCRIPT);
                e.clone()
            }
            other => panic!("not an edit: {other:?}"),
        })
        .collect()
}

#[test]
fn placement_and_facing_calls_queue_typed_script_edits() {
    let s = scene();
    let a = s.ids[0];
    let (rt, commands) = run(
        &s,
        "A2.SetPosition(a, 'P'); A2.SetPosition(a, 'P', 2); A2.PointAt(a, b); A2.PointAt(a, 'F'); \
         A2.SetAngularVelocity(a, Vector(0.1, 0.5, 0.2)); A2.SetPosition(0, 'P'); A2.PointAt(0, b); A2.SetAngularVelocity(0, Vector(0, 1, 0))",
    );
    assert_eq!(
        edits(&commands),
        vec![
            Edit::Position(a, at(10)),
            Edit::Position(a, at(30)),
            // The direction from where the unit now is (30, after its queued SetPosition); the sim normalises it.
            Edit::Heading(a, (Fixed::from_int(70), Fixed::ZERO)),
            Edit::Heading(a, (Fixed::from_int(1970), Fixed::ZERO)),
            // The sim's turn rate is positive from +x toward +z; the engine's yaw is the other way.
            Edit::AngularVelocity(a, Fixed::from_f64(-0.5)),
        ],
        "an absent unit's calls queue nothing"
    );
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
}

#[test]
fn placement_and_facing_calls_reject_bad_input_without_queuing() {
    let s = scene();
    for (script, key) in [
        ("A2.SetPosition(a, 'P', 3)", "A2.SetPosition"),
        ("A2.SetPosition(a, 'Nope')", "A2.SetPosition"),
        ("A2.PointAt(a, 'Nope')", "A2.PointAt"),
    ] {
        let (rt, err) = fails(&s, script);
        assert!(err.contains("is not available"), "{script}: {err}");
        assert!(counted(&rt, key), "{script}");
    }
    for script in [
        "A2.SetPosition(a)", "A2.SetPosition(a, 5)", "A2.SetPosition(a, 'P', 1.5)", "A2.PointAt(a, nil)",
        "A2.SetAngularVelocity(a, 1)", "A2.SetAngularVelocity(a, Vector(0, 1/0, 0))",
        "A2.SetAngularVelocity(a, Vector(0, 0/0, 0))",
    ] {
        let (_, err) = fails(&s, script);
        assert!(err.contains("A2."), "{script}: {err}");
    }
    // A target at the unit's own place has no direction: counted, nothing queued, no error.
    let (rt, commands) = run(&s, "A2.PointAt(a, a)");
    assert!(commands.is_empty());
    assert!(counted(&rt, "A2.PointAt"));
}

#[test]
fn queued_placement_edits_land_in_the_sim() {
    let mut s = scene();
    let (a, b) = (s.ids[0], s.ids[1]);
    let (_, commands) = run(&s, "A2.SetPosition(a, 'F', 1); A2.PointAt(a, 'P')");
    let out = s.sim.step(&commands);
    assert!(!out.events.iter().any(|e| matches!(e, sta_sim::SimEvent::Rejected { .. })));
    let u = s.sim.unit(a).unwrap();
    assert_eq!(u.pos, at(2500));
    // From (2500, 0, 0) toward (10, 0, 0): facing -x.
    assert_eq!(u.heading, (-Fixed::ONE, Fixed::ZERO));
    assert_eq!(s.sim.unit(b).unwrap().pos, at(100));
}

#[test]
fn ai_team_load_aip_queues_the_request_for_the_host() {
    let s = scene();
    let (rt, _) = run(&s, "A2.AI_Team_Load_AIP(2, 'x.aip'); A2.AI_Team_Load_AIP(2, 'Y.AIP')");
    assert_eq!(rt.unsupported(), [("Player.loadAIP".to_owned(), 2)], "counted until a host takes them");
    let got = rt.take_aip_requests();
    assert!(rt.unsupported().is_empty());
    assert_eq!(got.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>(), ["x.aip", "Y.AIP"]);
    assert!(rt.take_aip_requests().is_empty(), "taken once");
}
