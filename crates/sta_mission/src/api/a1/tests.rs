//! Per-function tests of the `A1` table. A shared scene (fake content, two
//! players) backs a table of valid calls, one per imported signature, and
//! behaviour tests for what each family queues. Not stock-mission tests.

use std::collections::BTreeSet;
use std::sync::Arc;

use sta_content::{ContentDb, MapDef, fake};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Vec3};
use sta_sim::command::{AlertState, Command, CommandKind, Edit, Order, Subsystem};
use sta_sim::present::{Arg, Presentation};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::view::{Relation, WorldView};
use sta_sim::{Sim, SimEvent, WorldSource};

use crate::api::MissionConfig;
use crate::runtime::MissionRuntime;

struct Scene {
    view: WorldView,
    db: Arc<ContentDb>,
    /// Cruisers of player 0 and 1, a player-0 station, a far player-1 frigate.
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

fn scene() -> Scene {
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
        spawn(1, "fx_cruiser", 0, 0.0),
        spawn(2, "fx_cruiser", 1, 100.0),
        spawn(3, "fx_station", 0, 300.0),
        spawn(4, "fx_frigate", 1, 5000.0),
    ]);
    let units: Vec<EntityId> = out.view.units.iter().map(|u| u.id).collect();
    assert_eq!(units.len(), 4, "scene units");
    let at = |x: f32| out.view.units.iter().find(|u| u.pos == Vec3::from_f32([x, 0.0, 0.0])).unwrap().id;
    Scene { ids: [at(0.0), at(100.0), at(300.0), at(5000.0)], view: (*out.view).clone(), db }
}

const PRELUDE: &str = r#"
    A1.paths.P = { {x=10,y=0,z=0}, {x=20,y=0,z=0}, {x=30,y=0,z=0} }
    A1.paths.Q = { {x=500,y=0,z=0} }
    f = A1.CreateSpecialFlag(a, 'Hero')
"#;

fn runtime(s: &Scene) -> MissionRuntime {
    let cfg = MissionConfig {
        content: Some(s.db.clone()),
        view: Some(s.view.clone()),
        labels: [(s.ids[0], "hero".to_owned())].into_iter().collect(),
        ..MissionConfig::default()
    };
    let mut rt = MissionRuntime::from_source_with("a1.lua", "", 3, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt.eval::<()>(&format!(
        "a, b, s, far = {}, {}, {}, {}; {PRELUDE}",
        s.ids[0].0, s.ids[1].0, s.ids[2].0, s.ids[3].0
    ))
    .unwrap();
    rt.take_commands();
    rt
}

fn run(s: &Scene, script: &str) -> (MissionRuntime, Vec<Command>) {
    let mut rt = runtime(s);
    rt.eval::<()>(script).unwrap_or_else(|e| panic!("{script}: {e}"));
    let commands = rt.take_commands();
    (rt, commands)
}

fn fails(s: &Scene, script: &str) -> String {
    let mut rt = runtime(s);
    let err = rt.eval::<()>(script).expect_err(script).to_string();
    assert!(rt.take_commands().is_empty(), "{script} queued commands before failing");
    err
}

fn presented(commands: &[Command]) -> Vec<Presentation> {
    commands
        .iter()
        .map(|c| match &c.kind {
            CommandKind::Edit(Edit::Present(p)) => {
                assert_eq!(c.player, PlayerId::SCRIPT);
                p.clone()
            }
            other => panic!("not a presentation: {other:?}"),
        })
        .collect()
}

fn call(name: &str, args: Vec<Arg>) -> Presentation {
    Presentation::Call { name: format!("A1.{name}"), args }
}

#[derive(Clone, Copy)]
enum Expect {
    Ok,
    /// A documented error (query without data, or a missing precondition).
    Err,
    /// The A2 binding it reuses depends on content the scene may lack.
    Either,
}
use Expect::{Either, Err as Fail, Ok as Fine};

/// One valid call per imported signature (overloads separately).
const CASES: &[(&str, &str, Expect)] = &[
    // Bridge scenes and movies.
    ("PreLoadBridgeScene", "A1.PreLoadBridgeScene(1)", Fine),
    ("SetBridgeSceneRace", "A1.SetBridgeSceneRace(4)", Fine),
    ("ShowBridgeScene", "A1.ShowBridgeScene(true)", Fine),
    ("ShowBridgeOnScreen", "A1.ShowBridgeOnScreen(false)", Fine),
    ("PlayBridgeMovie", "A1.PlayBridgeMovie('intro_a.bik', 1)", Fine),
    ("StopMovie", "A1.StopMovie()", Fine),
    ("IsMoviePlaying", "assert(A1.IsMoviePlaying() == false)", Fine),
    // Cineractives and camera.
    ("CineractiveStart", "A1.CineractiveStart()", Fine),
    ("CineractiveEnd", "A1.CineractiveEnd()", Fine),
    ("CineractiveShot", "A1.CineractiveShot(a, b)", Fine),
    ("CineractiveShot", "A1.CineractiveShot('P', 0.5, b)", Fine),
    ("CineractiveShot", "A1.CineractiveShot('P', 1, b)", Fine),
    ("CineractiveShot", "A1.CineractiveShot('P', 0.5, 'Q', 1)", Fine),
    ("CineractiveShot", "A1.CineractiveShot('P', 1, 'Q', 1)", Fine),
    ("CineractiveShot", "A1.CineractiveShot('P', 0.5, 'Q', 0.25)", Fine),
    ("CineractiveShot", "A1.CineractiveShot('P', 1, 'Q', 0.25)", Fine),
    ("FadeIn", "A1.FadeIn(1, 0, 0)", Fine),
    ("FadeOut", "A1.FadeOut(2, 0.5, 0)", Fine),
    ("CineractiveCameraOffset", "A1.CineractiveCameraOffset('P', 0.3, true)", Fine),
    ("CineractiveCameraOffset", "A1.CineractiveCameraOffset(60, 80, 80, true)", Fine),
    ("CenterCamera", "A1.CenterCamera('cam_path', 0)", Fine),
    // Titles, audio, objectives.
    ("AudioMessage", "assert(A1.AudioMessage('m.wav') == 1)", Fine),
    ("StopAudioMessage", "A1.StopAudioMessage(A1.AudioMessage('m.wav'))", Fine),
    ("IsAudioMessageDone", "assert(A1.IsAudioMessageDone(7) == true)", Fine),
    ("ObjectivesDisplay_Set_Visibility", "A1.ObjectivesDisplay_Set_Visibility(false)", Fine),
    ("ObjectivesDisplay_Set_Text_From_File", "A1.ObjectivesDisplay_Set_Text_From_File('none.txt')", Fine),
    ("ShowTitle", "assert(A1.ShowTitle(1, 2, 3, 4, 'T', true) == 1)", Fine),
    ("RemoveTitle", "A1.RemoveTitle(A1.ShowTitle(1, 2, 3, 4, 'T', true), true)", Fine),
    ("ShowSubtitle", "assert(A1.ShowSubtitle(5, 10, 300, 20, 'S', true) == 1)", Fine),
    ("RemoveSubtitle", "A1.RemoveSubtitle(A1.ShowSubtitle(5, 10, 300, 20, 'S', true), true)", Fine),
    ("RemoveSubtitle", "A1.RemoveSubtitle(0, true)", Fine),
    ("MinimapMessage", "A1.MinimapMessage(1, 2, 3.0)", Fine),
    ("MinimapMessage", "A1.MinimapMessage('m', 1, 2.0, 3)", Fine),
    // Orders and AI.
    ("Goto", "A1.Goto(a, 'P', 1)", Fine),
    ("Goto", "A1.Goto(a, b, 1)", Fine),
    ("BuildObject", "assert(A1.BuildObject('fx_cruiser', 0, 'Q', 0) >= 0x40000000)", Fine),
    ("BuildObject", "assert(A1.BuildObject('fx_cruiser', 0, 0, 1.0, 2.0, 3.0) >= 0x40000000)", Fine),
    ("SetWarpSpeed", "A1.SetWarpSpeed(a, 30)", Fine),
    ("ResetWarpSpeed", "A1.ResetWarpSpeed(a)", Fine),
    ("Stop", "A1.Stop(a, 1)", Fine),
    ("AI_Team_Load_AIP", "A1.AI_Team_Load_AIP(1, 'team.aip')", Fine),
    ("Attack", "A1.Attack(a, b, 1)", Fine),
    ("Attack", "A1.Attack(a, Vector(1, 2, 3))", Fine),
    ("AI_Attack", "A1.AI_Attack(1, 0)", Fine),
    ("AI_Attack", "A1.AI_Attack(1, Vector(1, 2, 3))", Fine),
    ("AI_Attack", "A1.AI_Attack(1)", Fine),
    ("SetImpulseSpeed", "A1.SetImpulseSpeed(a, 20)", Fine),
    ("SetSpecialWeaponAttackInterval", "A1.SetSpecialWeaponAttackInterval(2, 1.0)", Fine),
    ("Build", "A1.Build(s, 'fx_cruiser')", Fine),
    ("Patrol", "A1.Patrol(a, 'P')", Fine),
    ("Special_Attack", "A1.Special_Attack(a, Vector(1, 2, 3), 'quantum')", Fine),
    ("Special_Attack", "A1.Special_Attack(a, b, 'quantum')", Fine),
    ("Special_Attack", "A1.Special_Attack(a, 'quantum')", Fine),
    ("StopAllShips", "A1.StopAllShips(1, nil, true, true)", Fine),
    ("InStopMode", "A1.InStopMode(a)", Fail),
    ("OutOfSpecialAttackMode", "A1.OutOfSpecialAttackMode(a)", Fail),
    ("Mine", "A1.Mine(a, b)", Fine),
    ("GetAttackLocation", "A1.GetAttackLocation(a, b)", Fail),
    ("InAttackMode", "assert(A1.InAttackMode(a) == false)", Fine),
    ("UnderAttack", "assert(A1.UnderAttack(a, 5.0) == false)", Fine),
    ("GetAllyShipAttacks", "A1.GetAllyShipAttacks(4, 1)", Fail),
    ("GetAllyStationAttacks", "A1.GetAllyStationAttacks(4, 1)", Fail),
    ("AttackMainEnemyBase", "A1.AttackMainEnemyBase(1, true)", Fine),
    ("InQueue", "assert(A1.InQueue(a) == false)", Fine),
    ("SearchAndDestroy", "A1.SearchAndDestroy(a)", Fine),
    // Resources and crew.
    ("SetDilithium", "assert(A1.SetDilithium(0, 500) == 500)", Fine),
    ("AddDilithium", "assert(A1.AddDilithium(0, 10) >= 10)", Fine),
    // The returned count is clamped to the cap in the view (none in this world).
    ("SetOfficers", "assert(A1.SetOfficers(0, 100.0) <= 100.0)", Fine),
    ("AddOfficers", "A1.AddOfficers(0, 1.0)", Fine),
    ("SetTeamCrew", "A1.SetTeamCrew(0, 10.0)", Fine),
    ("AddCrew", "assert(A1.AddCrew(0, 5.0) <= 5.0)", Fine),
    ("GetCrew", "A1.GetCrew(a)", Either),
    ("SetCrew", "A1.SetCrew(a, 5.0)", Fine),
    ("GetDilithiumOnFreighter", "A1.GetDilithiumOnFreighter(a)", Fail),
    ("StartCredits", "A1.StartCredits()", Fine),
    ("EndCredits", "A1.EndCredits()", Fine),
    // Teams, players and relations.
    ("SetRelation", "A1.SetRelation(0, 1, 0)", Fine),
    ("DisableAllAITeams", "A1.DisableAllAITeams()", Fine),
    ("EnableAllAITeams", "A1.EnableAllAITeams()", Fine),
    ("GetNearestEnemy", "assert(A1.GetNearestEnemy(a, 500) == b)", Fine),
    ("GetRealTeam", "assert(A1.GetRealTeam(a) == 0)", Fine),
    ("SetRealTeam", "A1.SetRealTeam(a, 1)", Fine),
    ("SetPerceivedTeam", "A1.SetPerceivedTeam(a, 1)", Fine),
    ("SetRace", "A1.SetRace(a, 'borg')", Fine),
    ("DisableTeam", "A1.DisableTeam(1)", Fine),
    ("EnableTeam", "A1.EnableTeam(1)", Fine),
    ("SetIsAITeam", "A1.SetIsAITeam(1, true)", Fine),
    ("IsVisibleToTeam", "A1.IsVisibleToTeam(a, 0)", Fine),
    ("GetPercievedTeamOfFlag", "assert(A1.GetPercievedTeamOfFlag(f) == 0)", Fine),
    ("GetRealTeamOfFlag", "assert(A1.GetRealTeamOfFlag(f) == 0)", Fine),
    ("SetWormholeTeam", "A1.SetWormholeTeam(a, 1)", Fine),
    // Objects.
    ("GetHandle", "assert(A1.GetHandle('hero') == a)", Fine),
    ("GridVisible", "A1.GridVisible(false)", Fine),
    ("SetPosition", "A1.SetPosition(a, 'P')", Fine),
    ("Load_Rules", "assert(A1.Load_Rules('r') == 0)", Fine),
    ("Load_Action_Scripts", "assert(A1.Load_Action_Scripts('s') == 0)", Fine),
    ("GroupNew", "assert(A1.GroupNew('g') == 1)", Fine),
    ("GroupDelete", "A1.GroupDelete(A1.GroupNew('g'))", Fine),
    ("Add_Team_To_Group", "A1.Add_Team_To_Group(1, A1.GroupNew('g'))", Fine),
    ("strength_team_1", "A1.strength_team_1()", Fail),
    ("strength_team_2", "A1.strength_team_2()", Fail),
    ("hot_float_value", "A1.hot_float_value()", Fail),
    ("SetSpecialForcesFlag", "A1.SetSpecialForcesFlag(a, true)", Fine),
    ("RemoveObject", "A1.RemoveObject(a)", Fine),
    ("CountUnits", "assert(A1.CountUnits(0, nil) == 2)", Fine),
    ("GetLocation", "assert(A1.GetLocation(b).x == 100)", Fine),
    ("GetLocation", "assert(A1.GetLocation('P', 2).x == 30)", Fine),
    ("GetDistance", "assert(A1.GetDistance(a, b) == 100)", Fine),
    ("GetDistance", "assert(A1.GetDistance(a, 'P', 1) == 20)", Fine),
    ("CraftCannotDie", "A1.CraftCannotDie(a, true)", Fine),
    ("SetShieldPercent", "A1.SetShieldPercent(a, 0.5)", Fine),
    ("GetShieldPercent", "A1.GetShieldPercent(a)", Fine),
    ("CountUnitsNearPoint", "assert(A1.CountUnitsNearPoint('P', 50, 0, nil, 0) == 1)", Fine),
    ("DestroyCritical", "A1.DestroyCritical(a)", Fine),
    ("IsAlive", "assert(A1.IsAlive(a))", Fine),
    ("CountUnitsNearObject", "assert(A1.CountUnitsNearObject(a, 1000, 1, nil) == 1)", Fine),
    ("GetNearestStarbase", "A1.GetNearestStarbase(a, 0)", Fine),
    ("Cloak", "A1.Cloak(a)", Fine),
    ("DisableWeapons", "A1.DisableWeapons(a, true)", Fine),
    ("ImmediateCloak", "A1.ImmediateCloak(a)", Fine),
    ("RecruitSpecialForces", "local ok, out = A1.RecruitSpecialForces('x', 0, 7); assert(ok == false and out == 7)", Fine),
    ("FadeRemove", "A1.FadeRemove()", Fine),
    ("RemoveAllCraft", "A1.RemoveAllCraft(true, -1)", Fine),
    ("DestroyShieldGenerator", "A1.DestroyShieldGenerator(a)", Fine),
    ("SetSpecialEnergy", "A1.SetSpecialEnergy(a, 10)", Either),
    ("GetRealShieldPercent", "A1.GetRealShieldPercent(a)", Fine),
    ("CreateSpecialFlag", "assert(A1.CreateSpecialFlag(b, 'Other') == 2)", Fine),
    ("GetLocationOfFlag", "assert(A1.GetLocationOfFlag(f) == a)", Fine),
    ("DestroyLifeSupport", "A1.DestroyLifeSupport(a)", Fine),
    ("DestroyWeapons", "A1.DestroyWeapons(a)", Fine),
    ("SetMaxShields", "A1.SetMaxShields(a, 1200)", Fine),
    ("Exists", "assert(A1.Exists(a))", Fine),
    ("CountVisibleUnits", "A1.CountVisibleUnits(0, 1, true, nil)", Fine),
    ("CountVisibleUnitsNearObject", "A1.CountVisibleUnitsNearObject(a, 1000, 1, true, nil)", Fine),
    ("GetNearestObjectNearMe", "assert(A1.GetNearestObjectNearMe(a, 1000, 1, nil, -1, false) == b)", Fine),
    ("ReplaceObject", "assert(A1.ReplaceObject(a, 'fx_frigate') == 0)", Fine),
    ("DestroyEngines", "A1.DestroyEngines(a)", Fine),
    ("DisableShieldGenerator", "A1.DisableShieldGenerator(a, true)", Fine),
    ("RemoveAllObjects", "A1.RemoveAllObjects()", Fine),
    ("RestrictSpecialFlag", "A1.RestrictSpecialFlag(f, 1)", Fine),
    ("RestrictSpecialFlag", "A1.RestrictSpecialFlag(f, 2, 0)", Fine),
    ("GetNearestObjectNearPosition", "assert(A1.GetNearestObjectNearPosition(Vector(95, 0, 0), 20, 1, nil, -1, false, 0) == b)", Fine),
    ("GetNearestTranswarpGate", "A1.GetNearestTranswarpGate(a)", Fail),
    ("AllowMultipleSpecialWeaponFire", "A1.AllowMultipleSpecialWeaponFire(a, true)", Fine),
    ("DestroySensors", "A1.DestroySensors(a)", Fine),
    ("RemoveBillboardEffect", "A1.RemoveBillboardEffect(1)", Fine),
    // Other.
    ("GetTime", "assert(A1.GetTime() >= 0)", Fine),
    ("FailMission", "A1.FailMission(1.0, 'lost')", Fine),
    ("SucceedMission", "A1.SucceedMission(1.0, 'won')", Fine),
    ("SucceedMission", "A1.SucceedMission(1.0, nil)", Fine),
    ("FailMission", "A1.FailMission(1.0, nil)", Fine),
    ("IsCompromised", "A1.IsCompromised(a, 1)", Fine),
    ("ObstacleAvoidence", "A1.ObstacleAvoidence(true)", Fine),
    ("PointAt", "A1.PointAt(a, b)", Fine),
    ("PointAt", "A1.PointAt(a, 'P')", Fine),
    ("IsValid", "assert(A1.IsValid(a) and not A1.IsValid(0))", Fine),
    ("UnpauseSimulation", "A1.UnpauseSimulation()", Fine),
    ("PauseSimulation", "A1.PauseSimulation()", Fine),
    ("Decloak", "A1.Decloak(a)", Fine),
    ("LinkWormHoles", "A1.LinkWormHoles(a, b)", Fine),
    ("SetRedAlert", "A1.SetRedAlert(a)", Fine),
    ("EnableLight", "A1.EnableLight(a, true)", Fine),
    ("SetGreenAlert", "A1.SetGreenAlert(a)", Fine),
    ("IsExploding", "assert(A1.IsExploding(a) == false and A1.IsExploding(0) == false)", Fine),
    ("Starfield_Load_Background_Geometry", "A1.Starfield_Load_Background_Geometry('bg_a.sod')", Fine),
    ("DisableEngines", "A1.DisableEngines(a, true)", Fine),
    ("TransferFlag", "A1.TransferFlag(1, 2)", Fine),
    ("DoesNotExist", "assert(A1.DoesNotExist(0))", Fine),
    ("SetYellowAlert", "A1.SetYellowAlert(a)", Fine),
    ("IsDerelict", "A1.IsDerelict(a)", Fail),
    ("IsThereARomulanSpyOnShip", "A1.IsThereARomulanSpyOnShip(a)", Fail),
    ("GetTranswarpExitWormhole", "A1.GetTranswarpExitWormhole(a)", Fail),
    ("AddTransporterEffect", "A1.AddTransporterEffect(a)", Fine),
    ("ClearRestrictions", "A1.ClearRestrictions(f)", Fine),
    ("EnableWormholeTravel", "A1.EnableWormholeTravel(a, true)", Fine),
];

#[test]
fn every_imported_function_has_a_valid_call() {
    let s = scene();
    for (name, script, expect) in CASES {
        let rt = runtime(&s);
        let result = rt.eval::<()>(script);
        match (expect, &result) {
            (Fine, Ok(())) | (Fail, Err(_)) | (Either, _) => {}
            (Fine, Err(e)) => panic!("{name}: `{script}` failed: {e}"),
            (Fail, Ok(())) => panic!("{name}: `{script}` should report a missing seam"),
        }
        if let (Fail | Either, Err(e)) = (expect, &result) {
            assert!(e.to_string().contains(&format!("A1.{name}")), "{name}: error must name the A1 call: {e}");
        }
    }
}

#[test]
fn the_table_and_the_cases_name_the_same_functions() {
    let s = scene();
    let rt = runtime(&s);
    let bound: BTreeSet<String> = rt
        .eval::<Vec<String>>("local t = {}; for k, v in pairs(A1) do if type(v) == 'function' then t[#t + 1] = k end end; return t")
        .unwrap()
        .into_iter()
        // Host-side completion reports, tested below, are not DLL calls.
        .filter(|k| k != "MovieDone" && k != "AudioMessageDone" && k != "HostPending")
        .collect();
    let cased: BTreeSet<String> = CASES.iter().map(|c| c.0.to_owned()).collect();
    assert_eq!(bound, cased);
}

#[test]
fn malformed_calls_are_errors_that_queue_nothing() {
    let s = scene();
    for script in [
        "A1.StopMovie(1)",
        "A1.ShowBridgeScene(1)",
        "A1.PreLoadBridgeScene(9)",
        "A1.PreLoadBridgeScene('a')",
        "A1.PlayBridgeMovie('x')",
        "A1.PlayBridgeMovie(1, 1)",
        "A1.RemoveObject()",
        "A1.RemoveObject(a, b)",
        "A1.RemoveObject(1.5)",
        "A1.RemoveObject(-1)",
        "A1.SetImpulseSpeed(a, 0/0)",
        "A1.SetImpulseSpeed(a, -1)",
        "A1.Goto(a, 0, 1)",
        "A1.ShowTitle(0, 0, 200, 70, 'T')",
        "A1.RemoveTitle(9, true)",
        "A1.SetRelation(0, 1, 7)",
        "A1.SetDilithium(9, 1)",
        "A1.BuildObject('nope', 0, 'Q', 0)",
        "A1.BuildObject('fx_cruiser', 0, 'Missing', 0)",
        "A1.CineractiveShot(a, 0)",
        "A1.FadeIn(1, 0)",
        "A1.SetPosition(0, 'P')",
    ] {
        fails(&s, script);
    }
}

#[test]
fn errors_from_reused_a2_calls_name_the_a1_table() {
    let s = scene();
    let message = fails(&s, "A1.CraftCannotDie(a, 1)");
    assert!(message.contains("A1.CraftCannotDie") && !message.contains("A2."), "{message}");
    let message = fails(&s, "A1.RemoveObject(999)");
    assert!(message.contains("A1.RemoveObject"), "{message}");
}

#[test]
fn bridge_scene_calls_are_presentation_events_in_call_order() {
    let s = scene();
    let (mut rt, commands) = run(
        &s,
        "A1.PreLoadBridgeScene(1); A1.SetBridgeSceneRace(4); A1.ShowBridgeScene(true); A1.ShowBridgeOnScreen(true); \
         A1.PlayBridgeMovie('intro_b.bik', 1); playing = A1.IsMoviePlaying()",
    );
    assert_eq!(
        presented(&commands),
        vec![
            call("PreLoadBridgeScene", vec![Arg::Int(1), Arg::Text("federation".into())]),
            call("SetBridgeSceneRace", vec![Arg::Int(4), Arg::Text("borg".into())]),
            call("ShowBridgeScene", vec![Arg::Bool(true)]),
            call("ShowBridgeOnScreen", vec![Arg::Bool(true)]),
            Presentation::Movie { name: "intro_b.bik".into(), in_panel: true, skippable: true },
        ]
    );
    assert!(rt.eval::<bool>("return playing").unwrap());
    // The host reports completion; a stop request also ends it.
    rt.eval::<()>("A1.MovieDone()").unwrap();
    assert!(!rt.eval::<bool>("return A1.IsMoviePlaying()").unwrap());
    rt.eval::<()>("A1.PlayBridgeMovie('talk_a.bik', -1); A1.StopMovie()").unwrap();
    assert!(!rt.eval::<bool>("return A1.IsMoviePlaying()").unwrap());
    assert_eq!(
        presented(&rt.take_commands()),
        vec![
            Presentation::Movie { name: "talk_a.bik".into(), in_panel: true, skippable: true },
            Presentation::StopMovie
        ]
    );
    let (_, commands) = run(&s, "A1.SetBridgeSceneRace(0)");
    assert_eq!(presented(&commands), vec![call("SetBridgeSceneRace", vec![Arg::Int(0)])]);
}

#[test]
fn cineractive_calls_drive_the_shared_mode_and_retain_shots() {
    let s = scene();
    let (rt, commands) = run(&s, "A1.CineractiveStart(); A1.CineractiveShot('P', 0.5, b); A1.FadeOut(2, 0, 0); A1.CineractiveEnd()");
    let events = presented(&commands);
    // The skip key always works on an A1 cineractive (static): a start is skippable.
    assert_eq!(events[0], Presentation::Cineractive { on: true, skippable: true });
    assert_eq!(
        events[1],
        call("CineractiveShot", vec![Arg::Text("P".into()), Arg::Num(Fixed::from_f64(0.5)), Arg::Entity(s.ids[1])])
    );
    assert_eq!(
        events[2],
        call("FadeOut", vec![Arg::Num(Fixed::from_int(2)), Arg::Num(Fixed::ZERO), Arg::Num(Fixed::ZERO)])
    );
    assert_eq!(events[3], Presentation::Fade { out: true, seconds: Fixed::from_int(2), rgb: [0, 0, 0] });
    assert_eq!(events[4], Presentation::Cineractive { on: false, skippable: true });
    assert!(rt.unsupported().is_empty());
}

/// `FadeOut(seconds, delay, _)`: a delayed fade is presented when it starts (LUA-17).
#[test]
fn a_delayed_fade_is_presented_when_it_starts() {
    use sta_sim::setup::Controller;
    let s = scene();
    let (mut rt, commands) = run(&s, "A1.FadeOut(2, 25, 0)");
    let now = presented(&commands);
    assert_eq!(now.len(), 1, "only the retained call at once: {now:?}");
    let mut view = s.view.clone();
    let base = view.tick + 25 * 30;
    let fade = Presentation::Fade { out: true, seconds: Fixed::from_int(2), rgb: [0, 0, 0] };
    let mut at = |tick: u64, rt: &mut MissionRuntime| {
        view.tick = tick;
        let mut out = Vec::new();
        rt.think(&view, &[], &mut out);
        presented(&out)
    };
    assert!(!at(base - 1, &mut rt).contains(&fade));
    assert_eq!(at(base, &mut rt), vec![fade.clone()]);
    assert!(!at(base + 1, &mut rt).contains(&fade));
}

/// An authored opening: a voice line followed by a delayed `FadeOut`, then the cineractive ends.
/// A delayed fade is dropped with the cineractive, so it cannot black out the game after the
/// hand-over; the voice line's done flag is the host's to report.
#[test]
fn a_delayed_fade_does_not_outlive_its_cineractive() {
    use sta_sim::setup::Controller;
    let s = scene();
    let (mut rt, commands) = run(
        &s,
        "A1.CineractiveStart(); A1.FadeIn(3, 0, 0); log = A1.AudioMessage('log.wav'); A1.FadeOut(4, 40, 0); \
         function done() return A1.IsAudioMessageDone(log) end",
    );
    let first = presented(&commands);
    assert!(first.contains(&Presentation::Sound { id: 1, name: "log.wav".into(), at: None }), "{first:?}");
    assert!(!rt.eval::<bool>("return done()").unwrap(), "the line is playing until the host says it is done");
    let mut view = s.view.clone();
    let tick0 = view.tick;
    let mut at = |secs: u64, rt: &mut MissionRuntime| {
        view.tick = tick0 + secs * 30;
        let mut out = Vec::new();
        rt.think(&view, &[], &mut out);
        presented(&out)
    };
    assert!(at(20, &mut rt).is_empty() && !rt.eval::<bool>("return done()").unwrap());
    // The host reports the clip's end; the script's condition turns true and it ends the opening.
    rt.eval::<()>("A1.AudioMessageDone(1)").unwrap();
    assert!(rt.eval::<bool>("return done()").unwrap());
    rt.eval::<()>("A1.CineractiveEnd()").unwrap();
    // The fade scheduled for 40 s is gone: nothing darkens the game at 40 s or later.
    assert!(at(41, &mut rt).iter().all(|p| !matches!(p, Presentation::Fade { .. })));
    assert!(at(60, &mut rt).iter().all(|p| !matches!(p, Presentation::Fade { .. })));
}

#[test]
fn titles_keep_their_rectangle_and_ids_are_per_mission() {
    let s = scene();
    let (_, commands) = run(&s, "t = A1.ShowTitle(7, 8, 9, 10, 'Title A', true); u = A1.ShowSubtitle(5, 10, 300, 20, 'Sub A', true); A1.RemoveTitle(t, true); A1.RemoveSubtitle(u, true)");
    assert_eq!(
        presented(&commands),
        vec![
            Presentation::Title { id: 1, text: "Title A".into(), subtitle: false, seconds: None },
            call("ShowTitle", vec![Arg::Int(1), Arg::Int(7), Arg::Int(8), Arg::Int(9), Arg::Int(10), Arg::Bool(true)]),
            Presentation::Title { id: 2, text: "Sub A".into(), subtitle: true, seconds: None },
            call("ShowSubtitle", vec![Arg::Int(2), Arg::Int(5), Arg::Int(10), Arg::Int(300), Arg::Int(20), Arg::Bool(true)]),
            call("RemoveTitleFade", vec![Arg::Int(1)]),
            call("RemoveTitleFade", vec![Arg::Int(2)]),
        ]
    );
    // The plain form goes at once.
    let (_, commands) = run(&s, "t = A1.ShowTitle(7, 8, 9, 10, 'Title A', true); A1.RemoveTitle(t, false)");
    assert_eq!(presented(&commands).last(), Some(&Presentation::RemoveTitle { id: 1 }));
}

#[test]
fn audio_messages_are_done_when_stopped_or_reported() {
    let s = scene();
    let (rt, commands) = run(&s, "m = A1.AudioMessage('briefing.wav'); n = A1.AudioMessage('b2.wav'); A1.StopAudioMessage(n)");
    assert_eq!(
        presented(&commands),
        vec![
            Presentation::Sound { id: 1, name: "briefing.wav".into(), at: None },
            Presentation::Sound { id: 2, name: "b2.wav".into(), at: None },
            Presentation::StopSound { id: 2 },
        ]
    );
    assert!(!rt.eval::<bool>("return A1.IsAudioMessageDone(m)").unwrap());
    assert!(rt.eval::<bool>("return A1.IsAudioMessageDone(n)").unwrap());
    let pending = "local o = {} for i, p in ipairs(A1.HostPending()) do o[i] = p.kind .. ':' .. p.id .. ':' .. p.name end return o";
    assert_eq!(rt.eval::<Vec<String>>(pending).unwrap(), vec!["sound:1:briefing.wav".to_owned()]);
    rt.eval::<()>("A1.AudioMessageDone(m)").unwrap();
    assert!(rt.eval::<bool>("return A1.IsAudioMessageDone(m)").unwrap());
    assert!(rt.eval::<Vec<String>>(pending).unwrap().is_empty());
    rt.eval::<()>("A1.PlayBridgeMovie('m.bik', 0)").unwrap();
    assert_eq!(rt.eval::<Vec<String>>(pending).unwrap(), vec!["movie:0:m.bik".to_owned()]);
    rt.eval::<()>("A1.MovieDone()").unwrap();
    assert!(rt.eval::<Vec<String>>(pending).unwrap().is_empty());
    // a handle still at its -1 start value, or an id not playing: the engine finds no such sound
    let (_, commands) = run(&s, "A1.StopAudioMessage(-1); A1.StopAudioMessage(77)");
    assert!(commands.is_empty(), "{commands:?}");
}

#[test]
fn object_calls_reuse_the_a2_commands() {
    let s = scene();
    let [a, b, ..] = s.ids;
    let (_, commands) = run(&s, "A1.RemoveObject(b); A1.CraftCannotDie(a, true); A1.DisableEngines(a, true); A1.Cloak(a)");
    let kinds: Vec<_> = commands.iter().map(|c| c.kind.clone()).collect();
    assert_eq!(
        kinds,
        vec![
            CommandKind::Edit(Edit::Kill(b)),
            CommandKind::Edit(Edit::Immortal(a, true)),
            CommandKind::Edit(Edit::Engines(a, false)),
            CommandKind::Order(Order::Cloak { units: vec![a], on: true }),
        ]
    );
}

#[test]
fn switches_alerts_speed_and_resources_become_edits() {
    let s = scene();
    let a = s.ids[0];
    let (_, commands) = run(
        &s,
        "A1.SetImpulseSpeed(a, 20); A1.SetRedAlert(a); A1.SetYellowAlert(a); A1.SetGreenAlert(a); \
         A1.DisableShieldGenerator(a, true); A1.SetDilithium(1, 750)",
    );
    let kinds: Vec<_> = commands.iter().map(|c| c.kind.clone()).collect();
    assert_eq!(
        kinds,
        vec![
            CommandKind::Edit(Edit::MaxSpeed(a, Some(Fixed::from_int(20)))),
            CommandKind::Edit(Edit::Alert(a, AlertState::Red)),
            CommandKind::Edit(Edit::Alert(a, AlertState::Yellow)),
            CommandKind::Edit(Edit::Alert(a, AlertState::Green)),
            CommandKind::Edit(Edit::Subsystem(a, Subsystem::Shields, false)),
            CommandKind::Edit(Edit::Cargo(PlayerId(1), 0, Fixed::from_int(750))),
        ]
    );
}

#[test]
fn orders_are_sent_as_the_owner() {
    let s = scene();
    let [a, b, st, _] = s.ids;
    let (_, commands) = run(&s, "A1.Goto(a, 'P', 1); A1.Goto(a, b, 1); A1.Stop(a, 1); A1.Attack(a, b, 1); A1.Mine(a, b); A1.Build(s, 'fx_frigate')");
    let to = |x: f32| Vec3::from_f32([x, 0.0, 0.0]);
    let orders: Vec<_> = commands
        .iter()
        .map(|c| match &c.kind {
            CommandKind::Order(o) => (c.player, o.clone()),
            other => panic!("{other:?}"),
        })
        .collect();
    let frigate = s.db.key("fx_frigate").unwrap();
    assert_eq!(
        orders,
        vec![
            (PlayerId(0), Order::Move { units: vec![a], to: to(10.0), queue: false }),
            (PlayerId(0), Order::Move { units: vec![a], to: to(20.0), queue: true }),
            (PlayerId(0), Order::Move { units: vec![a], to: to(30.0), queue: true }),
            (PlayerId(0), Order::Move { units: vec![a], to: to(100.0), queue: false }),
            (PlayerId(0), Order::Stop { units: vec![a] }),
            (PlayerId(0), Order::Attack { units: vec![a], target: b }),
            (PlayerId(0), Order::Mine { units: vec![a], node: b }),
            (PlayerId(0), Order::Build { builder: st, what: frigate, at: None }),
        ]
    );
}

#[test]
fn stop_all_ships_stops_the_team_and_filters_by_odf() {
    let s = scene();
    let [a, _, st, _] = s.ids;
    let (_, commands) = run(&s, "A1.StopAllShips(0, nil, true, true)");
    let stopped: BTreeSet<EntityId> = commands
        .iter()
        .flat_map(|c| match &c.kind {
            CommandKind::Order(Order::Stop { units }) => units.clone(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(stopped, BTreeSet::from([a, st]));
    let (_, commands) = run(&s, "A1.StopAllShips(0, 'fx_station.odf', false, true)");
    assert_eq!(commands.len(), 1);
}

#[test]
fn queries_read_the_view() {
    let s = scene();
    let (rt, _) = run(&s, "");
    let r: (i64, i64, i64, i64, i64, bool) = rt
        .eval(
            "return A1.CountUnits(1, 'fx_cruiser'), A1.CountUnits(-1, nil), A1.CountUnitsNearObject(a, 400, -1, nil), \
             A1.GetNearestObjectNearMe(a, 400, 0, 'fx_station', -1, false), A1.GetNearestStarbase(b, 0), A1.GetShieldPercent(a) >= 0",
        )
        .unwrap();
    assert_eq!((r.0, r.1, r.2, r.3), (1, 4, 2, i64::from(s.ids[2].0)));
    // The fake station has no `is_starbase` flag, so the A2 station
    // classification (reused here) finds none.
    assert_eq!(r.4, 0);
    assert!(r.5);
    let distance: f64 = rt.eval("return A1.GetDistance(a, 'Q', 0)").unwrap();
    assert_eq!(distance, 500.0);
    assert_eq!(rt.eval::<i64>("return A1.GetHandle('missing')").unwrap(), 0);
}

#[test]
fn path_seam_is_a_documented_error_when_unfilled() {
    let s = scene();
    let message = fails(&s, "A1.paths = nil; A1.Goto(a, 'P', 1)");
    assert!(message.contains("A1.paths"), "{message}");
}

#[test]
fn absent_path_names_do_nothing_like_the_engines_null_lookup() {
    // LUA-18: Goto, Patrol and PointAt on a path the map does not have queue
    // nothing and are not counted; GetLocation answers a default vector.
    let s = scene();
    let (rt, commands) = run(&s, "A1.Goto(a, 'NOPE', 1); A1.Patrol(a, 'NOPE'); A1.PointAt(a, 'NOPE')");
    assert!(commands.is_empty(), "{commands:?}");
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
    let (rt, _) = run(&s, "");
    let v: (f64, f64, f64) = rt.eval("local p = A1.GetLocation('NOPE', 0); return p.x, p.y, p.z").unwrap();
    assert_eq!(v, (0.0, 0.0, 0.0));
    // a point beyond the path is the same default
    let v: f64 = rt.eval("return A1.GetLocation('P', 9).x").unwrap();
    assert_eq!(v, 0.0);
    let v: f64 = rt.eval("return A1.GetLocation('P', 1).x").unwrap();
    assert_eq!(v, 20.0);
    assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported());
}

#[test]
fn paths_are_the_maps_and_match_ignoring_case() {
    let s = scene();
    let (rt, _) = run(&s, "");
    assert!(rt.eval::<bool>("return rawequal(A1.paths, A2.paths)").unwrap());
    let (_, commands) = run(&s, "A1.Goto(a, 'q', 1)");
    assert_eq!(commands.len(), 1, "{commands:?}");
    let message = fails(&s, "A1.paths.Ab = { {x=1,y=0,z=0} }; A1.paths.aB = A1.paths.Ab; A1.Goto(a, 'AB', 1)");
    assert!(message.contains("ambiguous"), "{message}");
    // an exact name wins over its case variants
    let (_, commands) = run(&s, "A1.paths.Ab = { {x=1,y=0,z=0} }; A1.paths.aB = A1.paths.Ab; A1.Goto(a, 'Ab', 1)");
    assert_eq!(commands.len(), 1, "{commands:?}");
}

#[test]
fn set_relation_sends_a_one_directional_edit_with_the_engines_codes() {
    use sta_sim::command::{CommandKind, Edit};
    let s = scene();
    let (rt, commands) = run(&s, "A1.SetRelation(1, 2, 2); A1.SetRelation(2, 1, 1); A1.SetRelation(1, 2, 0)");
    let edits: Vec<_> = commands
        .iter()
        .filter_map(|c| match &c.kind {
            CommandKind::Edit(Edit::SetRelation { from, to, relation }) => Some((from.0, to.0, *relation)),
            _ => None,
        })
        .collect();
    // Codes are the engine's: 2 Ally, 1 Neutral, 0 Enemy.
    assert_eq!(edits.len(), 3, "{commands:?}");
    assert_eq!((edits[0].2, edits[1].2, edits[2].2), (Relation::Ally, Relation::Neutral, Relation::Enemy));
    assert_eq!((edits[0].0, edits[0].1), (edits[2].0, edits[2].1));
    assert_eq!((edits[1].0, edits[1].1), (edits[0].1, edits[0].0), "the reverse call is its own edit");
    assert_eq!(rt.relation(PlayerId(edits[1].0), PlayerId(edits[1].1)), Relation::Neutral);
}

#[test]
fn seam_gap_writes_are_retained_and_counted_without_other_effect() {
    let s = scene();
    let (rt, commands) = run(&s, "A1.SetWarpSpeed(a, 30)");
    assert_eq!(presented(&commands), vec![call("SetWarpSpeed", vec![Arg::Entity(s.ids[0]), Arg::Num(Fixed::from_int(30))])]);
    let unsupported: BTreeSet<String> = rt.unsupported().into_iter().map(|(k, _)| k).collect();
    assert!(unsupported.iter().any(|k| k.starts_with("A1.SetWarpSpeed")));
    // A dead target is an error, not a silent request.
    fails(&s, "A1.SetShieldPercent(999999, 0.5)");
}

#[test]
fn set_position_and_point_at_queue_typed_edits_and_are_not_counted() {
    let s = scene();
    let (rt, commands) = run(&s, "A1.SetPosition(a, 'P'); A1.PointAt(a, b); A1.PointAt(a, 'Q')");
    let edits: Vec<_> = commands.iter().map(|c| (c.player, c.kind.clone())).collect();
    let edit = |e| (PlayerId::SCRIPT, CommandKind::Edit(e));
    assert_eq!(
        edits,
        vec![
            edit(Edit::Position(s.ids[0], Vec3::from_f32([10.0, 0.0, 0.0]))),
            // From where the unit now is (10, after its queued SetPosition).
            edit(Edit::Heading(s.ids[0], (Fixed::from_int(90), Fixed::ZERO))),
            edit(Edit::Heading(s.ids[0], (Fixed::from_int(490), Fixed::ZERO))),
        ]
    );
    assert!(rt.unsupported().iter().all(|(k, _)| !k.contains("SetPosition") && !k.contains("PointAt")));
    // A target at the unit's own place has no direction: counted, nothing queued.
    let (rt, commands) = run(&s, "A1.PointAt(a, a)");
    assert!(commands.is_empty());
    assert!(rt.unsupported().iter().any(|(k, _)| k.starts_with("A1.PointAt")));
    // A path the host did not fill is an error (host data missing).
    assert!(fails(&s, "A1.SetPosition(a, 'Nope')").contains("Nope"));
}

#[test]
fn shield_writes_queue_typed_edits_and_are_not_counted() {
    let s = scene();
    let (rt, commands) = run(&s, "A1.SetShieldPercent(a, 0.5); A1.SetMaxShields(a, 900)");
    let edits: Vec<_> = commands.iter().map(|c| c.kind.clone()).collect();
    assert_eq!(
        edits,
        vec![
            CommandKind::Edit(Edit::ShieldPercent(s.ids[0], Fixed::from_f64(0.5))),
            CommandKind::Edit(Edit::MaxShields(s.ids[0], Fixed::from_int(900))),
        ]
    );
    assert!(rt.unsupported().iter().all(|(k, _)| !k.contains("Shield")));
}

#[test]
fn seam_gap_queries_name_what_is_missing() {
    let s = scene();
    let message = fails(&s, "A1.InStopMode(a)");
    assert!(message.contains("not available") && message.contains("order state"), "{message}");
    let rt = runtime(&s);
    let _ = rt.eval::<()>("A1.IsDerelict(a)");
    assert!(rt.unsupported().iter().any(|(k, _)| k.starts_with("A1.IsDerelict")));
}

#[test]
fn mission_outcome_is_retained_and_finishes_the_mission() {
    let s = scene();
    let (rt, commands) = run(&s, "A1.SucceedMission(3.0, 'victory')");
    assert!(rt.finished());
    assert_eq!(
        presented(&commands),
        vec![call("SucceedMission", vec![Arg::Num(Fixed::from_int(3)), Arg::Text("victory".into())])]
    );
    let (rt, _) = run(&s, "A1.FailMission(0, 'lost')");
    assert!(rt.finished());
}

#[test]
fn special_flags_follow_their_unit() {
    let s = scene();
    let (rt, _) = run(&s, "g = A1.CreateSpecialFlag(b, 'Hero2'); A1.RestrictSpecialFlag(g, 1); A1.SetPerceivedTeam(b, 0)");
    assert_eq!(rt.eval::<i64>("return A1.GetLocationOfFlag(g)").unwrap(), i64::from(s.ids[1].0));
    assert_eq!(rt.eval::<i64>("return A1.GetRealTeamOfFlag(g)").unwrap(), 1);
    rt.eval::<()>("A1.ClearRestrictions(g)").unwrap();
    assert!(rt.eval::<()>("A1.ClearRestrictions(99)").is_err());
}

/// BuildObject returns a pending handle that every A1 call resolves once the
/// host has delivered the spawn receipt.
#[test]
fn build_object_handles_resolve_after_the_receipt() {
    let s = scene();
    let mut sim = Sim::from_setup(&game(&s.db));
    let cfg = MissionConfig { content: Some(s.db.clone()), view: Some((*sim.view()).clone()), ..MissionConfig::default() };
    let mut rt = MissionRuntime::from_source_with("build.lua", "A1.paths.Q = { {x=40,y=0,z=0} }; h = A1.BuildObject('fx_cruiser', 1, 'Q', 0)", 3, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let token: u32 = rt.eval("return h").unwrap();
    assert!(token >= super::TOKEN_BASE);
    // Before the receipt the object does not exist: queries cannot use it, a void order
    // is held (LUA-17: the original's handle is usable at once).
    assert!(!rt.eval::<bool>("return A1.Exists(h)").unwrap());
    assert!(rt.eval::<()>("A1.GetLocation(h)").is_err());
    rt.eval::<()>("A1.Stop(h, 1)").expect("held");
    assert!(!rt.eval::<bool>("return A1.IsValid(h)").unwrap());
    let first = rt.take_commands();
    assert!(matches!(&first[0].kind, CommandKind::Edit(Edit::SpawnRequest { player, .. }) if *player == PlayerId(1)));
    let out = sim.step(&first);
    let id = out
        .events
        .iter()
        .find_map(|e| match e {
            SimEvent::ScriptSpawned { id, .. } => Some(*id),
            _ => None,
        })
        .expect("receipt");
    let mut later = Vec::new();
    use sta_sim::setup::Controller;
    rt.think(&out.view, &out.events, &mut later);
    // The first A1 call after the receipt replays the held Stop.
    assert!(rt.eval::<bool>("return A1.IsAlive(h) and A1.Exists(h)").unwrap());
    let held = rt.take_commands();
    assert_eq!(held.iter().filter(|c| c.kind == CommandKind::Order(Order::Stop { units: vec![id] })).count(), 1, "{held:?}");
    assert_eq!(rt.eval::<f64>("return A1.GetLocation(h).x").unwrap(), 40.0);
    rt.eval::<()>("A1.Stop(h, 1)").unwrap();
    let commands = rt.take_commands();
    assert_eq!(commands[0].kind, CommandKind::Order(Order::Stop { units: vec![id] }));
    assert_eq!(commands[0].player, PlayerId(1));
}

#[test]
fn dilithium_writes_in_one_tick_build_on_each_other() {
    let s = scene();
    let (rt, commands) = run(&s, "x = A1.SetDilithium(1, 100); y = A1.AddDilithium(1, 50); z = A1.AddDilithium(1, 25)");
    let amounts: Vec<Fixed> = commands
        .iter()
        .map(|c| match &c.kind {
            CommandKind::Edit(Edit::Cargo(PlayerId(1), 0, v)) => *v,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(amounts, vec![Fixed::from_int(100), Fixed::from_int(150), Fixed::from_int(175)]);
    assert_eq!(rt.eval::<(i64, i64, i64)>("return x, y, z").unwrap(), (100, 150, 175));
}

#[test]
fn remove_all_craft_filters_by_team() {
    let s = scene();
    let [_, b, _, far] = s.ids;
    let (_, commands) = run(&s, "A1.RemoveAllCraft(true, 1)");
    let killed: BTreeSet<EntityId> = commands
        .iter()
        .map(|c| match &c.kind {
            CommandKind::Edit(Edit::Kill(id)) => *id,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(killed, BTreeSet::from([b, far]));
}

#[test]
fn failed_spawn_handles_are_no_object_for_queries_and_errors_elsewhere() {
    let s = scene();
    // A sim without content rejects the spawn request.
    let mut sim = Sim::new(3);
    let cfg = MissionConfig { content: Some(s.db.clone()), view: Some((*sim.view()).clone()), ..MissionConfig::default() };
    let mut rt = MissionRuntime::from_source_with("fail.lua", "A1.paths.Q = { {x=1,y=0,z=0} }; h = A1.BuildObject('fx_cruiser', 0, 'Q', 0)", 3, cfg);
    let first = rt.take_commands();
    let out = sim.step(&first);
    let mut later = Vec::new();
    use sta_sim::setup::Controller;
    rt.think(&out.view, &out.events, &mut later);
    assert!(!rt.eval::<bool>("return A1.Exists(h) or A1.IsAlive(h)").unwrap());
    assert!(rt.eval::<bool>("return A1.DoesNotExist(h)").unwrap());
    let err = rt.eval::<()>("A1.RemoveObject(h)").expect_err("failed spawn").to_string();
    assert!(err.contains("A1.RemoveObject"), "{err}");
}

#[test]
fn immediate_cloak_errors_name_their_own_call() {
    let s = scene();
    let message = fails(&s, "A1.ImmediateCloak(999999)");
    assert!(message.contains("A1.ImmediateCloak") && !message.contains("A1.Cloak"), "{message}");
}

/// LUA-17: a held call keeps its place: later orders and the end of a cineractive wait
/// behind it, and all are replayed in call order after the receipt.
#[test]
fn held_calls_keep_the_script_order() {
    use sta_sim::setup::Controller;
    let s = scene();
    let mut sim = Sim::from_setup(&game(&s.db));
    let cfg = MissionConfig { content: Some(s.db.clone()), view: Some((*sim.view()).clone()), ..MissionConfig::default() };
    let src = format!(
        "A1.paths.Q = {{ {{x=40,y=0,z=0}} }}; A1.paths.R = {{ {{x=90,y=0,z=0}} }}; \
         A1.CineractiveStart(); h = A1.BuildObject('fx_cruiser', 1, 'Q', 0); \
         A1.Attack({a}, h, 1); A1.Goto({a}, 'R', 1); A1.CineractiveEnd()",
        a = s.ids[0].0
    );
    let mut rt = MissionRuntime::from_source_with("order.lua", &src, 3, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let first = rt.take_commands();
    assert!(!first.iter().any(|c| matches!(c.kind, CommandKind::Order(_))), "nothing ordered before the receipt: {first:?}");
    assert!(!presented_any(&first).contains(&Presentation::Cineractive { on: false, skippable: true }), "the end waits too");
    let out = sim.step(&first);
    rt.think(&out.view, &out.events, &mut Vec::new());
    rt.eval::<bool>("return A1.Exists(h)").unwrap();
    let kinds: Vec<CommandKind> = rt.take_commands().into_iter().map(|c| c.kind).collect();
    let attack = kinds.iter().position(|k| matches!(k, CommandKind::Order(Order::Attack { .. }))).expect("attack");
    let goto = kinds.iter().position(|k| matches!(k, CommandKind::Order(Order::Move { .. }))).expect("goto");
    let end = kinds.iter().position(|k| matches!(k, CommandKind::Edit(Edit::Present(Presentation::Cineractive { on: false, .. })))).expect("end");
    assert!(attack < goto && goto < end, "{kinds:?}");
}

fn presented_any(commands: &[Command]) -> Vec<Presentation> {
    commands
        .iter()
        .filter_map(|c| match &c.kind {
            CommandKind::Edit(Edit::Present(p)) => Some(p.clone()),
            _ => None,
        })
        .collect()
}
