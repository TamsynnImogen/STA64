//! LUA-20: the AI switches reach the host's hub in call order, with the rules of
//! `sta_ai::mission`. Authored scenes on fake content, no game files.

use std::sync::Arc;

use sta_ai::{AiRequest, MissionAiHub};
use sta_content::fake;
use sta_core::{PlayerId, TeamId};
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};

use crate::api::{MissionConfig, PlayerNumbering};
use crate::runtime::MissionRuntime;

const P0: PlayerId = PlayerId(0);
const P1: PlayerId = PlayerId(1);

fn setup() -> GameSetup {
    let seat = |id: u8, control| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control };
    GameSetup {
        seed: 3,
        map: Arc::new(fake::two_bases()),
        content: Arc::new(fake::db()),
        players: vec![seat(0, Control::Human), seat(1, Control::Idle)],
        options: GameOptions::default(),
    }
}

fn runtime(numbering: PlayerNumbering) -> MissionRuntime {
    let cfg = MissionConfig {
        seat: Some(P0),
        content: Some(Arc::new(fake::db())),
        player_numbering: numbering,
        ..MissionConfig::default()
    };
    let rt = MissionRuntime::from_source_with("t.lua", "", 1, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt
}

fn a2(script: &str) -> MissionRuntime {
    let rt = runtime(PlayerNumbering::Slots);
    rt.eval::<()>(script).unwrap_or_else(|e| panic!("{script}: {e}"));
    rt
}

fn a1(script: &str) -> MissionRuntime {
    let rt = runtime(PlayerNumbering::ArmadaTeams);
    rt.eval::<()>(script).unwrap_or_else(|e| panic!("{script}: {e}"));
    rt
}

#[test]
fn a2_calls_queue_one_request_each_in_call_order_and_count_nothing() {
    let rt = a2("A2.DisableAllAITeams(); A2.AI_Team_Load_AIP(1, 'x.aip'); A2.EnableTeam(1); A2.DisableTeam(1);
                 A2.SetIsAITeam(1, true); A2.ResetStrategyCycle(); A2.EnableAllAITeams()");
    // Until a host takes them they are counted, each under its own name (a load as the plan load it is).
    assert_eq!(
        rt.unsupported(),
        [
            ("AI.DisableAllAITeams".to_owned(), 1),
            ("AI.DisableTeam".to_owned(), 1),
            ("AI.EnableAllAITeams".to_owned(), 1),
            ("AI.EnableTeam".to_owned(), 1),
            ("AI.ResetStrategyCycle".to_owned(), 1),
            ("AI.SetIsAITeam".to_owned(), 1),
            ("Player.loadAIP".to_owned(), 1),
        ]
    );
    assert_eq!(
        rt.take_ai_requests(),
        [
            AiRequest::DisableAll,
            AiRequest::Load(P1, "x.aip".into()),
            AiRequest::Enable(P1),
            AiRequest::Disable(P1),
            AiRequest::SetAi(P1, true),
            AiRequest::Replan,
            AiRequest::EnableAll,
        ]
    );
    assert!(rt.unsupported().is_empty(), "no AI switch is an unsupported call once the host has them");
}

#[test]
fn a2_argument_checks_stay_strict_and_an_unknown_team_is_the_originals_no_op() {
    let rt = runtime(PlayerNumbering::Slots);
    assert!(rt.eval::<()>("A2.EnableTeam(true)").is_err());
    assert!(rt.eval::<()>("A2.SetIsAITeam(1, 1)").is_err());
    assert!(rt.eval::<()>("A2.DisableAllAITeams(1)").is_err());
    rt.eval::<()>("A2.EnableTeam(300); A2.DisableTeam(-4)").unwrap();
    assert!(rt.take_ai_requests().is_empty(), "no range check in the original: nothing to switch");
}

#[test]
fn set_is_ai_on_the_human_seat_is_counted_unsupported_only_when_it_hands_it_to_the_ai() {
    let rt = a2("A2.SetIsAITeam(0, false); A2.SetIsAITeam(1, true)");
    assert_eq!(rt.take_ai_requests().len(), 2);
    assert!(rt.unsupported().is_empty(), "false on the human seat and true on a computer seat do nothing");
    let rt = a2("A2.SetIsAITeam(0, true)");
    let _ = rt.take_ai_requests();
    assert_eq!(rt.unsupported(), [("A2.SetIsAITeam: handing the human seat to the AI".to_owned(), 1)]);
}

#[test]
fn armada_one_calls_use_team_numbers_and_has_no_strategy_cycle() {
    let rt = a1("A1.DisableAllAITeams(); A1.EnableTeam(2); A1.DisableTeam(2); A1.SetIsAITeam(2, true); A1.EnableAllAITeams()");
    assert_eq!(
        rt.take_ai_requests(),
        [AiRequest::DisableAll, AiRequest::Enable(P1), AiRequest::Disable(P1), AiRequest::SetAi(P1, true), AiRequest::EnableAll]
    );
    assert!(rt.unsupported().is_empty());
    let rt = runtime(PlayerNumbering::ArmadaTeams);
    assert!(rt.eval::<bool>("return rawget(A1, 'ResetStrategyCycle') == nil").unwrap(), "A1 does not export it");
}

/// What the stock missions do, through the hub: the human seat (team 1) is never switched, a
/// computer seat follows the calls in order.
#[test]
fn the_armada_one_calls_that_do_nothing_do_nothing() {
    let s = setup();
    let hub = MissionAiHub::new(sta_ai::AipLibrary::new());
    let _ai = hub.controller(&s, P1);
    // finale4: EnableTeam / DisableTeam on the human's team. rom5: SetIsAITeam(computer, true).
    let rt = a1("A1.DisableTeam(1); A1.EnableTeam(1); A1.SetIsAITeam(2, true); A1.SetIsAITeam(1, false)");
    hub.submit_requests(rt.take_ai_requests());
    assert!(!hub.is_ai(P0) && !hub.is_enabled(P0), "the human seat stays without an AI team");
    assert!(hub.is_ai(P1) && !hub.is_enabled(P1), "the computer seat keeps its flags");
    assert!(hub.unresolved().is_empty());
    assert!(rt.unsupported().is_empty());
}

fn present_calls(rt: &mut MissionRuntime) -> Vec<(String, Vec<sta_sim::present::Arg>)> {
    use sta_sim::command::{CommandKind, Edit};
    rt.take_commands()
        .into_iter()
        .map(|c| match c.kind {
            CommandKind::Edit(Edit::Present(sta_sim::present::Presentation::Call { name, args })) => (name, args),
            other => panic!("a switch queued more than a retained call: {other:?}"),
        })
        .collect()
}

#[test]
fn reset_strategy_cycle_and_the_switches_change_no_other_state() {
    let mut rt = a2("A2.ResetStrategyCycle()");
    assert_eq!(rt.take_ai_requests(), [AiRequest::Replan]);
    assert_eq!(present_calls(&mut rt), [("A2.ResetStrategyCycle".to_owned(), vec![])]);
}

#[test]
fn both_games_retain_each_switch_as_a_presentation_call_so_recorded_streams_stay_alike() {
    use sta_sim::present::Arg;
    let mut rt = a2("A2.DisableAllAITeams(); A2.EnableAllAITeams(); A2.EnableTeam(1); A2.DisableTeam(1); A2.SetIsAITeam(1, true)");
    let a2_calls = present_calls(&mut rt);
    assert_eq!(
        a2_calls,
        [
            ("A2.DisableAllAITeams".to_owned(), vec![]),
            ("A2.EnableAllAITeams".to_owned(), vec![]),
            ("A2.EnableTeam".to_owned(), vec![Arg::Int(1)]),
            ("A2.DisableTeam".to_owned(), vec![Arg::Int(1)]),
            ("A2.SetIsAITeam".to_owned(), vec![Arg::Int(1), Arg::Bool(true)]),
        ]
    );
    // A1 pushed these before LUA-20 (through its gap path) and keeps doing so: `A1.<name>`, same arguments.
    let mut rt = a1("A1.DisableAllAITeams(); A1.EnableAllAITeams(); A1.EnableTeam(2); A1.DisableTeam(2); A1.SetIsAITeam(2, true)");
    let a1_calls = present_calls(&mut rt);
    let want = [
        ("A1.DisableAllAITeams".to_owned(), vec![]),
        ("A1.EnableAllAITeams".to_owned(), vec![]),
        ("A1.EnableTeam".to_owned(), vec![Arg::Int(2)]),
        ("A1.DisableTeam".to_owned(), vec![Arg::Int(2)]),
        ("A1.SetIsAITeam".to_owned(), vec![Arg::Int(2), Arg::Bool(true)]),
    ];
    assert_eq!(a1_calls, want);
    let _ = rt.take_ai_requests();
    assert!(rt.unsupported().is_empty());
}

#[test]
fn an_invalid_team_raises_in_armada_one_and_is_the_originals_silent_no_op_in_armada_two() {
    // A1: the shared `team` helper raises (team numbers are 0..=8 there), and nothing is queued.
    let rt = runtime(PlayerNumbering::ArmadaTeams);
    for call in ["A1.EnableTeam(9)", "A1.DisableTeam(-1)", "A1.SetIsAITeam(200, true)"] {
        assert!(rt.eval::<()>(call).is_err(), "{call}");
    }
    assert!(rt.take_ai_requests().is_empty());
    // A2: the same numbers do nothing and raise nothing.
    let rt = runtime(PlayerNumbering::Slots);
    for call in ["A2.EnableTeam(9)", "A2.DisableTeam(-1)", "A2.SetIsAITeam(200, true)"] {
        rt.eval::<()>(call).unwrap_or_else(|e| panic!("{call}: {e}"));
    }
    assert!(rt.take_ai_requests().is_empty());
}

#[test]
fn armada_one_neutral_queues_a_request_the_hub_drops() {
    let s = setup();
    let hub = MissionAiHub::new(sta_ai::AipLibrary::new());
    let _ai = hub.controller(&s, P1);
    // Team 0 is the neutral team in A1's numbering: a valid number, no AI team behind it.
    let rt = a1("A1.EnableTeam(0); A1.DisableTeam(0); A1.SetIsAITeam(0, true); A1.SetIsAITeam(0, false)");
    let requests = rt.take_ai_requests();
    assert_eq!(requests.len(), 4, "queued: {requests:?}");
    hub.submit_requests(requests);
    assert!(!hub.is_ai(sta_sim::world::NEUTRAL) && !hub.is_enabled(sta_sim::world::NEUTRAL));
    assert!(hub.is_ai(P1) && !hub.is_enabled(P1), "the computer seat is untouched");
    assert!(hub.unresolved().is_empty());
    assert!(rt.unsupported().is_empty());
}
