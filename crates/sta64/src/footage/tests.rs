//! The plugin through a headless app, with the log pointed at memory.

use std::sync::Arc;

use bevy::time::TimeUpdateStrategy;
use sta_content::Races;
use sta_core::{PlayerId, TeamId};
use sta_sim::replay::Replay;
use sta_sim::command::LocalRouter;
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, WorldSource};

use super::*;
use crate::media::MovieId;
use crate::sim_link::SimLinkPlugin;
use log::MemSink;

fn app(sink: &MemSink) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).insert_resource(TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f64(1.0 / 60.0)));
    crate::app::configure_sets(&mut app);
    install(&mut app, FootageLog::new(Box::new(sink.clone()), Stamps::now()), Some("Fleet Ops Roots".into()));
    app
}

fn types(sink: &MemSink) -> Vec<String> {
    sink.lines().iter().map(|l| l["type"].as_str().unwrap().to_owned()).collect()
}

#[test]
fn the_first_record_is_start_and_movies_are_logged_by_name() {
    let sink = MemSink::default();
    let mut app = app(&sink);
    app.update();
    let w = app.world_mut();
    w.write_message(PlayMovie { id: MovieId(3), name: "intro.bik".into(), target: MovieTarget::Fullscreen, skippable: true });
    w.write_message(MovieFinished { id: MovieId(3), name: "intro.bik".into(), outcome: MovieOutcome::Skipped });
    w.write_message(MovieFinished { id: MovieId(4), name: "gone.bik".into(), outcome: MovieOutcome::Failed("no file".into()) });
    app.update();
    let lines = sink.lines();
    assert_eq!(types(&sink), ["start", "movie_start", "movie_end", "movie_end"]);
    assert_eq!(lines[0]["format"], "sta64-footage/1");
    assert!(lines[0]["pid"].as_u64().unwrap() > 0 && lines[0]["started_at"].as_str().unwrap().ends_with('Z'));
    assert_eq!((lines[1]["name"].as_str(), lines[1]["target"].as_str(), lines[1]["skippable"].as_bool()), (Some("intro.bik"), Some("fullscreen"), Some(true)));
    assert_eq!((lines[2]["outcome"].as_str(), lines[3]["outcome"].as_str(), lines[3]["reason"].as_str()), (Some("skipped"), Some("failed"), Some("no file")));
    assert!(lines.iter().all(|l| l["tick"].is_null()), "no match, no tick");
    // Stamps never go backwards.
    assert!(lines.windows(2).all(|w| w[0]["ms"].as_u64() <= w[1]["ms"].as_u64() && w[0]["t"].as_f64() <= w[1]["t"].as_f64()));
}

fn setup() -> GameSetup {
    let seat = |i: u8, control| PlayerSetup { id: PlayerId(i), team: TeamId(i), race: "fx".into(), control };
    GameSetup {
        seed: 5,
        map: Arc::new(sta_content::fake::two_bases()),
        content: Arc::new(sta_content::fake::db()),
        players: vec![seat(0, Control::Human), seat(1, Control::Idle)],
        options: GameOptions::default(),
    }
}

/// A fresh match link, and the ids of units of both seats.
fn match_link(s: &GameSetup) -> (SimLink, Vec<EntityId>) {
    let (sim, _) = Sim::from_setup_with_starts(s, &Races::default());
    let ids = sim.view().units.iter().map(|u| u.id).collect();
    (SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0)).with_replay(Replay::new(s)), ids)
}

fn meta() -> MatchMeta {
    MatchMeta { start: sta_bench::capture::StartMode::Starts, mount: Default::default() }
}

/// The app with a match running (the sim link steps from the clock).
fn match_app(sink: &MemSink, s: &GameSetup) -> (App, Vec<EntityId>) {
    let mut app = app(sink);
    app.add_plugins(SimLinkPlugin).insert_resource(ContentRes(s.content.clone())).init_resource::<CineractiveState>();
    let (link, ids) = match_link(s);
    app.insert_resource(link).insert_resource(meta());
    (app, ids)
}

fn run(app: &mut App, frames: u32) {
    for _ in 0..frames {
        app.update();
    }
}

fn of<'a>(lines: &'a [Value], ty: &str) -> Vec<&'a Value> {
    lines.iter().filter(|l| l["type"] == ty).collect()
}

#[test]
fn cutscene_pause_and_exit_close_what_is_open() {
    let sink = MemSink::default();
    let s = setup();
    let (mut app, _) = match_app(&sink, &s);
    run(&mut app, 5);
    *app.world_mut().resource_mut::<CineractiveState>() = CineractiveState { on: true, skippable: true };
    app.world_mut().resource_mut::<AnimClock>().set_paused(true);
    run(&mut app, 3);
    app.world_mut().write_message(AppExit::Success);
    app.update();
    let lines = sink.lines();
    let cut: Vec<_> = of(&lines, "cutscene").iter().map(|l| (l["on"].as_bool().unwrap(), l["reason"].as_str().map(str::to_owned))).collect();
    assert_eq!(cut, [(true, None), (false, Some("exit".into()))]);
    let pause: Vec<_> = of(&lines, "pause").iter().map(|l| (l["on"].as_bool().unwrap(), l["reason"].as_str().map(str::to_owned))).collect();
    assert_eq!(pause, [(true, None), (false, Some("exit".into()))]);
    assert_eq!(types(&sink).last().unwrap(), "stop");
    assert_eq!(lines.last().unwrap()["code"], 0);
}

#[test]
fn leaving_the_match_closes_intervals_with_a_reason_and_no_false_resume() {
    let sink = MemSink::default();
    let s = setup();
    let (mut app, _) = match_app(&sink, &s);
    run(&mut app, 5);
    *app.world_mut().resource_mut::<CineractiveState>() = CineractiveState { on: true, skippable: false };
    app.world_mut().resource_mut::<AnimClock>().set_paused(true);
    run(&mut app, 2);
    app.world_mut().remove_resource::<SimLink>();
    run(&mut app, 3);
    let lines = sink.lines();
    let off: Vec<_> = lines.iter().filter(|l| l["on"] == false).map(|l| (l["type"].as_str().unwrap(), l["reason"].as_str().unwrap_or(""))).collect();
    assert_eq!(off, [("cutscene", "match_end"), ("pause", "match_end")]);
    assert!(of(&lines, "cutscene").len() == 2 && of(&lines, "pause").len() == 2, "no second on after the match: {:?}", types(&sink));
}

#[test]
fn a_match_is_logged_with_the_app_state_stuck_at_boot() {
    // `--skirmish` has an `AppState` that never leaves `Boot`; no `Flow` runs.
    let sink = MemSink::default();
    let s = setup();
    let mut app = app(&sink);
    app.add_plugins(bevy::state::app::StatesPlugin).init_state::<AppState>();
    app.add_plugins(SimLinkPlugin).insert_resource(ContentRes(s.content.clone()));
    app.insert_resource(match_link(&s).0).insert_resource(meta());
    run(&mut app, 5);
    let lines = sink.lines();
    assert_eq!(*app.world().resource::<State<AppState>>().get(), AppState::Boot);
    assert_eq!(of(&lines, "match").len(), 1, "{:?}", types(&sink));
    let m = of(&lines, "match")[0];
    assert_eq!((m["mode"].as_str(), m["mod"].as_str(), m["local_seat"].as_u64()), (Some("skirmish"), Some("Fleet Ops Roots"), Some(0)));
    assert_eq!((m["players"].as_array().unwrap().len(), m["players"][0]["control"].as_str()), (2, Some("human")));
    assert_eq!(m["map"], s.map.name.as_str());
    assert!(m["profile"].is_string() && m["game"].is_string());
    assert!(lines.last().unwrap()["tick"].is_u64());
}

#[test]
fn with_the_front_end_flow_a_match_waits_for_in_game_and_a_second_one_starts_fresh() {
    let sink = MemSink::default();
    let s = setup();
    let mut app = app(&sink);
    app.add_plugins(bevy::state::app::StatesPlugin).init_state::<AppState>();
    app.add_plugins(SimLinkPlugin).insert_resource(ContentRes(s.content.clone())).insert_resource(Flow).init_resource::<CineractiveState>();
    app.insert_resource(match_link(&s).0).insert_resource(meta());
    run(&mut app, 3);
    assert!(of(&sink.lines(), "match").is_empty(), "loading is not a match yet");
    let shot = |app: &mut App| {
        let at = Spot::Position(sta_sim::Vec3::from_f32([1.0, 2.0, 3.0]));
        app.world_mut().write_message(SimMsg(SimEvent::Present(Presentation::Shot { from: at.clone(), at, seconds: Fixed::ZERO })));
        app.update();
    };
    let go = |app: &mut App, state| {
        app.world_mut().resource_mut::<NextState<AppState>>().set(state);
        run(app, 3);
    };
    go(&mut app, AppState::InGame);
    shot(&mut app);
    shot(&mut app);
    app.world_mut().resource_mut::<FootageState>().verdict = Some("victory");
    go(&mut app, AppState::FrontEnd);
    app.world_mut().remove_resource::<SimLink>();
    run(&mut app, 2);
    // A second match, with its own link.
    app.insert_resource(match_link(&s).0);
    go(&mut app, AppState::InGame);
    shot(&mut app);
    let lines = sink.lines();
    assert_eq!(of(&lines, "match").len(), 2, "{:?}", types(&sink));
    let shots: Vec<_> = of(&lines, "shot").iter().map(|l| l["shot"].as_u64().unwrap()).collect();
    assert_eq!(shots, [1, 2, 1], "the shot counter restarts with the match");
    assert_eq!(app.world().resource::<FootageState>().verdict, None);
    let states: Vec<_> = of(&lines, "app_state").iter().map(|l| l["state"].as_str().unwrap().to_owned()).collect();
    assert_eq!(states, ["boot", "in_game", "front_end", "in_game"]);
}

#[test]
fn events_outside_a_match_are_dropped_and_unseen_units_are_not_reported() {
    let sink = MemSink::default();
    let s = setup();
    let (mut app, ids) = match_app(&sink, &s);
    app.world_mut().remove_resource::<SimLink>();
    let id = ids[0];
    app.world_mut().write_message(SimMsg(SimEvent::Destroyed { id, by: None }));
    run(&mut app, 3);
    let (link, _) = match_link(&s);
    app.insert_resource(link);
    run(&mut app, 3);
    assert!(of(&sink.lines(), "destroyed").is_empty(), "no match, no record");
    // Seen units are reported, with class and owner; a unit nobody saw is not.
    app.world_mut().write_message(SimMsg(SimEvent::Destroyed { id, by: Some(ids[1]) }));
    app.world_mut().write_message(SimMsg(SimEvent::Destroyed { id: EntityId(900_000), by: None }));
    run(&mut app, 2);
    let lines = sink.lines();
    let d = of(&lines, "destroyed");
    assert_eq!(d.len(), 1);
    assert_eq!((d[0]["id"].as_u64(), d[0]["by"].as_u64()), (Some(u64::from(id.0)), Some(u64::from(ids[1].0))));
    assert!(d[0]["class"].is_string() && d[0]["owner"].is_u64());
}

#[test]
fn shields_down_is_one_record_per_unit_and_facing_until_it_is_up() {
    let sink = MemSink::default();
    let s = setup();
    let (mut app, ids) = match_app(&sink, &s);
    run(&mut app, 3);
    for facing in [0, 0, 0, 1, 0] {
        app.world_mut().write_message(SimMsg(SimEvent::ShieldsDown { id: ids[0], facing }));
    }
    run(&mut app, 1);
    let lines = sink.lines();
    let f: Vec<_> = of(&lines, "shields_down").iter().map(|l| l["facing"].as_u64().unwrap()).collect();
    assert_eq!(f, [0, 1]);
}

#[test]
fn sight_decides_what_a_record_may_say() {
    let seen = |class: &str| Seen { class: Some(class.into()), owner: 1 };
    let last: BTreeMap<EntityId, Seen> = BTreeMap::from([(EntityId(1), seen("a"))]);
    let now: BTreeSet<EntityId> = BTreeSet::from([EntityId(2)]);
    let views = |id: EntityId| (id == EntityId(2)).then(|| seen("b"));
    // Seen last frame: its remembered class, even when it is gone from both views.
    assert_eq!(resolve_seen(EntityId(1), &last, &now, views), Some(seen("a")));
    // Drawn now only (just built): from the views.
    assert_eq!(resolve_seen(EntityId(2), &last, &now, views), Some(seen("b")));
    // Neither: fogged, no record.
    assert_eq!(resolve_seen(EntityId(3), &last, &now, views), None);
}

#[test]
fn requested_by_flag_or_environment() {
    assert_eq!(requested_from(None, None), None);
    assert_eq!(requested_from(Some(""), None).as_deref(), Some(""));
    assert_eq!(requested_from(Some("/tmp/a.jsonl"), Some("1")).as_deref(), Some("/tmp/a.jsonl"));
    assert_eq!(requested_from(None, Some("1")).as_deref(), Some("1"));
    assert_eq!(requested_from(None, Some("/tmp/x/")).as_deref(), Some("/tmp/x/"));
    assert_eq!(requested_from(None, Some("")), None);
    assert_eq!(requested_from(None, Some("0")), None);
}

#[test]
fn an_unwritable_place_is_a_warning_and_the_plugin_adds_nothing() {
    // A file where a folder is wanted.
    let file = std::env::temp_dir().join(format!("sta64-footage-test-{}-blocker", std::process::id()));
    std::fs::write(&file, b"x").unwrap();
    let mut app = App::new();
    app.add_plugins(FootagePlugin { spec: file.join("sub/log.jsonl").display().to_string(), launch_mod: None });
    assert!(!app.world().contains_resource::<FootageLog>());
    app.update();
    let _ = std::fs::remove_file(&file);
}

#[test]
fn the_plugin_writes_a_file_with_one_line_per_record() {
    let path = std::env::temp_dir().join(format!("sta64-footage-test-{}-file.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    crate::app::configure_sets(&mut app);
    app.add_plugins(FootagePlugin { spec: path.display().to_string(), launch_mod: None });
    app.update();
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 1);
    assert!(text.ends_with('\n') && text.starts_with(r#"{"ms":"#));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn pose_changes_need_more_than_the_epsilon() {
    let a = [100.0, 0.0, 200.0, 0.5, 0.9, 8000.0];
    assert!(!pose_moved(&a, &a));
    assert!(!pose_moved(&a, &[100.4, 0.0, 200.0, 0.505, 0.9, 8010.0]));
    assert!(pose_moved(&a, &[105.0, 0.0, 200.0, 0.5, 0.9, 8000.0]));
    assert!(pose_moved(&a, &[100.0, 0.0, 200.0, 0.6, 0.9, 8000.0]));
    assert!(pose_moved(&a, &[100.0, 0.0, 200.0, 0.5, 0.9, 8200.0]));
}

#[test]
fn cast_is_capped_to_the_largest_with_the_full_total() {
    let unit = |id: u32, size: f32| CastUnit { id, class: Some("fx_ship".into()), name: None, owner: 1, race: Some("fx".into()), at: Placement { x: 0.5, y: 0.25, size } };
    let units: Vec<_> = (0..50).map(|i| unit(i, 0.01 + i as f32 * 0.001)).collect();
    let v = cast_fields("shot", Some(2), units);
    assert_eq!((v["total"].as_u64(), v["units"].as_array().unwrap().len()), (Some(50), CAST_MAX));
    assert_eq!(v["units"][0]["id"], 49, "biggest first");
    let one = cast_fields("fight", None, vec![unit(9, 0.1234567)]);
    let l = log::line(10, 0.25, Some(7), "cast", one);
    assert_eq!(
        l,
        r#"{"ms":10,"t":0.25,"tick":7,"type":"cast","shot":null,"total":1,"units":[{"class":"fx_ship","id":9,"name":null,"owner":1,"race":"fx","size":0.123,"x":0.5,"y":0.25}],"why":"fight"}"#
    );
}

#[test]
fn golden_lines_for_the_other_records() {
    let l = |ty: &str, f: Value| log::line(1000, 2.0, Some(120), ty, f);
    assert_eq!(l("app_state", json!({"state": "in_game", "from": "loading"})), r#"{"ms":1000,"t":2.0,"tick":120,"type":"app_state","from":"loading","state":"in_game"}"#);
    assert_eq!(l("destroyed", json!({"id": 5, "by": 9, "class": "fx_ship", "owner": 1})), r#"{"ms":1000,"t":2.0,"tick":120,"type":"destroyed","by":9,"class":"fx_ship","id":5,"owner":1}"#);
    assert_eq!(l("shot", json!({"shot": 1, "from": spot_json(&Spot::Entity(EntityId(3))), "at": spot_json(&Spot::Path("p".into())), "seconds": 2.5})), r#"{"ms":1000,"t":2.0,"tick":120,"type":"shot","at":{"path":"p"},"from":{"entity":3},"seconds":2.5,"shot":1}"#);
    assert_eq!(l("fight_in_frame", json!({"on": true, "units": 2, "classes": ["a", null]})), r#"{"ms":1000,"t":2.0,"tick":120,"type":"fight_in_frame","classes":["a",null],"on":true,"units":2}"#);
}
