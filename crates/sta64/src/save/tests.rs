//! UI-55's headless tests: a match started through the front end's flow in a bare `App` (as
//! `game::flow_tests` does), stepped tick by tick through the link, saved through [`SaveMatch`],
//! torn down, loaded through [`LoadMatch`] and continued. The map view's part is a stand-in that
//! builds the sim from the setup and arms the link's journal, as `map` and `capture::arm` do.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use sta_core::{PlayerId, TeamId};
use sta_sim::command::{CommandKind, LocalRouter, Order};
use sta_sim::replay::Replay;
use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use sta_sim::{Sim, Vec3 as SimVec3};

use super::*;
use crate::app::{AppState, configure_sets};
use crate::audio::{Mixer, MusicConfig, NullAudio};
use crate::clock::MatchSpeed;
use crate::frontend::loading::LoadingPlugin;
use crate::frontend::scene::{MenuMap, MenuSceneConfig, MenuSceneCorePlugin};
use crate::frontend::screens::{ScreenData, ScreensConfig, ScreensPlugin};
use crate::game::flow::{FlowPlugin, MenuSceneSource};
use crate::render::fog::FogGrid;
use crate::sim_link::{BenchDriven, SimLinkPlugin};

/// Makes the match's sim from the setup and links it with a journal; `Unlogged` leaves the journal out.
#[derive(Resource)]
struct Unlogged;

fn build_match(mut commands: Commands, active: Res<ActiveMatch>, link: Option<Res<SimLink>>, scene: Option<Res<crate::frontend::scene::MenuScene>>, unlogged: Option<Res<Unlogged>>) {
    if !active.requested || link.is_some() || scene.is_some() {
        return;
    }
    let (sim, _) = Sim::from_setup_with_warnings(&active.setup);
    let link = SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), active.local.unwrap_or(PlayerId::BENCH));
    let link = if active.local.is_none() { link.with_observer() } else { link };
    commands.insert_resource(ContentRes(active.setup.content.clone()));
    commands.insert_resource(if unlogged.is_some() { link } else { link.with_replay(Replay::new(&active.setup)) });
}

/// The front end's fake data, with ships on the two-bases map: a cruiser for seat 0 and a frigate
/// for seat 1, near each other, so a match has something to order and to fight over.
fn armed() -> ScreenData {
    let mut data = ScreenData::fake();
    let mut map = sta_content::fake::two_bases();
    let ship = |odf: &str, owner: u8, x: f32, z: f32, h: f32| sta_content::MapObject {
        odf: odf.into(),
        owner: Some(owner),
        pos: SimVec3::from_f32([x, 0.0, z]),
        heading: (sta_sim::Fixed::from_int(h as i32), sta_sim::Fixed::ZERO),
        label: None,
    };
    map.objects.extend([ship("fx_cruiser", 0, -300.0, 0.0, 1.0), ship("fx_cruiser", 0, -300.0, 200.0, 1.0), ship("fx_frigate", 1, 500.0, 50.0, -1.0)]);
    let entry = data.maps.iter_mut().find(|m| m.def.name == map.name).expect("the fake map");
    *entry = crate::frontend::screens::MapEntry::new(map);
    data
}

fn temp_dir(tag: &str) -> PathBuf {
    let mut id = [0; 8];
    getrandom::fill(&mut id).unwrap();
    std::env::temp_dir().join(format!("sta64-ui55-{tag}-{:016x}", u64::from_le_bytes(id)))
}

fn app(dir: &PathBuf, map_file: bool) -> App {
    let (null, _) = NullAudio::new();
    let mut app = App::new();
    app.set_error_handler(bevy::ecs::error::panic)
        .add_plugins((MinimalPlugins, StatesPlugin))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(0.1)))
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ButtonInput<KeyCode>>()
        .insert_non_send(Mixer::new(Box::new(null)))
        .insert_resource(crate::audio::AudioTestTones(true));
    configure_sets(&mut app);
    let data = armed();
    let scene = MenuSceneConfig::new(data.content.clone(), MenuMap::builtin(), sta_content::fake::two_bases());
    app.insert_resource(data)
        .insert_resource(ScreensConfig { own_camera: false, fit_scale: false, ..default() })
        .insert_resource(MenuSceneSource(scene))
        .insert_resource(MusicConfig::default())
        .insert_resource(crate::clock::AnimClock::new(crate::clock::ClockMode::Wall, 0))
        .add_plugins((FlowPlugin, SimLinkPlugin, MenuSceneCorePlugin, ScreensPlugin, LoadingPlugin, crate::audio::AudioPlugin::default(), crate::input::InputPlugin, crate::ui::hud::HudPlugin, SavePlugin))
        .add_systems(Update, build_match.run_if(resource_exists::<ActiveMatch>))
        // The test steps the link itself, tick by tick.
        .insert_resource(BenchDriven)
        .insert_resource(SaveDir(Some(dir.clone())))
        .init_resource::<FogMemory>()
        .insert_resource(MapFiles(Arc::new(move |name| map_file.then(|| (format!("bzn/{name}.bzn"), format!("authored map {name}").into_bytes())))));
    app
}

fn setup(data: &ScreenData, seats: [Control; 2]) -> GameSetup {
    let map = data.maps.iter().find(|m| m.def.name == sta_content::fake::two_bases().name).expect("the fake map").def.clone();
    let seat = |i: u8, control| PlayerSetup { id: PlayerId(i), team: TeamId(i), race: "fx".into(), control };
    let [a, b] = seats;
    GameSetup { seed: 5, map, content: data.content.clone(), players: vec![seat(0, a), seat(1, b)], options: GameOptions::default() }
}

fn settle(app: &mut App, frames: usize) {
    for _ in 0..frames {
        app.update();
    }
}

fn state(app: &App) -> AppState {
    *app.world().resource::<State<AppState>>().get()
}

fn start(app: &mut App, setup: GameSetup, speed: MatchSpeed) {
    settle(app, 4);
    app.world_mut().write_message(StartGame(setup, speed, None, Default::default(), None));
    for _ in 0..200 {
        app.update();
        if state(app) == AppState::InGame {
            break;
        }
    }
    assert_eq!(state(app), AppState::InGame);
    settle(app, 3);
}

/// The scripted orders: a move for everything, a stop, a second move, at fixed ticks.
fn script(link: &mut SimLink, tick: u64) {
    let me = link.player();
    let mine: Vec<(sta_sim::EntityId, [f32; 3])> = link.real_curr().units.iter().filter(|u| u.owner == me).map(|u| (u.id, u.pos.to_f32())).collect();
    let go = |to: [f32; 3]| SimVec3::from_f32(to);
    match tick {
        2 => {
            for (id, p) in &mine {
                link.submit(CommandKind::Order(Order::Move { units: vec![*id], to: go([p[0] + 600.0, p[1], p[2] + 150.0]), queue: false }));
            }
        }
        12 => {
            if let Some((id, _)) = mine.first() {
                link.submit(CommandKind::Order(Order::Stop { units: vec![*id] }));
            }
        }
        25 => {
            for (id, p) in mine.iter().skip(1) {
                link.submit(CommandKind::Order(Order::Move { units: vec![*id], to: go([p[0] - 200.0, p[1], p[2] - 500.0]), queue: true }));
            }
        }
        _ => {}
    }
}

/// Runs `n` ticks of the match through the link with the script and returns each tick's digest.
fn run(app: &mut App, n: u64) -> Vec<u64> {
    let mut digests = Vec::new();
    for _ in 0..n {
        let mut link = app.world_mut().resource_mut::<SimLink>();
        let t = link.real_curr().tick;
        script(&mut link, t);
        link.tick().expect("a local router never stalls");
        digests.push(link.real_curr().digest);
    }
    digests
}

fn drain<T: Message + Clone>(app: &mut App) -> Vec<T> {
    app.world_mut().resource_mut::<Messages<T>>().drain().collect()
}

const N: u64 = 40;
const M: u64 = 60;

#[test]
fn a_saved_match_loads_and_continues_to_the_same_digests_as_one_that_never_stopped() {
    let dir = temp_dir("roundtrip");
    let data = armed();
    let seats = || setup(&data, [Control::Human, Control::Idle]);

    // The match that never stops.
    let mut reference = app(&dir, true);
    start(&mut reference, seats(), MatchSpeed::Fast);
    let want = run(&mut reference, N + M);
    assert!(want.windows(2).filter(|w| w[0] != w[1]).count() > 20, "the match has ships that move, or the test proves nothing");
    let want_replay = reference.world().resource::<SimLink>().replay().unwrap().clone();

    // The same match, saved at tick N.
    let mut app = app(&dir, true);
    start(&mut app, seats(), MatchSpeed::Fast);
    let first = run(&mut app, N);
    assert_eq!(first, want[..N as usize]);
    // What the client holds besides the sim: a camera pose and explored ground.
    app.world_mut().spawn((Camera3d::default(), Orbit { focus: Vec3::new(120.5, 0.0, -80.25), yaw: 0.4, pitch: -0.9, dist: 1234.5, home: (Vec3::ZERO, 1.0) }));
    let mut grid = FogGrid::new([-4096.0, -4096.0], [4096.0, 4096.0]);
    grid.set_sensor_for_test(Vec2::new(500.0, -250.0), 900.0);
    app.world_mut().resource_mut::<FogMemory>().set_grid_for_test(grid);
    let explored = app.world().resource::<FogMemory>().explored().unwrap();
    app.world_mut().resource_mut::<Selection>().0.push(sta_sim::EntityId(1));
    app.world_mut().write_message(SaveMatch { description: "round trip".into(), overwrite: None });
    app.update();
    let saved = drain::<SaveOutcome>(&mut app);
    let [SaveOutcome::Saved { path, tick }] = saved.as_slice() else { panic!("not saved: {saved:?}") };
    assert_eq!(*tick, N);
    assert!(path.starts_with(&dir) && path.extension().is_some_and(|e| e == "stasave"));
    // The file reads back without the match, and says what it is.
    let header = container::read_header(&container::store::read_file(path).unwrap()).unwrap();
    assert_eq!((header.tick, header.description.as_str()), (N, "round trip"));
    let listed = container::store::list(&dir, None).unwrap();
    assert_eq!(listed.len(), 1);

    // Another pose and an empty selection: what a fresh start would have.
    {
        let mut q = app.world_mut().query::<&mut Orbit>();
        for mut o in q.iter_mut(app.world_mut()) {
            (o.focus, o.yaw, o.pitch, o.dist) = (Vec3::ZERO, 0.0, 0.0, 10.0);
        }
    }
    // Load: the running match is left, the start goes through the loading screen, the saved game takes over.
    app.world_mut().write_message(LoadMatch { path: path.clone() });
    let mut loaded = Vec::new();
    for _ in 0..300 {
        app.update();
        loaded.extend(drain::<LoadOutcome>(&mut app));
        if !loaded.is_empty() && state(&app) == AppState::InGame {
            break;
        }
    }
    assert!(matches!(loaded.as_slice(), [LoadOutcome::Loaded { tick, left_out, .. }] if *tick == N && left_out.is_empty()), "{loaded:?}");
    assert_eq!(state(&app), AppState::InGame);
    assert_eq!(app.world().resource::<SimLink>().real_curr().tick, N);
    assert_eq!(app.world().resource::<ActiveMatch>().speed, MatchSpeed::Fast, "the game speed is the saved one");
    assert!(app.world().resource::<Selection>().0.is_empty(), "the selection starts empty");
    let orbit = app.world_mut().query::<&Orbit>().single(app.world()).unwrap();
    assert_eq!((orbit.focus, orbit.yaw, orbit.pitch, orbit.dist), (Vec3::new(120.5, 0.0, -80.25), 0.4, -0.9, 1234.5), "the camera pose is the saved one");
    assert_eq!(app.world().resource::<FogMemory>().import.as_ref(), Some(&explored), "the explored memory waits for the fog's next update, unchanged");

    // Continue: the same digest on every tick, and the same journal, as the match that never stopped.
    let after = run(&mut app, M);
    assert_eq!(after, want[N as usize..]);
    let link = app.world().resource::<SimLink>();
    assert_eq!(link.replay().unwrap(), &want_replay, "the journal continues the saved one from tick 0");

    // A capture of the loaded game still replays: the journal plays back from tick 0 to the same digests.
    let content = data.content.clone();
    let played = link.replay().unwrap().play(content).expect("the journal replays");
    assert_eq!(played.digest(), *want.last().unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn commands_in_flight_at_the_save_and_the_players_next_number_survive_the_load() {
    let dir = temp_dir("inflight");
    let data = armed();
    let seats = || setup(&data, [Control::Human, Control::Idle]);
    let order = |link: &SimLink| {
        let me = link.player();
        let id = link.real_curr().units.iter().find(|u| u.owner == me).unwrap().id;
        CommandKind::Order(Order::Move { units: vec![id], to: SimVec3::from_f32([900.0, 0.0, 900.0]), queue: false })
    };

    let mut reference = app(&dir, true);
    start(&mut reference, seats(), MatchSpeed::Normal);
    run(&mut reference, 10);
    let o = order(reference.world().resource::<SimLink>());
    reference.world_mut().resource_mut::<SimLink>().submit(o);
    let want = run(&mut reference, 10);

    let mut app = app(&dir, true);
    start(&mut app, seats(), MatchSpeed::Normal);
    run(&mut app, 10);
    // Queued by input, not yet stamped: the save puts it in flight, and the file keeps it.
    let o = order(app.world().resource::<SimLink>());
    app.world_mut().resource_mut::<CommandQueue>().0.push(o);
    app.world_mut().write_message(SaveMatch { description: String::new(), overwrite: None });
    app.update();
    let saved = drain::<SaveOutcome>(&mut app);
    let [SaveOutcome::Saved { path, .. }] = saved.as_slice() else { panic!("not saved") };
    let path = path.clone();
    assert_eq!(app.world().resource::<SimLink>().in_flight().len(), 1);

    app.world_mut().write_message(LoadMatch { path });
    for _ in 0..300 {
        app.update();
        if state(&app) == AppState::InGame && app.world().resource::<SimLink>().real_curr().tick == 10 {
            break;
        }
    }
    assert_eq!(app.world().resource::<SimLink>().in_flight().len(), 1, "handed to the new router");
    let after = run(&mut app, 10);
    assert_eq!(after, want, "the order applies on the next tick as in the match that never stopped");
    let _ = std::fs::remove_dir_all(&dir);
}

fn blocked(app: &mut App) -> Option<SaveBlock> {
    app.world_mut().write_message(SaveMatch { description: String::new(), overwrite: None });
    app.update();
    let out = drain::<SaveOutcome>(app);
    match out.as_slice() {
        [SaveOutcome::NotAllowed(why)] => {
            assert_eq!(app.world().resource::<SaveAvailability>().0.as_ref(), Some(why), "the menu is told the same reason");
            Some(why.clone())
        }
        [SaveOutcome::Saved { .. }] => None,
        other => panic!("{other:?}"),
    }
}

#[test]
fn save_is_not_allowed_where_it_cannot_be_restored_and_says_why() {
    let dir = temp_dir("blocked");
    let data = armed();

    // No match at all.
    let mut idle = app(&dir, true);
    settle(&mut idle, 4);
    assert_eq!(blocked(&mut idle), Some(SaveBlock::NotInMatch));

    // A map without a file (a generated map, one started from the editor).
    let mut a = app(&dir, false);
    start(&mut a, setup(&data, [Control::Human, Control::Idle]), MatchSpeed::Normal);
    assert_eq!(blocked(&mut a), Some(SaveBlock::NoMapFile));
    assert!(container::store::list(&dir, None).unwrap().is_empty(), "nothing was written");

    // A journal-less link (a replay or a bench session).
    let mut a = app(&dir, true);
    a.insert_resource(Unlogged);
    start(&mut a, setup(&data, [Control::Human, Control::Idle]), MatchSpeed::Normal);
    assert_eq!(blocked(&mut a), Some(SaveBlock::NoJournal));

    // A cutscene, a movie.
    let mut a = app(&dir, true);
    start(&mut a, setup(&data, [Control::Human, Control::Idle]), MatchSpeed::Normal);
    a.world_mut().insert_resource(crate::ui::presentation::CineractiveState { on: true, skippable: false });
    assert_eq!(blocked(&mut a), Some(SaveBlock::Cutscene));
    a.world_mut().insert_resource(crate::ui::presentation::CineractiveState::default());
    a.world_mut().insert_resource(crate::media::Movies::busy_for_test());
    assert_eq!(blocked(&mut a), Some(SaveBlock::Movie));
    a.world_mut().insert_resource(crate::media::Movies::default());
    assert_eq!(blocked(&mut a), None, "and allowed again when none runs");

    // An observer.
    let mut a = app(&dir, true);
    let ai = Control::Ai("idle".into());
    start(&mut a, setup(&data, [ai.clone(), ai]), MatchSpeed::Normal);
    assert_eq!(blocked(&mut a), Some(SaveBlock::Observer));

    // A computer player that has not opted in to saving: the seat is named.
    let mut a = app(&dir, true);
    start(&mut a, setup(&data, [Control::Human, Control::Ai("idle".into())]), MatchSpeed::Normal);
    assert_eq!(blocked(&mut a), Some(SaveBlock::Controller(PlayerId(1))));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_save_that_cannot_be_loaded_here_is_refused_with_its_reason_and_the_match_is_left_running() {
    let dir = temp_dir("refused");
    let data = armed();
    let mut a = app(&dir, true);
    start(&mut a, setup(&data, [Control::Human, Control::Idle]), MatchSpeed::Normal);
    run(&mut a, 5);
    a.world_mut().write_message(SaveMatch { description: "x".into(), overwrite: None });
    a.update();
    let saved = drain::<SaveOutcome>(&mut a);
    let [SaveOutcome::Saved { path, .. }] = saved.as_slice() else { panic!("not saved") };
    let path = path.clone();

    // Not a save.
    let junk = dir.join("junk.stasave");
    std::fs::write(&junk, b"nothing like a save").unwrap();
    a.world_mut().write_message(LoadMatch { path: junk });
    a.update();
    let out = drain::<LoadOutcome>(&mut a);
    assert!(matches!(out.as_slice(), [LoadOutcome::Refused { reason, .. }] if reason.contains("not a STA64 save")), "{out:?}");
    assert_eq!(state(&a), AppState::InGame, "refused before leaving the match");

    // A file that is not there.
    a.world_mut().write_message(LoadMatch { path: dir.join("missing.stasave") });
    a.update();
    assert!(matches!(drain::<LoadOutcome>(&mut a).as_slice(), [LoadOutcome::Refused { .. }]));

    // The map file changed since the save: refused before the match is left.
    let mut changed = app(&dir, true);
    changed.insert_resource(MapFiles(Arc::new(|name| Some((format!("bzn/{name}.bzn"), b"another map".to_vec())))));
    start(&mut changed, setup(&data, [Control::Human, Control::Idle]), MatchSpeed::Normal);
    changed.world_mut().write_message(LoadMatch { path: path.clone() });
    changed.update();
    let out = drain::<LoadOutcome>(&mut changed);
    assert!(matches!(out.as_slice(), [LoadOutcome::Refused { reason, .. }] if reason.contains("has changed")), "{out:?}");
    assert_eq!(state(&changed), AppState::InGame);

    // The map file is gone.
    let mut gone = app(&dir, false);
    start(&mut gone, setup(&data, [Control::Human, Control::Idle]), MatchSpeed::Normal);
    gone.world_mut().write_message(LoadMatch { path: path.clone() });
    gone.update();
    let out = drain::<LoadOutcome>(&mut gone);
    assert!(matches!(out.as_slice(), [LoadOutcome::Refused { reason, .. }] if reason.contains("not available")), "{out:?}");

    // Other content: found after the build, which is then taken down again.
    let mut other = app(&dir, true);
    let mut data2 = armed();
    data2.content = Arc::new(sta_content::fake::db_with_formations());
    assert_ne!(data2.content.hash(), data.content.hash());
    other.insert_resource(data2);
    settle(&mut other, 4);
    other.world_mut().write_message(LoadMatch { path });
    let mut out = Vec::new();
    for _ in 0..300 {
        other.update();
        out.extend(drain::<LoadOutcome>(&mut other));
        if !out.is_empty() && state(&other) == AppState::FrontEnd {
            break;
        }
    }
    assert!(matches!(out.as_slice(), [LoadOutcome::Refused { reason, .. }] if reason.contains("other content")), "{out:?}");
    assert_eq!(state(&other), AppState::FrontEnd, "back at the front end");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_unreadable_client_record_loads_the_match_with_defaults_and_says_so() {
    let dir = temp_dir("defaults");
    let data = armed();
    let mut a = app(&dir, true);
    start(&mut a, setup(&data, [Control::Human, Control::Idle]), MatchSpeed::Normal);
    run(&mut a, 3);
    // A save as another build's client would have written it: the sim is the same, the record is not ours.
    let (path, bytes) = {
        let mut link = a.world_mut().resource_mut::<SimLink>();
        let sim = Sim::from_snapshot(&link.snapshot().unwrap(), Some(data.content.clone())).unwrap();
        let replay = link.replay().unwrap().clone();
        let info = SaveInfo { map: Some(MapId { path: format!("bzn/{}.bzn", replay.map.name), hash: file_hash(format!("authored map {}", replay.map.name).as_bytes()) }), local_seat: Some(PlayerId(0)), ..SaveInfo::default() };
        let bytes = container::save(&sim, &replay, &info, &[], &ClientState { version: record::VERSION + 7, data: b"from the future".to_vec() }).unwrap();
        (container::store::write_new(&dir, &bytes).unwrap_or_else(|_| dir.join("x")), bytes)
    };
    let _ = bytes;
    a.world_mut().write_message(LoadMatch { path });
    let mut loaded = Vec::new();
    for _ in 0..300 {
        a.update();
        loaded.extend(drain::<LoadOutcome>(&mut a));
        if !loaded.is_empty() && state(&a) == AppState::InGame {
            break;
        }
    }
    assert!(matches!(loaded.as_slice(), [LoadOutcome::Loaded { tick: 3, left_out, .. }] if left_out.len() == 1), "{loaded:?}");
    assert_eq!(a.world().resource::<ActiveMatch>().speed, MatchSpeed::Normal);
    assert!(a.world().resource::<FogMemory>().import.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}
