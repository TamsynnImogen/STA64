//! The plugin through a headless app, with the log pointed at memory.

use std::sync::Arc;

use bevy::time::TimeUpdateStrategy;
use sta_content::Races;
use sta_core::{PlayerId, TeamId};
use sta_sim::replay::Replay;
use sta_sim::command::LocalRouter;
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::Sim;

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

#[test]
fn cutscene_and_exit_edges() {
    let sink = MemSink::default();
    let mut app = app(&sink);
    app.init_resource::<CineractiveState>();
    app.update();
    app.world_mut().resource_mut::<CineractiveState>().on = true;
    app.world_mut().resource_mut::<CineractiveState>().skippable = true;
    app.update();
    app.update();
    *app.world_mut().resource_mut::<CineractiveState>() = CineractiveState::default();
    app.update();
    app.world_mut().write_message(AppExit::Success);
    app.update();
    let lines = sink.lines();
    let cut: Vec<_> = lines.iter().filter(|l| l["type"] == "cutscene").map(|l| (l["on"].as_bool().unwrap(), l["skippable"].as_bool().unwrap())).collect();
    assert_eq!(cut, [(true, true), (false, false)]);
    assert_eq!(types(&sink).last().unwrap(), "stop");
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

#[test]
fn a_match_is_logged_once_with_its_tick_and_the_pause() {
    let s = setup();
    let (sim, _) = Sim::from_setup_with_starts(&s, &Races::default());
    let sink = MemSink::default();
    let mut app = app(&sink);
    app.add_plugins(SimLinkPlugin).insert_resource(ContentRes(s.content.clone()));
    let link = SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0)).with_replay(Replay::new(&s));
    app.insert_resource(link).insert_resource(MatchMeta { start: sta_bench::capture::StartMode::Starts, mount: Default::default() });
    for _ in 0..40 {
        app.update();
    }
    app.world_mut().resource_mut::<AnimClock>().set_paused(true);
    app.update();
    let lines = sink.lines();
    let matches: Vec<_> = lines.iter().filter(|l| l["type"] == "match").collect();
    assert_eq!(matches.len(), 1, "{:?}", types(&sink));
    let m = matches[0];
    assert_eq!(m["mode"], "skirmish");
    assert_eq!(m["mod"], "Fleet Ops Roots");
    assert!(m["profile"].is_string() && m["game"].is_string());
    assert_eq!(m["players"].as_array().unwrap().len(), 2);
    assert_eq!(m["players"][0]["control"], "human");
    assert_eq!(m["local_seat"], 0);
    assert_eq!(m["map"], s.map.name.as_str());
    let pause = lines.iter().find(|l| l["type"] == "pause").expect("a pause record");
    assert_eq!(pause["on"], true);
    assert!(pause["tick"].as_u64().unwrap() > 0, "the match runs on a tick");
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
