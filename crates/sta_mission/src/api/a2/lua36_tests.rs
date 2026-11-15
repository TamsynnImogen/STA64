//! LUA-36: the Armada II binding batch. Fixtures are authored here (fake content, two
//! players); no installed game data.

use std::sync::Arc;

use sta_content::{ContentDb, MapDef, fake};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{AutonomyKind, AutonomyLevel, Command, CommandKind, Edit, Order};
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::WorldView;
use sta_sim::{Sim, SimEvent, WorldSource};

use crate::api::MissionConfig;
use crate::runtime::{MissionEndState, MissionRuntime};

struct Scene {
    sim: Sim,
    view: WorldView,
    db: Arc<ContentDb>,
    /// Cruisers of player 0 (x=0) and 1 (x=100), a player-0 station (x=300), a far player-1 frigate.
    ids: [EntityId; 4],
}

fn game(db: &Arc<ContentDb>) -> GameSetup {
    let seat = |n| PlayerSetup { id: PlayerId(n), team: TeamId(n), race: "fx".into(), control: Control::Idle };
    GameSetup {
        seed: 3,
        map: Arc::new(MapDef::empty()),
        content: db.clone(),
        players: vec![seat(0), seat(1)],
        options: GameOptions::default(),
    }
}

/// `first_owner`: the owner of the unit at x = 0 (`NEUTRAL.0` is no player).
fn scene(first_owner: u8) -> Scene {
    let db = Arc::new(fake::db());
    let mut sim = Sim::from_setup(&game(&db));
    let spawn = |seq, odf: &str, player, x: f32| Command {
        player: PlayerId::SCRIPT,
        seq,
        kind: CommandKind::Edit(Edit::Spawn {
            odf: db.key(odf).expect("fake odf"),
            player: PlayerId(player),
            pos: Vec3::from_f32([x, 0.0, 0.0]),
            heading: (Fixed::ZERO, Fixed::ONE),
        }),
    };
    let out = sim.step(&[
        spawn(1, "fx_cruiser", first_owner, 0.0),
        spawn(2, "fx_cruiser", 1, 100.0),
        spawn(3, "fx_station", 0, 300.0),
        spawn(4, "fx_frigate", 1, 5000.0),
    ]);
    let at = |x: f32| out.view.units.iter().find(|u| u.pos == Vec3::from_f32([x, 0.0, 0.0])).unwrap().id;
    let view = crate::api::special_forces::flagged_view(&out.view, &db);
    Scene { ids: [at(0.0), at(100.0), at(300.0), at(5000.0)], view, db, sim }
}

fn runtime(s: &Scene) -> MissionRuntime {
    let cfg = MissionConfig { content: Some(s.db.clone()), view: Some(s.view.clone()), ..MissionConfig::default() };
    let rt = MissionRuntime::from_source_with("lua36.lua", "", 3, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt.eval::<()>(&format!(
        "a, b, s, far = {}, {}, {}, {}; A2.paths.P = {{ {{x=10,y=0,z=0}}, {{x=20,y=0,z=0}} }}",
        s.ids[0].0, s.ids[1].0, s.ids[2].0, s.ids[3].0
    ))
    .unwrap();
    rt
}

fn unsupported(rt: &MissionRuntime, needle: &str) -> u32 {
    rt.unsupported().iter().filter(|(k, _)| k.contains(needle)).map(|(_, n)| *n).sum()
}

fn kinds(rt: &mut MissionRuntime) -> Vec<(PlayerId, CommandKind)> {
    rt.take_commands().into_iter().map(|c| (c.player, c.kind)).collect()
}

/// The queued commands, after the sim has taken them: none may be rejected.
fn kinds_stepped(rt: &mut MissionRuntime, s: &mut Scene) -> (Vec<(PlayerId, CommandKind)>, WorldView) {
    let commands = rt.take_commands();
    let out = s.sim.step(&commands);
    let rejected: Vec<_> = out.events.iter().filter(|e| matches!(e, SimEvent::Rejected { .. })).collect();
    assert!(rejected.is_empty(), "{rejected:?}");
    (commands.into_iter().map(|c| (c.player, c.kind)).collect(), (*out.view).clone())
}

#[test]
fn obstacle_avoidance_is_an_accepted_no_op_and_not_a_gap() {
    let s = scene(0);
    let rt = runtime(&s);
    rt.eval::<()>("A2.ObstacleAvoidance(false); A2.ObstacleAvoidance(true)").unwrap();
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
    assert!(rt.eval::<()>("A2.ObstacleAvoidance(0)").is_err(), "the argument is still checked");
    // A gap call would taint what it names; this one taints nothing, so an end is not blocked.
    rt.eval::<()>(&format!("A2.ObstacleAvoidance(false); if A2.IsAlive({}) then A2.SucceedMission(0, 0) end", s.ids[0].0)).unwrap();
    assert_eq!(rt.mission_end(s.view.tick + 1000), MissionEndState::Ended { won: true });
}

#[test]
fn an_order_to_a_unit_with_no_player_owner_is_sent_as_the_script_seat() {
    let mut s = scene(sta_sim::world::NEUTRAL.0);
    let mut rt = runtime(&s);
    assert!(!s.view.units.iter().find(|u| u.id == s.ids[0]).unwrap().owner.is_player());
    rt.eval::<()>("A2.Goto(a, 'P', false, 1); A2.Goto(a, b, true, 1); A2.Stop(a, 1)").unwrap();
    let (sent, _) = kinds_stepped(&mut rt, &mut s);
    assert_eq!(sent.len(), 4, "{sent:?}");
    assert!(sent.iter().all(|(p, _)| *p == PlayerId::SCRIPT), "{sent:?}");
    assert!(matches!(&sent[0].1, CommandKind::Order(Order::Move { units, queue: false, .. }) if *units == [s.ids[0]]));
    assert!(matches!(&sent[1].1, CommandKind::Order(Order::Move { queue: true, .. })));
    assert!(matches!(&sent[2].1, CommandKind::Order(Order::MoveWarp { .. })));
    assert!(matches!(&sent[3].1, CommandKind::Order(Order::Stop { .. })));
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
}

#[test]
fn an_owned_unit_is_still_ordered_as_its_owner() {
    let mut s = scene(0);
    let mut rt = runtime(&s);
    rt.eval::<()>("A2.Goto(a, 'P', false, 1)").unwrap();
    let (sent, _) = kinds_stepped(&mut rt, &mut s);
    assert_eq!(sent.len(), 2, "both points of the path: {sent:?}");
    assert!(sent.iter().all(|(p, _)| *p == PlayerId(0)));
}

#[test]
fn move_autonomy_low_orders_the_movement_level_of_a_live_craft() {
    let mut s = scene(sta_sim::world::NEUTRAL.0);
    let mut rt = runtime(&s);
    // No owner, no special-forces flag needed.
    rt.eval::<()>("A2.SetCommandMoveAutonomyLow(a); A2.SetCommandMoveAutonomyLow(b)").unwrap();
    let (sent, view) = kinds_stepped(&mut rt, &mut s);
    for id in [s.ids[0], s.ids[1]] {
        let level = view.units.iter().find(|u| u.id == id).and_then(|u| u.autonomy);
        assert_eq!(level.map(|l| l[0]), Some(0), "movement autonomy low is stored for {id:?}");
    }
    let want = |id| CommandKind::Order(Order::Autonomy { units: vec![id], kind: AutonomyKind::Movement, level: AutonomyLevel::Low });
    assert_eq!(sent, vec![(PlayerId::SCRIPT, want(s.ids[0])), (PlayerId::SCRIPT, want(s.ids[1]))]);
    // A handle naming nothing live does nothing; a malformed one raises.
    rt.eval::<()>("A2.SetCommandMoveAutonomyLow(0); A2.SetCommandMoveAutonomyLow(123456)").unwrap();
    assert!(rt.take_commands().is_empty());
    assert!(rt.eval::<()>("A2.SetCommandMoveAutonomyLow()").is_err());
    assert!(rt.eval::<()>("A2.SetCommandMoveAutonomyLow('a')").is_err());
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
}

#[test]
fn the_command_calls_wait_for_a_handle_still_being_built() {
    let mut s = scene(0);
    let cfg = MissionConfig { content: Some(s.db.clone()), view: Some((*s.sim.view()).clone()), ..MissionConfig::default() };
    let src = "A2.paths.Q = { {x=40,y=0,z=0} }; h = A2.BuildObject('fx_cruiser', 1, 'Q', 0); \
               A2.SetCommandMoveAutonomyLow(h); A2.SetCommandWhatWhoParamVO(h, 37, 1, 1, false)";
    let mut rt = MissionRuntime::from_source_with("held.lua", src, 3, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let commands = rt.take_commands();
    assert_eq!(commands.len(), 1, "only the spawn request is queued: {commands:?}");
    assert_eq!(unsupported(&rt, "still being built"), 0, "holding is not a failure");
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
    let out = s.sim.step(&commands);
    let id = out.events.iter().find_map(|e| match e { SimEvent::ScriptSpawned { id, .. } => Some(*id), _ => None }).unwrap();
    rt.think(&out.view, &out.events, &mut Vec::new());
    assert!(rt.eval::<bool>("return A2.Exists(h)").unwrap());
    let sent = kinds(&mut rt);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(matches!(&sent[0].1, CommandKind::Order(Order::Autonomy { units, .. }) if *units == [id]));
    // A command with a parameter has no order in the sim: it is then, and only then, a counted gap.
    assert!(unsupported(&rt, "SetCommandWhatWhoParamVO") >= 1, "{:?}", rt.unsupported());
}

#[test]
fn the_nearest_search_mask_is_a_team_the_object_must_be_visible_to() {
    let s = scene(0);
    let mut rt = runtime(&s);
    let near = |rt: &mut MissionRuntime, call: &str| rt.eval::<i64>(&format!("return {call}")).unwrap();
    let (a, b, st) = (i64::from(s.ids[0].0), i64::from(s.ids[1].0), i64::from(s.ids[2].0));
    assert_eq!(near(&mut rt, "A2.GetNearestObjectNearMe(a, 500, -1, nil, -1, false)"), b, "-1 does not filter");
    for team in 0..2 {
        let visible: Vec<i64> = [a, b, st]
            .into_iter()
            .filter(|id| rt.eval::<bool>(&format!("return A2.IsVisibleToTeam({id}, {team})")).unwrap())
            .collect();
        // The nearest to x = 0 among the others (a is the asker; ties cannot occur here).
        let want = [b, st].into_iter().find(|id| visible.contains(id)).unwrap_or(0);
        assert_eq!(near(&mut rt, &format!("A2.GetNearestObjectNearMe(a, 500, -1, nil, {team}, false)")), want, "team {team}");
        assert_eq!(
            near(&mut rt, &format!("A2.GetNearestObjectNearPosition(Vector(0, 0, 0), 500, -1, nil, {team}, false, a)")),
            want,
            "team {team}"
        );
        // The shift count is taken modulo 32.
        assert_eq!(near(&mut rt, &format!("A2.GetNearestObjectNearMe(a, 500, -1, nil, {}, false)", team + 32)), want);
    }
    // A candidate one team cannot see is skipped for that team and returned for the other.
    let seen = |team: i64| rt.eval::<bool>(&format!("return A2.IsVisibleToTeam(far, {team})")).unwrap();
    let (by0, by1) = (seen(0), seen(1));
    assert!(by1 && !by0, "the far frigate is its own team's and out of the other's sight");
    let far_id = i64::from(s.ids[3].0);
    let pos = "A2.GetNearestObjectNearPosition(Vector(5000, 0, 0), -1, -1, nil, ";
    assert_eq!(near(&mut rt, &format!("{pos}1, false, 0)")), far_id, "visible to team 1");
    assert_ne!(near(&mut rt, &format!("{pos}0, false, 0)")), far_id, "not visible to team 0");
    // A mask naming no seated team matches nothing.
    assert_eq!(near(&mut rt, "A2.GetNearestObjectNearMe(a, 500, -1, nil, 7, false)"), 0);
    assert!(rt.take_commands().is_empty());
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
}

#[test]
fn the_path_form_of_get_distance_answers_for_a_dead_handle_an_absent_path_and_a_far_index() {
    let s = scene(0);
    let rt = runtime(&s);
    let far = 1.000_000_015_047_466_2e30;
    let d = |call: &str| rt.eval::<f64>(&format!("return {call}")).unwrap();
    assert_eq!(d("A2.GetDistance(99999, 'P', 0)"), far, "a handle naming nothing live");
    assert_eq!(d("A2.GetDistance(a, 'NOPE', 0)"), far, "the engine's path lookup is null");
    assert_eq!(rt.absent_paths().len(), 1, "{:?}", rt.absent_paths());
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
    // The index is clamped into the path.
    assert_eq!(d("A2.GetDistance(a, 'P', 0)"), 10.0);
    assert_eq!(d("A2.GetDistance(a, 'P', 1)"), 20.0);
    assert_eq!(d("A2.GetDistance(a, 'P', 7)"), 20.0);
    assert_eq!(d("A2.GetDistance(a, 'P', -3)"), 10.0);
    // A malformed handle is not a dead one, and a missing path table is still a host gap.
    assert!(rt.eval::<f64>("return A2.GetDistance(-1, 'P', 0)").is_err());
    rt.eval::<()>("A2.paths = nil").unwrap();
    assert!(rt.eval::<f64>("return A2.GetDistance(a, 'P', 0)").is_err());
    assert_eq!(unsupported(&rt, "A2.paths is missing"), 1);
}

#[test]
fn a_dead_handle_answer_does_not_block_an_end_unless_a_failed_call_named_it() {
    let s = scene(0);
    let end = |script: &str| {
        let rt = runtime(&s);
        rt.eval::<()>(script).unwrap();
        rt.mission_end(s.view.tick + 1000)
    };
    // A handle that never existed, with no handle-producing call a gap: the answer stands.
    let clean = "if A2.GetDistance(99999, 'P', 0) > 1000 then A2.SucceedMission(0, 0) end";
    assert_eq!(end(clean), MissionEndState::Ended { won: true });
    // The same read after a failed call named the object is tainted: the end waits.
    let named = format!("A2.NoteGap('Special_Attack', {}, 5)\nif A2.IsAlive({}) then A2.SucceedMission(0, 0) end", s.ids[0].0, s.ids[0].0);
    assert!(matches!(end(&named), MissionEndState::Blocked(_)));
}
