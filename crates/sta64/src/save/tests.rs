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
use crate::sim_link::{BenchDriven, ContentRes, SimLinkPlugin};

/// Makes the match's sim from the setup and links it with a journal; `Unlogged` leaves the journal out.
#[derive(Resource)]
struct Unlogged;

/// A stateful computer player that reads events: it counts the thinks and the events it was given,
/// and every third think, or whenever it saw events, orders its first ship somewhere that depends on
/// both counts, so a think skipped or repeated changes the game.
#[derive(Clone, Default, PartialEq, Debug)]
struct CounterState {
    thinks: u32,
    events: u32,
    sent: u32,
}

impl CounterState {
    fn bytes(&self) -> Vec<u8> {
        [self.thinks, self.events, self.sent].iter().flat_map(|n| n.to_le_bytes()).collect()
    }

    fn of(data: &[u8]) -> Option<CounterState> {
        let n = |i: usize| data.get(i * 4..i * 4 + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        if data.len() != 12 {
            return None;
        }
        Some(CounterState { thinks: n(0)?, events: n(1)?, sent: n(2)? })
    }
}

struct Counter {
    seat: PlayerId,
    state: Arc<std::sync::Mutex<CounterState>>,
}

impl sta_sim::setup::Controller for Counter {
    fn think(&mut self, view: &sta_sim::WorldView, events: &[sta_sim::SimEvent], out: &mut Vec<sta_sim::command::Command>) {
        let mut s = self.state.lock().unwrap();
        s.thinks += 1;
        s.events += events.len() as u32;
        if events.is_empty() && s.thinks % 3 != 0 {
            return;
        }
        if let Some(u) = view.units.iter().find(|u| u.owner == self.seat) {
            let p = u.pos.to_f32();
            let to = SimVec3::from_f32([p[0] + 100.0 * (s.events % 5 + 1) as f32, 0.0, p[2] + 50.0 * (s.thinks % 7) as f32]);
            out.push(sta_sim::command::Command { player: self.seat, seq: s.sent, kind: CommandKind::Order(Order::move_to(vec![u.id], to, false)) });
            s.sent += 1;
        }
    }

    fn save_state(&self) -> Option<sta_sim::setup::ControllerState> {
        Some(sta_sim::setup::ControllerState { version: 1, data: self.state.lock().unwrap().bytes() })
    }

    fn restore_state(&mut self, state: &sta_sim::setup::ControllerState) -> Result<(), String> {
        match (state.version, CounterState::of(&state.data)) {
            (1, Some(s)) => {
                *self.state.lock().unwrap() = s;
                Ok(())
            }
            _ => Err("unreadable counter record".into()),
        }
    }
}

type Handles = Arc<std::sync::Mutex<std::collections::BTreeMap<u8, Arc<std::sync::Mutex<CounterState>>>>>;

#[derive(Resource, Clone, Default)]
struct CounterHandles(Handles);

impl CounterHandles {
    fn make(&self, seat: PlayerId, state: CounterState) -> Box<dyn sta_sim::setup::Controller> {
        let st = Arc::new(std::sync::Mutex::new(state));
        self.0.lock().unwrap().insert(seat.0, st.clone());
        Box::new(Counter { seat, state: st })
    }

    /// The controller a match build makes: registered unless a load has already registered the seat's
    /// restored one (the build's is replaced by it).
    fn build_for(&self, seat: PlayerId) -> Box<dyn sta_sim::setup::Controller> {
        if self.0.lock().unwrap().contains_key(&seat.0) {
            Box::new(Counter { seat, state: Arc::default() })
        } else {
            self.make(seat, CounterState::default())
        }
    }

    fn state(&self, seat: u8) -> Option<CounterState> {
        self.0.lock().unwrap().get(&seat).map(|s| s.lock().unwrap().clone())
    }

    /// The test's own registry name, for a load to build a computer player by.
    fn extra(&self) -> ExtraControllers {
        let me = self.clone();
        ExtraControllers(Arc::new(move |name, seat| (name == "counter").then(|| me.make(seat, CounterState::default()))))
    }
}

fn build_match(
    mut commands: Commands,
    active: Res<ActiveMatch>,
    link: Option<Res<SimLink>>,
    scene: Option<Res<crate::frontend::scene::MenuScene>>,
    unlogged: Option<Res<Unlogged>>,
    handles: Res<CounterHandles>,
) {
    if !active.requested || link.is_some() || scene.is_some() {
        return;
    }
    let (sim, _) = Sim::from_setup_with_warnings(&active.setup);
    let controllers: Vec<Box<dyn sta_sim::setup::Controller>> = active
        .setup
        .players
        .iter()
        .filter_map(|p| match &p.control {
            Control::Ai(name) if name == "counter" => Some(handles.build_for(p.id)),
            Control::Ai(name) => sta_ai::Registry::builtin().create_for_seat(name, &active.setup, p.id).ok(),
            _ => None,
        })
        .collect();
    let link = SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), active.local.unwrap_or(PlayerId::BENCH)).with_controllers(controllers);
    let link = if active.local.is_none() { link.with_observer() } else { link };
    commands.insert_resource(ContentRes(active.setup.content.clone()));
    commands.insert_resource(if unlogged.is_some() { link } else { link.with_replay(Replay::new(&active.setup)) });
}

/// How many times the map file lookup was called.
#[derive(Resource, Clone)]
struct MapReads(Arc<std::sync::atomic::AtomicUsize>);

impl MapReads {
    fn count(&self) -> usize {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// The front end's fake data, with ships on the two-bases map: a cruiser for seat 0 and a frigate
/// for seat 1, near each other, so a match has something to order and to fight over.
pub(crate) fn armed() -> ScreenData {
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

pub(crate) fn temp_dir(tag: &str) -> PathBuf {
    let mut id = [0; 8];
    getrandom::fill(&mut id).unwrap();
    std::env::temp_dir().join(format!("sta64-ui55-{tag}-{:016x}", u64::from_le_bytes(id)))
}

pub(crate) fn app(dir: &PathBuf, map_file: bool) -> App {
    let (null, _) = NullAudio::new();
    let handles = CounterHandles::default();
    let counted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let reads = counted.clone();
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
        .insert_resource(handles.extra())
        .insert_resource(handles)
        .init_resource::<FogMemory>()
        .insert_resource(MapFiles(Arc::new(move |path| {
            reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            map_file.then(|| format!("authored map {path}").into_bytes())
        })))
        .insert_resource(MapReads(counted));
    app
}

pub(crate) fn setup(data: &ScreenData, seats: [Control; 2]) -> GameSetup {
    let map = data.maps.iter().find(|m| m.def.name == sta_content::fake::two_bases().name).expect("the fake map").def.clone();
    let seat = |i: u8, control| PlayerSetup { id: PlayerId(i), team: TeamId(i), race: "fx".into(), control };
    let [a, b] = seats;
    GameSetup { seed: 5, map, content: data.content.clone(), players: vec![seat(0, a), seat(1, b)], options: GameOptions::default() }
}

pub(crate) fn settle(app: &mut App, frames: usize) {
    for _ in 0..frames {
        app.update();
    }
}

pub(crate) fn state(app: &App) -> AppState {
    *app.world().resource::<State<AppState>>().get()
}

pub(crate) fn start(app: &mut App, setup: GameSetup, speed: MatchSpeed) {
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
                link.submit(CommandKind::Order(Order::move_to(vec![*id], go([p[0] + 600.0, p[1], p[2] + 150.0]), false)));
            }
        }
        12 => {
            if let Some((id, _)) = mine.first() {
                link.submit(CommandKind::Order(Order::Stop { units: vec![*id] }));
            }
        }
        25 => {
            for (id, p) in mine.iter().skip(1) {
                link.submit(CommandKind::Order(Order::move_to(vec![*id], go([p[0] - 200.0, p[1], p[2] - 500.0]), true)));
            }
        }
        _ => {}
    }
}

/// Runs `n` ticks of the match through the link with the script and returns each tick's digest.
pub(crate) fn run(app: &mut App, n: u64) -> Vec<u64> {
    let mut digests = Vec::new();
    for _ in 0..n {
        let mut link = app.world_mut().resource_mut::<SimLink>();
        let t = link.real_curr().tick;
        script(&mut link, t);
        let events = link.tick().expect("a local router never stalls");
        let rejected: Vec<_> = events.iter().filter(|e| matches!(e, sta_sim::SimEvent::Rejected { .. })).collect();
        assert!(rejected.is_empty(), "tick {t}: {rejected:?}");
        digests.push(link.real_curr().digest);
    }
    digests
}

pub(crate) fn drain<T: Message + Clone>(app: &mut App) -> Vec<T> {
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
        CommandKind::Order(Order::move_to(vec![id], SimVec3::from_f32([900.0, 0.0, 900.0]), false))
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

#[derive(Clone, Copy, Debug)]
enum Phase {
    /// Before any tick.
    Start,
    /// The first tick after which the computer player's command is in flight.
    AfterItsCommand,
    /// Player, sandbox and queued input commands all on their way.
    PlayerAndSandbox,
}

fn counter_seats(data: &ScreenData) -> GameSetup {
    setup(data, [Control::Human, Control::Ai("counter".into())])
}

fn counter(app: &App) -> CounterState {
    app.world().resource::<CounterHandles>().state(1).expect("the computer player")
}

fn computer_command_in_flight(app: &App) -> bool {
    app.world().resource::<SimLink>().in_flight().iter().any(|c| c.player == PlayerId(1))
}

/// What the player and the sandbox send at the `PlayerAndSandbox` save point. `queued` is the third,
/// which in the saved run waits in the input's queue and in the reference is submitted at once.
fn inject(app: &mut App, queued: bool) {
    let (id, victim) = {
        let link = app.world().resource::<SimLink>();
        let mut mine = link.real_curr().units.iter().filter(|u| u.owner == link.player()).map(|u| u.id);
        (mine.next().unwrap(), mine.next().unwrap())
    };
    let go = |x: f32| CommandKind::Order(Order::move_to(vec![id], SimVec3::from_f32([x, 0.0, 700.0]), false));
    {
        let mut link = app.world_mut().resource_mut::<SimLink>();
        link.submit(go(900.0));
        // The sandbox's command changes the state for certain (the fake ships' orders show in the journal only).
        link.submit_sandbox(CommandKind::Edit(sta_sim::command::Edit::Kill(victim)));
        if !queued {
            link.submit(go(-900.0));
        }
    }
    if queued {
        app.world_mut().resource_mut::<CommandQueue>().0.push(go(-900.0));
    }
}

fn save_and_load(app: &mut App, tick: u64) {
    app.world_mut().write_message(SaveMatch { description: String::new(), overwrite: None });
    app.update();
    let saved = drain::<SaveOutcome>(app);
    let [SaveOutcome::Saved { path, tick: at }] = saved.as_slice() else { panic!("not saved: {saved:?}") };
    assert_eq!(*at, tick);
    let path = path.clone();
    app.world_mut().write_message(LoadMatch { path });
    let mut loaded = Vec::new();
    for _ in 0..300 {
        app.update();
        loaded.extend(drain::<LoadOutcome>(app));
        if !loaded.is_empty() && state(app) == AppState::InGame {
            break;
        }
    }
    assert!(matches!(loaded.as_slice(), [LoadOutcome::Loaded { .. }]), "{loaded:?}");
}

/// The game saved at a point where the computer player's or the player's commands are in flight, loaded
/// and continued, equals the game that never stopped: every tick's digest, the whole journal, the
/// computer player's state and the command counter.
#[test]
fn a_computer_players_game_saved_with_commands_in_flight_continues_exactly() {
    let dir = temp_dir("counter");
    let data = armed();
    for phase in [Phase::Start, Phase::AfterItsCommand, Phase::PlayerAndSandbox] {
        // The game that never stops.
        let mut reference = app(&dir, true);
        start(&mut reference, counter_seats(&data), MatchSpeed::Normal);
        let mut want = Vec::new();
        let n = match phase {
            Phase::Start => 0,
            Phase::AfterItsCommand => loop {
                want.extend(run(&mut reference, 1));
                if computer_command_in_flight(&reference) {
                    break want.len() as u64;
                }
                assert!(want.len() < 300, "the computer player sent nothing");
            },
            Phase::PlayerAndSandbox => {
                want.extend(run(&mut reference, 20));
                20
            }
        };
        if matches!(phase, Phase::PlayerAndSandbox) {
            inject(&mut reference, false);
        }
        let in_flight_at_save = reference.world().resource::<SimLink>().in_flight().to_vec();
        want.extend(run(&mut reference, M));
        let want_replay = reference.world().resource::<SimLink>().replay().unwrap().clone();
        let (want_state, want_seq) = (counter(&reference), reference.world().resource::<SimLink>().next_seq());
        assert!(want_state.events > 0, "the computer player was given events");
        if !matches!(phase, Phase::Start) {
            assert!(!in_flight_at_save.is_empty());
        }

        // The same game, saved at tick n and loaded.
        let mut app = app(&dir, true);
        start(&mut app, counter_seats(&data), MatchSpeed::Normal);
        let mut got = run(&mut app, n);
        if matches!(phase, Phase::PlayerAndSandbox) {
            inject(&mut app, true);
        }
        let thinks_before = counter(&app).thinks;
        save_and_load(&mut app, n);
        assert_eq!(app.world().resource::<SimLink>().in_flight(), in_flight_at_save.as_slice(), "{phase:?}: the commands in flight, in order");
        assert_eq!(counter(&app).thinks, thinks_before, "{phase:?}: restoring does not make the computer player think");
        got.extend(run(&mut app, M));
        assert_eq!(got, want, "{phase:?}: a digest on every tick");
        let link = app.world().resource::<SimLink>();
        assert_eq!(link.replay().unwrap(), &want_replay, "{phase:?}: the whole journal");
        assert_eq!((counter(&app), link.next_seq()), (want_state, want_seq), "{phase:?}: the computer player's state and the command counter");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The control: a save that does not carry the commands in flight (taken before they were sent) gives
/// another game, so the test above does fail when they are dropped.
#[test]
fn dropping_the_commands_in_flight_changes_the_game() {
    let dir = temp_dir("dropped");
    let data = armed();
    let mut reference = app(&dir, true);
    start(&mut reference, counter_seats(&data), MatchSpeed::Normal);
    let mut want = run(&mut reference, 20);
    inject(&mut reference, false);
    want.extend(run(&mut reference, M));
    // The orders matter to the game at all: without them it runs differently.
    let mut plain = app(&dir, true);
    start(&mut plain, counter_seats(&data), MatchSpeed::Normal);
    let mut without = run(&mut plain, 20);
    without.extend(run(&mut plain, M));
    assert_ne!(want, without, "the orders in flight change the game, or this test proves nothing");

    let mut app = app(&dir, true);
    start(&mut app, counter_seats(&data), MatchSpeed::Normal);
    let mut got = run(&mut app, 20);
    save_and_load(&mut app, 20);
    // The player's and the sandbox's orders were never sent here: the loaded game has no commands in flight.
    got.extend(run(&mut app, M));
    assert_ne!(got, want);
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

    // The skirmish AI has not opted in to saving yet (AI-07): the seat is named, with a clear reason.
    let files = container::store::list(&dir, None).unwrap().len();
    let mut a = app(&dir, true);
    start(&mut a, setup(&data, [Control::Human, Control::Ai(sta_ai::SKIRMISH.into())]), MatchSpeed::Normal);
    let why = blocked(&mut a);
    assert_eq!(why, Some(SaveBlock::Controller(PlayerId(1))));
    assert!(why.unwrap().to_string().contains("seat 1 cannot be saved yet"));
    assert_eq!(container::store::list(&dir, None).unwrap().len(), files, "nothing was written");

    // The idle computer player opts in (stateless): such a game saves.
    let mut a = app(&dir, true);
    start(&mut a, setup(&data, [Control::Human, Control::Ai("idle".into())]), MatchSpeed::Normal);
    assert_eq!(blocked(&mut a), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_of_another_version_or_a_damaged_one_is_refused_with_found_and_expected_and_listed() {
    let dir = temp_dir("versions");
    let data = armed();
    let mut a = app(&dir, true);
    start(&mut a, setup(&data, [Control::Human, Control::Idle]), MatchSpeed::Normal);
    run(&mut a, 3);
    a.world_mut().write_message(SaveMatch { description: "v".into(), overwrite: None });
    a.update();
    let saved = drain::<SaveOutcome>(&mut a);
    let [SaveOutcome::Saved { path, .. }] = saved.as_slice() else { panic!("not saved: {saved:?}") };
    let good = container::store::read_file(path).unwrap();

    let mut other = good.clone();
    other[4..6].copy_from_slice(&9u16.to_le_bytes());
    let mut damaged = good.clone();
    let last = damaged.len() - 1;
    damaged[last] ^= 0xff;
    std::fs::write(dir.join("other.stasave"), &other).unwrap();
    std::fs::write(dir.join("damaged.stasave"), &damaged).unwrap();
    std::fs::write(dir.join("empty.stasave"), b"").unwrap();

    let listed = container::store::list(&dir, None).unwrap();
    assert_eq!(listed.len(), 4, "refused files are listed, not hidden");
    let reason = |name: &str| listed.iter().find(|l| l.file_name == name).unwrap().outcome.as_ref().err().map(ToString::to_string);
    assert_eq!(reason("other.stasave").as_deref().map(|r| r.contains("version 9") && r.contains("reads version 1")), Some(true), "{:?}", reason("other.stasave"));
    assert!(reason("empty.stasave").is_some());
    assert!(listed.iter().any(|l| l.path == *path && l.outcome.is_ok()));

    // The header says the version: refused before the running match is left.
    a.world_mut().write_message(LoadMatch { path: dir.join("other.stasave") });
    a.update();
    let out = drain::<LoadOutcome>(&mut a);
    assert!(matches!(out.as_slice(), [LoadOutcome::Refused { reason, .. }] if reason.contains("version 9") && reason.contains("reads version 1")), "{out:?}");
    assert_eq!(state(&a), AppState::InGame);
    // Damage past the header is found too before the match is left: the whole file is checked first.
    a.world_mut().write_message(LoadMatch { path: dir.join("damaged.stasave") });
    a.update();
    let out = drain::<LoadOutcome>(&mut a);
    assert!(matches!(out.as_slice(), [LoadOutcome::Refused { reason, .. }] if reason.contains("damaged")), "{out:?}");
    settle(&mut a, 40);
    assert_eq!(state(&a), AppState::InGame, "the running match is still running");
    assert!(a.world().get_resource::<PendingLoad>().is_none());
    assert_eq!(a.world().resource::<SimLink>().real_curr().tick, 3, "and it is the same game");
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
    changed.insert_resource(MapFiles(Arc::new(|_| Some(b"another map".to_vec()))));
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

    assert_eq!(state(&gone), AppState::InGame);

    // Other content (another mod active): refused with the running match still running.
    let mut other = app(&dir, true);
    start(&mut other, setup(&data, [Control::Human, Control::Idle]), MatchSpeed::Normal);
    let mut data2 = armed();
    data2.content = Arc::new(sta_content::fake::db_with_formations());
    assert_ne!(data2.content.hash(), data.content.hash());
    other.insert_resource(data2);
    other.world_mut().write_message(LoadMatch { path });
    other.update();
    let out = drain::<LoadOutcome>(&mut other);
    assert!(matches!(out.as_slice(), [LoadOutcome::Refused { reason, .. }] if reason.contains("other content")), "{out:?}");
    settle(&mut other, 40);
    assert_eq!(state(&other), AppState::InGame, "the running match is still running");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A saved game, written by a match of its own app.
fn a_save_on_disk(dir: &PathBuf, data: &ScreenData, fog: crate::frontend::screens::SetupFog) -> (PathBuf, Vec<u64>) {
    let mut a = app(dir, true);
    settle(&mut a, 4);
    a.world_mut().write_message(StartGame(setup(data, [Control::Human, Control::Idle]), MatchSpeed::Fast, None, fog, None));
    for _ in 0..200 {
        a.update();
        if state(&a) == AppState::InGame {
            break;
        }
    }
    settle(&mut a, 3);
    let mut digests = run(&mut a, 20);
    a.world_mut().write_message(SaveMatch { description: "fresh".into(), overwrite: None });
    a.update();
    let saved = drain::<SaveOutcome>(&mut a);
    let [SaveOutcome::Saved { path, .. }] = saved.as_slice() else { panic!("not saved: {saved:?}") };
    digests.extend(run(&mut a, 20));
    (path.clone(), digests)
}

/// The done-when's wording: a fresh app world loads the file the first one wrote, and continues to the
/// same digests; the setup's shroud and fog choice and the speed come back from the file.
#[test]
fn a_fresh_app_loads_the_file_another_app_wrote_with_the_setups_fog_choice() {
    use crate::frontend::screens::SetupFog;
    let dir = temp_dir("fresh");
    let data = armed();
    let (path, want) = a_save_on_disk(&dir, &data, SetupFog::Neither);

    let mut fresh = app(&dir, true);
    settle(&mut fresh, 4);
    assert!(fresh.world().get_resource::<SimLink>().is_none_or(|l| l.replay().is_none()), "nothing carried over");
    fresh.world_mut().write_message(LoadMatch { path });
    let mut loaded = Vec::new();
    for _ in 0..300 {
        fresh.update();
        loaded.extend(drain::<LoadOutcome>(&mut fresh));
        if !loaded.is_empty() && state(&fresh) == AppState::InGame {
            break;
        }
    }
    assert!(matches!(loaded.as_slice(), [LoadOutcome::Loaded { tick: 20, left_out, .. }] if left_out.is_empty()), "{loaded:?}");
    let active = fresh.world().resource::<ActiveMatch>();
    assert_eq!((active.fog, active.speed), (SetupFog::Neither, MatchSpeed::Fast), "the choice made on the setup screen");
    assert_eq!(run(&mut fresh, 20), want[20..]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A load whose start fails does not wait for the next match: the pending load is dropped with a refusal,
/// and an ordinary start gives an ordinary match.
#[test]
fn a_load_whose_start_fails_is_dropped_and_cannot_take_a_later_match() {
    let dir = temp_dir("stale");
    let data = armed();
    let (path, _) = a_save_on_disk(&dir, &data, crate::frontend::screens::SetupFog::Both);

    let mut a = app(&dir, true);
    settle(&mut a, 4);
    // The start the load writes is refused by the flow (a one-seat roster): it never becomes a match.
    let mut broken = armed();
    broken.maps.retain(|m| m.def.name == sta_content::fake::two_bases().name);
    let mut lone = broken.maps[0].def.as_ref().clone();
    lone.starts.truncate(1);
    broken.maps[0] = crate::frontend::screens::MapEntry::new(lone);
    a.insert_resource(broken);
    a.world_mut().write_message(LoadMatch { path: path.clone() });
    let mut out = Vec::new();
    for _ in 0..80 {
        a.update();
        out.extend(drain::<LoadOutcome>(&mut a));
    }
    assert!(matches!(out.as_slice(), [LoadOutcome::Refused { .. }]), "{out:?}");
    assert!(a.world().get_resource::<PendingLoad>().is_none(), "dropped");

    // An ordinary start now gives an ordinary match at tick 0.
    a.insert_resource(armed());
    a.world_mut().write_message(StartGame(setup(&data, [Control::Human, Control::Idle]), MatchSpeed::Normal, None, Default::default(), None));
    for _ in 0..200 {
        a.update();
        if state(&a) == AppState::InGame {
            break;
        }
    }
    settle(&mut a, 5);
    assert_eq!(a.world().resource::<SimLink>().real_curr().tick, 0, "not the saved game");
    assert!(drain::<LoadOutcome>(&mut a).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The availability check reads no file and asks no controller each frame: the map file is read once
/// when the match's link is up, and once more by a save that is taken.
#[test]
fn the_availability_check_does_no_file_io_per_frame() {
    let dir = temp_dir("cheap");
    let data = armed();
    let mut a = app(&dir, true);
    start(&mut a, setup(&data, [Control::Human, Control::Idle]), MatchSpeed::Normal);
    settle(&mut a, 3);
    let reads = a.world().resource::<MapReads>().clone();
    let after_start = reads.count();
    assert!(after_start >= 1, "read once for the match");
    settle(&mut a, 120);
    assert_eq!(reads.count(), after_start, "no read in 120 frames");
    assert_eq!(a.world().resource::<SaveAvailability>().0, None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A saved memory no update took in does not outlive the match it was loaded for.
#[test]
fn a_saved_fog_memory_not_yet_taken_in_is_cleared_on_leaving_the_match() {
    let dir = temp_dir("fogclear");
    let data = armed();
    let mut a = app(&dir, true);
    start(&mut a, setup(&data, [Control::Human, Control::Idle]), MatchSpeed::Normal);
    let grid = FogGrid::new([-4096.0, -4096.0], [4096.0, 4096.0]);
    a.world_mut().resource_mut::<FogMemory>().import = Some(grid.export_explored());
    a.world_mut().write_message(LeaveMatch);
    settle(&mut a, 6);
    assert_eq!(state(&a), AppState::FrontEnd);
    assert!(a.world().resource::<FogMemory>().import.is_none());
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
        let info = SaveInfo { map: Some(MapId { path: format!("bzn/{}.bzn", replay.map.name), hash: file_hash(format!("authored map bzn/{}.bzn", replay.map.name).as_bytes()) }), local_seat: Some(PlayerId(0)), ..SaveInfo::default() };
        let flight = InFlight { next_seq: link.next_seq(), commands: link.in_flight().to_vec() };
        let bytes = container::save(&sim, &replay, &info, &[], &flight, &ClientState { version: record::VERSION + 7, data: b"from the future".to_vec() }).unwrap();
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
