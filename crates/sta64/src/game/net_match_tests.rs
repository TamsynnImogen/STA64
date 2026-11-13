//! A match from the lobby, played by several client `App`s in lockstep (INT-M5). Everything
//! is headless: no window, no GPU, no audio. The clients connect through a relay in this
//! process on 127.0.0.1 with an OS-chosen port; every wait is a loop on a real condition with
//! a deadline (a 1 ms yield between polls, never a fixed sleep standing in for an event), and
//! no test assumes two connections are ordered against each other.
//!
//! Each client is the front end flow (`FlowPlugin`: loading, the match, leaving), the lobby
//! screens, the sim link and the net match plugin; the one stand-in is the map view's build
//! (no BZN files with fake content), which runs `build_side` and `link`, the functions
//! `map::rebuild` runs. Scripted commands go into `CommandQueue`, where the input systems put
//! theirs; from there they take the real path (`SimLink::submit`, the router adapter, the
//! lockstep router, the relay, every peer's router, the sim).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use sta_content::{RaceDef, Races};
use sta_core::{PlayerId, Vec3};
use sta_net::lockstep::{Destination, Fault};
use sta_net::proto::{Message, TurnCommands};
use sta_net::resync::ResyncLimits;
use sta_net::transport::{Relay, RelayConfig};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::replay::StartRec;
use sta_sim::setup::Control;
use sta_sim::{EntityId, Sim};

use super::flow::*;
use super::net_match::*;
use super::net_overlay::NetStatus;
use crate::app::{AppState, configure_sets};
use crate::clock::{AnimClock, ClockMode};
use crate::frontend::lobby::tests::*;
use crate::frontend::lobby::MpAct;
use crate::frontend::lobby::Multiplayer;
use crate::frontend::screens::{Screen, ScreenData, ScreensConfig, ScreensPlugin};
use crate::map::{MapBrowser, MapSeat};
use crate::sim_link::{CommandQueue, SimLink, SimLinkPlugin};

/// How long a wait may go without any peer making progress (a tick run, a line said or shown)
/// before it fails. The deadline follows the work: a slow machine that keeps stepping is never
/// failed for being slow, only a match that stopped moving is.
const STALLED: Duration = Duration::from_secs(60);

/// The lobby-driven matches run one at a time: each spins several apps, a relay and its
/// threads flat out, and eleven at once on a four-core runner (beside the rest of the suite)
/// starved each other into the timeouts of #619.
static ONE_MATCH_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    // A test that failed holding it does not fail the others.
    ONE_MATCH_AT_A_TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What moves while a match is alive: every peer's tick and what it said (not the overlay, whose
/// stall line counts seconds by itself), and which screen and state each app is in.
fn progress(apps: &[App]) -> Vec<(u64, usize, String)> {
    apps.iter()
        .map(|a| {
            let (tick, said) = a.world().get_resource::<NetGame>().map_or((0, 0), |g| (g.tick(), g.read(|s| s.said.len())));
            (tick, said, format!("{:?}", state(a)))
        })
        .collect()
}

/// A deadline that moves on while the apps make progress, under a cap so that a match that
/// keeps running without ever meeting the condition still fails.
struct Watch {
    seen: Vec<(u64, usize, String)>,
    end: Instant,
    cap: Instant,
}

impl Watch {
    fn new(apps: &[App]) -> Watch {
        Watch { seen: progress(apps), end: Instant::now() + STALLED, cap: Instant::now() + 10 * STALLED }
    }

    /// Whether the apps went `STALLED` without progress.
    fn stalled(&mut self, apps: &[App]) -> bool {
        let now = progress(apps);
        if now != self.seen {
            self.seen = now;
            self.end = Instant::now() + STALLED;
        }
        Instant::now() >= self.end || Instant::now() >= self.cap
    }
}

fn race(name: &str, slot: i64) -> RaceDef {
    RaceDef {
        stem: name.into(),
        name: name.into(),
        display_key: None,
        slot: Some(slot),
        interface: None,
        interface_sprites: None,
        single_player_music: None,
        multiplayer_music: None,
        standard_units: vec!["fx_yard".into(), "fx_cruiser".into(), "fx_cruiser".into(), "fx_frigate".into()],
        minimal_units: Vec::new(),
        stock: [5000, 0, 0, 0],
        lots_stock: [9000, 0, 0, 0],
        normal_crew: Some(200),
        lots_crew: Some(400),
    }
}

/// The fake install, four sides that start with a yard, two cruisers and a frigate each.
fn net_data() -> ScreenData {
    let mut d = ScreenData::fake();
    d.races = Arc::new(Races::from_defs(vec![race("fed", 0), race("kli", 1), race("rom", 2), race("bor", 3)]));
    d
}

fn fast_relay() -> Relay {
    // The default rates are for people: a test steps its clients as fast as it can.
    let cfg = RelayConfig { frames_per_sec: 1_000_000, burst_frames: 1_000_000, bytes_per_sec: 1 << 30, burst_bytes: 1 << 30, max_connections_per_ip: 64, max_rooms_per_ip: 64, ..RelayConfig::default() };
    Relay::bind("127.0.0.1:0", cfg).expect("a relay on an OS-chosen port")
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sta64-net-{name}-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn opts(dir: &Path) -> NetOptions {
    NetOptions { report_dir: Some(dir.to_path_buf()), stall_after: Duration::from_millis(60), drop_grace: Duration::from_millis(400), limits: None }
}

/// The map view's build, as a stand-in: what `map::rebuild` does for a lobby match, with the
/// map from the setup unless a test says this machine has another.
#[derive(Resource, Default)]
struct OwnMap(Option<sta_content::MapDef>);

fn build_match(mut commands: Commands, active: Res<ActiveMatch>, mut browser: ResMut<MapBrowser>, data: Res<ScreenData>, mut net: NetBuild, own: Res<OwnMap>) {
    if !active.requested || !browser.dirty {
        return;
    }
    browser.dirty = false;
    let Some(nm) = net.take() else { return };
    let map = own.0.clone().unwrap_or_else(|| (*nm.setup.map).clone());
    match build_side(&nm, map, &data.races) {
        Ok(side) => {
            let local = side.seats.local.clone();
            let (link, game) = link(nm, &side.setup, side.sim, &data.races, side.controllers, &net.options()).expect("the peer");
            commands.insert_resource(link);
            commands.insert_resource(game);
            commands.insert_resource(local);
            commands.insert_resource(crate::sim_link::ContentRes(data.content.clone()));
        }
        Err(why) => {
            browser.info = why.clone();
            net.fail(nm, why);
        }
    }
}

/// Valid orders from this client's own units, as the input systems would queue them.
#[derive(Default)]
struct Script {
    moved: bool,
    built: bool,
    attacked: bool,
}

fn script(mut queue: ResMut<CommandQueue>, link: Option<Res<SimLink>>, mut st: Local<Script>, content: Option<Res<crate::sim_link::ContentRes>>) {
    let Some(link) = link else { return };
    let (view, me) = (link.curr().clone(), link.player());
    let name = |u: &sta_sim::UnitView| content.as_ref().and_then(|c| c.0.name(u.def)).unwrap_or("").to_owned();
    let mine: Vec<&sta_sim::UnitView> = view.units.iter().filter(|u| u.owner == me).collect();
    let ships: Vec<EntityId> = mine.iter().filter(|u| name(u) == "fx_cruiser").map(|u| u.id).collect();
    if view.tick >= 12 && !st.moved && !ships.is_empty() {
        st.moved = true;
        // Toward the middle of the map, where the other side's ships will be.
        let to = Vec3::from_f32([0.0, 0.0, 400.0 * f32::from(me.0 + 1)]);
        queue.0.push(CommandKind::Order(Order::Move { units: ships.clone(), to, queue: false }));
    }
    if view.tick >= 18 && !st.built && let Some(yard) = mine.iter().find(|u| name(u) == "fx_yard") {
        st.built = true;
        let what = content.as_ref().and_then(|c| c.0.key("fx_frigate")).expect("fx_frigate");
        queue.0.push(CommandKind::Order(Order::Build { builder: yard.id, what, at: None }));
    }
    if view.tick >= 300 && !st.attacked && !ships.is_empty() && let Some(enemy) = view.units.iter().find(|u| u.owner != me && u.owner != sta_sim::world::NEUTRAL && name(u) == "fx_cruiser") {
        st.attacked = true;
        queue.0.push(CommandKind::Order(Order::Attack { units: ships, target: enemy.id }));
    }
}

/// Pauses the clock once the peer has reached the tick, so two peers can be compared at one tick.
#[derive(Resource)]
struct Stop(u64);

fn stop_at(stop: Res<Stop>, game: Option<Res<NetGame>>, mut clock: ResMut<AnimClock>) {
    if game.is_some_and(|g| g.tick() >= stop.0) {
        clock.set_paused(true);
    }
}

fn client(data: ScreenData, o: NetOptions) -> App {
    let mut app = App::new();
    app.set_error_handler(bevy::ecs::error::panic).add_plugins((MinimalPlugins, StatesPlugin));
    configure_sets(&mut app);
    app.add_plugins(crate::ui::profile::UiProfilePlugin);
    let browser = crate::game::flow::browser_for(&data, String::new());
    app.insert_resource(data)
        .insert_resource(ScreensConfig { own_camera: false, fit_scale: false, start: Screen::Main, ..default() })
        // Six ticks a frame: two turns of three.
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(200)))
        .init_resource::<ButtonInput<KeyCode>>()
        .insert_resource(AnimClock::new(ClockMode::Wall, 0))
        .insert_resource(browser)
        .insert_resource(OwnMap::default())
        .insert_resource(o)
        .add_plugins((FlowPlugin, SimLinkPlugin, ScreensPlugin))
        .add_systems(Update, stop_at.in_set(crate::app::GameSet::Bench).run_if(resource_exists::<Stop>))
        .add_systems(Update, build_match.run_if(resource_exists::<ActiveMatch>).in_set(crate::app::GameSet::Bench))
        .add_systems(Update, script.in_set(crate::app::GameSet::Input).run_if(resource_exists::<SimLink>).run_if(resource_exists::<NetGame>));
    app.world_mut().resource_mut::<NextState<AppState>>().set(AppState::FrontEnd);
    settle(&mut app);
    app
}

fn state(a: &App) -> AppState {
    *a.world().resource::<State<AppState>>().get()
}

fn game(a: &App) -> NetGame {
    a.world().resource::<NetGame>().clone()
}

fn playing(a: &App) -> bool {
    state(a) == AppState::InGame && a.world().contains_resource::<NetGame>()
}

/// Updates every app until `cond`, failing at the deadline.
fn until(apps: &mut [App], what: &str, cond: impl Fn(&[App]) -> bool) {
    let mut watch = Watch::new(apps);
    loop {
        for a in apps.iter_mut() {
            a.update();
        }
        if cond(apps) {
            return;
        }
        if watch.stalled(apps) {
            let said: Vec<_> = apps.iter().map(|a| a.world().get_resource::<NetGame>().map(|g| (g.tick(), g.read(|s| s.said.clone()), dump(g)))).collect();
            panic!("no progress for {STALLED:?} waiting for: {what}; peers: {said:#?}");
        }
        std::thread::yield_now();
    }
}

/// What a peer's router and resync state say, for a failure message.
fn dump(g: &NetGame) -> String {
    g.with_peer(|p| {
        let r = p.router();
        let tick = r.desync().map(|n| n.theirs.tick).or_else(|| r.peer_desync().map(|(_, n)| n.ours.tick));
        format!(
            "halted={} desync={:?} peer_desync_from={:?} digests@{tick:?}={:?} resyncing={} failed={} diverged_from_host={:?}",
            r.halted(),
            r.desync().map(|n| (n.slot.0, n.ours.tick)),
            r.peer_desync().map(|(f, _)| f.0),
            tick.map(|t| r.digests_at(t)),
            p.resyncing(),
            p.resync_failed(),
            p.diverged_from_host(),
        )
    })
}

/// Presses a guest's Ready once the lobby has the host's table and the button is there: the
/// guest's screen follows the host's `Lobby` message, which no other app's state implies.
fn ready_up(apps: &mut [App], guest: usize) {
    pump_apps(&mut refs(apps), "the guest's Ready button is enabled", |a| act_entity(a[guest], MpAct::ToggleReady).is_some());
    press(&mut apps[guest], MpAct::ToggleReady);
}

fn refs(apps: &mut [App]) -> Vec<&mut App> {
    apps.iter_mut().collect()
}

/// Host and guests through the real screens to the lobby, side picks, ready, Launch.
/// `computers`: slots the host makes computer players. Returns once every app is in the match.
fn launch(relay: &Relay, n_guests: usize, map: usize, computers: &[usize], o: &NetOptions) -> Vec<App> {
    let data = net_data();
    let mut apps: Vec<App> = (0..=n_guests).map(|_| client(data.clone(), o.clone())).collect();
    let names = ["Picard", "Riker", "Data", "Worf"];
    host_through_screens(&mut apps[0], relay, names[0], "");
    pump_apps(&mut [&mut apps[0]], "the host is in the lobby", |a| in_screen(a[0], Screen::Lobby));
    // The map is picked on its own screen in the product; here the session is told directly.
    apps[0].world_mut().resource_mut::<Multiplayer>().session.as_mut().unwrap().set_map(map).expect("the map");
    for &c in computers {
        apps[0].world_mut().resource_mut::<Multiplayer>().session.as_mut().unwrap().set_slot_kind(c, crate::frontend::lobby::Kind::Computer);
    }
    let code = session_of(&apps[0]).code().to_string();
    for i in 1..=n_guests {
        join_through_screens(&mut apps[i], relay, names[i], &code, "");
        let seats = i + 1;
        pump_apps(&mut refs(&mut apps), "the guest is seated", move |a| in_screen(a[i], Screen::Lobby) && session_of(a[0]).view().seats.iter().filter(|s| s.kind == crate::frontend::lobby::Kind::Human).count() == seats);
    }
    for i in 1..=n_guests {
        ready_up(&mut apps, i);
    }
    pump_apps(&mut refs(&mut apps), "the host may launch", |a| act_entity(a[0], MpAct::Launch).is_some());
    press(&mut apps[0], MpAct::Launch);
    until(&mut apps, "every app is in the match", |a| a.iter().all(playing));
    apps
}

/// Updates every app until each has run `ticks`, failing if one halts or stalls for good.
fn run_to(apps: &mut [App], ticks: u64) {
    until(apps, &format!("tick {ticks}"), |a| a.iter().all(|x| !x.world().contains_resource::<NetGame>() || game(x).tick() >= ticks));
}

/// The digests every peer logged at the same ticks agree; returns how many ticks were compared.
fn assert_same_digests(apps: &[App]) -> usize {
    let logs: Vec<Vec<(u64, u64)>> = apps.iter().filter(|a| a.world().contains_resource::<NetGame>()).map(|a| game(a).with_peer(|p| p.replay().digests.clone())).collect();
    let mut common = 0;
    for (t, d) in &logs[0] {
        for other in &logs[1..] {
            if let Some((_, e)) = other.iter().find(|(u, _)| u == t) {
                assert_eq!(e, d, "digests differ at tick {t}");
            }
        }
        if logs[1..].iter().all(|o| o.iter().any(|(u, _)| u == t)) {
            common += 1;
        }
    }
    common
}

/// The kinds of command each seat sent, as every peer applied them.
fn kinds_applied(a: &App, player: u8) -> Vec<&'static str> {
    game(a).with_peer(|p| {
        p.replay()
            .commands
            .iter()
            .filter(|l| l.command.player == PlayerId(player))
            .map(|l| match &l.command.kind {
                CommandKind::Order(Order::Move { .. }) => "move",
                CommandKind::Order(Order::Build { .. }) => "build",
                CommandKind::Order(Order::Attack { .. }) => "attack",
                _ => "other",
            })
            .collect()
    })
}

fn sleep_ms(ms: u64) {
    // Wall time that must pass for a stall to be a stall; every other wait is on a condition.
    std::thread::sleep(Duration::from_millis(ms));
}

// ---------------------------------------------------------------------------
// Playing
// ---------------------------------------------------------------------------

#[test]
fn two_clients_play_three_thousand_ticks_in_step_with_scripted_orders() {
    let _one = one_at_a_time();
    let (r, dir) = (fast_relay(), tmp("two"));
    let mut apps = launch(&r, 1, 0, &[], &opts(&dir));
    // The match is the host's `Start` on each machine: same seats, own seat human, MapSeat set.
    for (i, a) in apps.iter().enumerate() {
        assert_eq!(a.world().resource::<MapSeat>().0, Some(i as u8));
        assert_eq!(a.world().resource::<SimLink>().player(), PlayerId(i as u8));
        assert!(a.world().contains_resource::<crate::game::LocalRace>());
        let setup = &a.world().resource::<ActiveMatch>().setup;
        assert_eq!(setup.players.iter().filter(|p| p.control == Control::Human).count(), 1);
    }
    run_to(&mut apps, 3000);
    let common = assert_same_digests(&apps);
    assert!(common >= 900, "{common} turns compared");
    for (i, a) in apps.iter().enumerate() {
        let kinds = kinds_applied(a, 0).into_iter().chain(kinds_applied(a, 1)).collect::<Vec<_>>();
        for want in ["move", "build", "attack"] {
            assert_eq!(kinds.iter().filter(|k| **k == want).count(), 2, "app {i}: every seat's {want} order reached every peer once ({kinds:?})");
        }
        let rejected = game(a).read(|s| s.peer().router().halted());
        assert!(!rejected, "app {i} halted");
        // The ships moved, the yard built, nobody stepped outside the router: the sim's tick is the peer's.
        let link = a.world().resource::<SimLink>();
        assert_eq!(link.curr().tick, game(a).tick());
        assert_eq!(link.curr().digest, game(a).digest());
        assert!(link.curr().units.len() > 8, "the frigates were built: {} units", link.curr().units.len());
    }
    // Same replay start on every peer (starting forces recorded for `sta-bench desync`).
    for a in &apps {
        assert!(matches!(game(a).with_peer(|p| p.replay().starts), StartRec::Races { .. }));
    }
    // The overlay is silent while all is well.
    assert!(apps[0].world().resource::<NetStatus>().0.is_empty());
}

#[test]
fn three_clients_play_in_step() {
    let _one = one_at_a_time();
    let (r, dir) = (fast_relay(), tmp("three"));
    let mut apps = launch(&r, 2, 1, &[], &opts(&dir));
    run_to(&mut apps, 3000);
    assert!(assert_same_digests(&apps) >= 900);
    for a in &apps {
        let n: usize = (0..3).map(|p| kinds_applied(a, p).len()).sum();
        assert_eq!(n, 9, "three orders from each of three seats");
    }
}

#[test]
fn the_host_runs_the_computer_seat_and_every_peer_agrees_to_the_snapshot() {
    let _one = one_at_a_time();
    let (r, dir) = (fast_relay(), tmp("ai"));
    let mut apps = launch(&r, 1, 1, &[2], &opts(&dir));
    for a in &apps {
        let players = &a.world().resource::<ActiveMatch>().setup.players;
        let ai = players.iter().find(|p| p.id == PlayerId(2)).unwrap();
        // The host runs the controller; the guest holds the same AI seat without one (#569).
        assert!(matches!(ai.control, Control::Ai(_) | Control::RemoteAi(_)), "{:?}", ai.control);
    }
    let controllers = |a: &mut App| a.world_mut().resource_mut::<SimLink>().controllers();
    assert_eq!(controllers(&mut apps[0]), 1, "the host runs the AI");
    assert_eq!(controllers(&mut apps[1]), 0, "the guest does not");
    run_to(&mut apps, 3000);
    assert!(assert_same_digests(&apps) >= 900);
    // Equal state, not only equal digests: with both peers frozen at the same tick the snapshots are the same bytes.
    let target = apps.iter().map(|a| game(a).tick()).max().unwrap() + 30;
    for a in &mut apps {
        // One tick a frame from here, so no peer overshoots the target.
        a.world_mut().insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_micros(33_400)));
        // Run each peer to exactly `target` by stepping its sim link only while below it.
        a.world_mut().insert_resource(Stop(target));
    }
    until(&mut apps, "both peers sit at the target tick", |a| a.iter().all(|x| game(x).tick() == target));
    let (a, b) = (apps[0].world_mut().resource_mut::<SimLink>().snapshot(), apps[1].world_mut().resource_mut::<SimLink>().snapshot());
    assert!(a.is_some() && a == b, "the snapshots differ at tick {target}");
    // The AI's commands went through the router like a player's: the replay holds seat 2's orders on both.
    for a in &apps {
        let from_ai = game(a).with_peer(|p| p.replay().commands.iter().filter(|l| l.command.player == PlayerId(2)).count());
        assert!(from_ai > 0, "the AI sent no command in 3000 ticks");
    }
}

// ---------------------------------------------------------------------------
// Stalls and drops
// ---------------------------------------------------------------------------

#[test]
fn a_client_that_stalls_is_waited_for_in_words_and_resumes() {
    let _one = one_at_a_time();
    let (r, dir) = (fast_relay(), tmp("stall"));
    let mut apps = launch(&r, 2, 1, &[], &opts(&dir));
    run_to(&mut apps, 300);
    // Seat 2 stops updating (a frozen window); the other two play until they cannot.
    let mut rest = apps.split_off(2);
    let mut watch = Watch::new(&apps);
    loop {
        for a in apps.iter_mut() {
            a.update();
        }
        if apps.iter().all(|a| game(a).read(|s| s.stall.as_ref().is_some_and(|(_, w)| w.contains(&PlayerId(2))))) && apps.iter().all(|a| a.world().resource::<NetStatus>().0.iter().any(|l| l.contains("Waiting for Data"))) {
            break;
        }
        assert!(!watch.stalled(&apps), "never showed the wait");
        sleep_ms(2);
    }
    let line = apps[0].world().resource::<NetStatus>().0.iter().find(|l| l.contains("Waiting for")).cloned().unwrap();
    assert!(line.contains("Data") && line.contains(" s)."), "{line}");
    assert!(apps[1].world().resource::<NetStatus>().0.iter().any(|l| l.contains("Waiting for Data")), "every peer shows it");
    // Nothing ran meanwhile, nothing diverged.
    let frozen = game(&apps[0]).tick();
    sleep_ms(150);
    for a in apps.iter_mut() {
        a.update();
    }
    assert!(game(&apps[0]).tick() <= frozen + 3);
    // It comes back: everyone plays on and the wait is gone.
    apps.append(&mut rest);
    let resume = frozen + 600;
    run_to(&mut apps, resume);
    assert!(assert_same_digests(&apps) > 100);
    until(&mut apps, "the wait line clears", |a| a[0].world().resource::<NetStatus>().0.is_empty());
}

#[test]
fn a_client_that_drops_is_marked_and_the_others_play_on() {
    let _one = one_at_a_time();
    let (r, dir) = (fast_relay(), tmp("drop"));
    let mut apps = launch(&r, 2, 1, &[], &opts(&dir));
    run_to(&mut apps, 300);
    // Seat 2's connection dies without a word.
    game(&apps[2]).control().abort();
    let gone = apps.pop().unwrap();
    let before = game(&apps[0]).tick();
    until(&mut apps, "the host drops the seat", |a| a.iter().all(|x| game(x).read(|s| s.dropped.contains_key(&PlayerId(2)))));
    run_to(&mut apps, before + 600);
    assert!(assert_same_digests(&apps) > 100);
    for a in &apps {
        let lines = a.world().resource::<NetStatus>().0.clone();
        let dropped = lines.iter().find(|l| l.contains("Dropped out")).unwrap_or_else(|| panic!("{lines:?}"));
        assert!(dropped.contains("Data") && dropped.contains("not decided yet"), "{dropped}");
        assert!(!lines.iter().any(|l| l.contains("Waiting for")));
    }
    // NET-02's behaviour: the seat's units are still in the game, and nothing eliminated it.
    let view = apps[0].world().resource::<SimLink>().curr().clone();
    assert!(view.units.iter().any(|u| u.owner == PlayerId(2)));
    assert!(!view.players.iter().find(|p| p.id == PlayerId(2)).unwrap().eliminated);
    // The host said so as it happened.
    assert!(game(&apps[0]).read(|s| s.said.iter().any(|l| l.contains("lost the connection"))));
    gone.world();
}

#[test]
fn the_host_leaving_ends_the_match_for_the_others_with_a_message() {
    let _one = one_at_a_time();
    let (r, dir) = (fast_relay(), tmp("hostleft"));
    let mut apps = launch(&r, 1, 0, &[], &opts(&dir));
    run_to(&mut apps, 300);
    // F10 on the host: leave.
    apps[0].world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::F10);
    until(&mut apps, "the guest is back at the multiplayer screen", |a| state(&a[0]) == AppState::FrontEnd && state(&a[1]) == AppState::FrontEnd);
    apps[0].world_mut().resource_mut::<ButtonInput<KeyCode>>().release(KeyCode::F10);
    let guest = &apps[1];
    assert_eq!(screen_of(guest), Screen::Multiplayer);
    let notice = guest.world().resource::<Multiplayer>().notice.clone().unwrap();
    assert!(notice.contains("host left"), "{notice}");
    assert!(!guest.world().contains_resource::<NetGame>() && !guest.world().contains_resource::<SimLink>() || guest.world().resource::<SimLink>().player() != PlayerId(1));
    // The guest wrote its replay before it went.
    let replays = std::fs::read_dir(&dir).unwrap().filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().starts_with("match-") && e.file_name().to_string_lossy().ends_with(".star")).count();
    assert!(replays >= 1, "no replay in {dir:?}");
}

#[test]
fn a_removed_client_is_back_at_the_multiplayer_screen_with_the_reason() {
    let _one = one_at_a_time();
    let (r, dir) = (fast_relay(), tmp("removed"));
    let mut apps = launch(&r, 2, 1, &[], &opts(&dir));
    run_to(&mut apps, 300);
    game(&apps[0]).control().kick(PlayerId(2)).unwrap();
    until(&mut apps, "the kicked client leaves", |a| state(&a[2]) == AppState::FrontEnd);
    assert_eq!(screen_of(&apps[2]), Screen::Multiplayer);
    let notice = apps[2].world().resource::<Multiplayer>().notice.clone().unwrap();
    assert!(notice.contains("removed"), "{notice}");
    // The others drop the seat and play on.
    let at = game(&apps[0]).tick();
    let mut rest = apps.split_off(2);
    run_to(&mut apps, at + 450);
    assert!(assert_same_digests(&apps) > 100);
    rest.clear();
}

#[test]
fn a_protocol_violation_is_a_kick() {
    let _one = one_at_a_time();
    let (r, dir) = (fast_relay(), tmp("violation"));
    let mut apps = launch(&r, 2, 1, &[], &opts(&dir));
    run_to(&mut apps, 300);
    // Seat 2 sends a record for a turn nobody asked for: the host's router calls it a violation.
    game(&apps[2]).with_peer(|p| {
        let bogus = TurnCommands { turn: 999_999, slot: PlayerId(2), commands: Vec::new(), schedule: Vec::new(), digest: None };
        p.router_mut().send_message(Destination::All, &Message::Turn(bogus)).unwrap();
    });
    until(&mut apps, "the host kicks it", |a| game(&a[0]).read(|s| s.violations.contains_key(&PlayerId(2))) && state(&a[2]) == AppState::FrontEnd);
    assert_eq!(protocol_reaction(PlayerId(0), PlayerId(0), PlayerId(2)), Reaction::Kick(PlayerId(2)));
    let said = game(&apps[0]).read(|s| s.said.iter().any(|l| l.contains("was removed from the game")));
    assert!(said);
    // A peer that is not the host only notes it; the host breaking the protocol ends the match.
    assert_eq!(protocol_reaction(PlayerId(1), PlayerId(0), PlayerId(2)), Reaction::Note);
    assert_eq!(protocol_reaction(PlayerId(1), PlayerId(0), PlayerId(0)), Reaction::EndMatch);
}

// ---------------------------------------------------------------------------
// Desync
// ---------------------------------------------------------------------------

fn spoil() -> Fault {
    // A ship that only this peer's sim has.
    let odf = net_data().content.key("fx_frigate").unwrap();
    Fault::Inject(Command {
        player: PlayerId::SCRIPT,
        seq: 9_000_000,
        kind: CommandKind::Edit(Edit::Spawn { odf, player: PlayerId(1), pos: Vec3::from_f32([100.0, 0.0, 100.0]), heading: (sta_core::Fixed::ZERO, sta_core::Fixed::ONE) }),
    })
}

fn reports(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "star")).collect();
    v.sort();
    v
}

fn side_of(path: &Path, data: &ScreenData) -> sta_net::diagnose::Side {
    let label = path.file_name().unwrap().to_string_lossy().into_owned();
    let replay = sta_net::diagnose::read_replay(&label, &std::fs::read(path).unwrap()).unwrap();
    let setup = replay.setup(data.content.clone()).unwrap();
    let sim = match replay.starts {
        StartRec::Bare => Sim::from_setup(&setup),
        StartRec::Races { .. } => Sim::from_setup_with_starts(&setup, &data.races).0,
    };
    let sidecar = std::fs::read_to_string(format!("{}.desync.txt", path.display())).ok();
    sta_net::diagnose::Side { label, replay, sim, sidecar }
}

#[test]
fn an_injected_desync_halts_every_peer_and_writes_reports_the_diagnosis_localises() {
    let _one = one_at_a_time();
    let (r, dir) = (fast_relay(), tmp("desync"));
    // The host will not repair anyone (a seat with no resyncs left): the match stays stopped.
    let o = NetOptions { limits: Some(ResyncLimits { resyncs_per_seat: 0, min_attempt_interval: Duration::from_millis(1), repair_wait: Duration::from_secs(3), ..ResyncLimits::default() }), ..opts(&dir) };
    let mut apps = launch(&r, 2, 1, &[], &o);
    run_to(&mut apps, 300);
    // Damage on seat 1's sim only, applied on its next step.
    game(&apps[1]).inject(spoil());
    // Halted, the report written, and at the tick that differs: a peer that the others stopped
    // before it got there runs on to it and stops there.
    until(&mut apps, "every peer halted with a report at the differing tick", |a| {
        a.iter().all(|x| {
            game(x).read(|s| {
                let r = s.peer().router();
                r.halted() && s.desync.as_ref().is_some_and(|d| d.report.is_some() && r.own_digest(d.tick).is_some())
            })
        })
    });
    // They stay halted: the match does not run on.
    let at: Vec<_> = apps.iter().map(|a| game(a).tick()).collect();
    for _ in 0..20 {
        for a in apps.iter_mut() {
            a.update();
        }
    }
    assert_eq!(apps.iter().map(|a| game(a).tick()).collect::<Vec<_>>(), at);
    // Each peer's words name the tick and the report, outside the HUD.
    for a in &apps {
        let lines = a.world().resource::<NetStatus>().0.clone();
        assert!(lines.iter().any(|l| l.contains("out of step at tick")), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("Report:") && l.contains(".star")), "{lines:?}");
    }
    until(&mut apps, "every peer says the match is stopped for good", |a| a.iter().all(|x| x.world().resource::<NetStatus>().0.iter().any(|l| l.contains("stopped for good"))));
    // Reports: one `.star` and one sidecar per peer, and the diagnosis finds the injected tick.
    let files = reports(&dir);
    assert_eq!(files.len(), 3, "{files:?}");
    for f in &files {
        assert!(PathBuf::from(format!("{}.desync.txt", f.display())).exists());
    }
    let seat_of = |p: &PathBuf| p.file_name().unwrap().to_string_lossy().contains("seat1");
    let bad = files.iter().find(|p| seat_of(p)).unwrap();
    let host = files.iter().find(|p| p.file_name().unwrap().to_string_lossy().contains("seat0")).unwrap();
    let data = net_data();
    let report = sta_net::diagnose::diagnose(side_of(host, &data), side_of(bad, &data), &sta_net::diagnose::Options::default()).expect("a diagnosis");
    assert!(report.diverged(), "{}", report.to_markdown());
    let d = report.divergence.as_ref().unwrap();
    let injected = game(&apps[1]).with_peer(|p| p.replay().commands.iter().find(|l| l.command.seq == 9_000_000).map(|l| l.tick));
    assert!(injected.is_none_or(|t| d.tick >= t.saturating_sub(3) && d.tick <= t + 3), "diverged at {} vs injected {injected:?}", d.tick);
    assert!(!d.sections.is_empty());
}

#[test]
fn a_repairable_desync_is_repaired_and_the_reports_are_still_written() {
    let _one = one_at_a_time();
    let (r, dir) = (fast_relay(), tmp("repair"));
    let o = NetOptions { limits: Some(ResyncLimits { min_attempt_interval: Duration::from_millis(1), bytes_per_sec: 100_000_000, burst_bytes: 100_000_000, ..ResyncLimits::default() }), ..opts(&dir) };
    let mut apps = launch(&r, 1, 0, &[], &o);
    run_to(&mut apps, 300);
    game(&apps[1]).inject(spoil());
    until(&mut apps, "the guest is repaired", |a| game(&a[1]).read(|s| s.desync.as_ref().is_some_and(|d| d.stage == DesyncStage::Repaired)) && game(&a[0]).tick() > 400);
    let now = game(&apps[0]).tick();
    run_to(&mut apps, now + 300);
    // The digests agree again from the repair on.
    let (a, b) = (game(&apps[0]).with_peer(|p| p.replay().digests.clone()), game(&apps[1]).with_peer(|p| p.replay().digests.clone()));
    let last = a.iter().rev().find(|(t, _)| b.iter().any(|(u, _)| u == t)).unwrap();
    assert_eq!(b.iter().find(|(u, _)| *u == last.0).unwrap().1, last.1);
    assert!(!reports(&dir).is_empty());
    assert!(apps[1].world().resource::<NetStatus>().0.iter().any(|l| l.contains("repaired")));
}

// ---------------------------------------------------------------------------
// The same map
// ---------------------------------------------------------------------------

#[test]
fn a_peer_with_another_map_refuses_the_start_naming_it() {
    let _one = one_at_a_time();
    let (r, dir) = (fast_relay(), tmp("map"));
    let o = opts(&dir);
    let data = net_data();
    let mut apps: Vec<App> = (0..2).map(|_| client(data.clone(), o.clone())).collect();
    // The guest's file for the same map is not the host's.
    let mut other = data.maps[0].def.as_ref().clone();
    other.bounds.1.x = sta_core::Fixed::from_int(77_777);
    apps[1].world_mut().resource_mut::<OwnMap>().0 = Some(other);
    host_through_screens(&mut apps[0], &r, "Picard", "");
    pump_apps(&mut [&mut apps[0]], "the host is in the lobby", |a| in_screen(a[0], Screen::Lobby));
    let code = session_of(&apps[0]).code().to_string();
    join_through_screens(&mut apps[1], &r, "Riker", &code, "");
    pump_apps(&mut refs(&mut apps), "seated", |a| in_screen(a[1], Screen::Lobby) && session_of(a[0]).view().seats[1].kind == crate::frontend::lobby::Kind::Human);
    ready_up(&mut apps, 1);
    pump_apps(&mut refs(&mut apps), "may launch", |a| act_entity(a[0], MpAct::Launch).is_some());
    press(&mut apps[0], MpAct::Launch);
    until(&mut apps, "the guest refuses", |a| state(&a[1]) == AppState::FrontEnd && a[1].world().resource::<Multiplayer>().notice.is_some());
    let notice = apps[1].world().resource::<Multiplayer>().notice.clone().unwrap();
    assert!(notice.contains("fx_two_bases") && notice.contains("not the host's copy"), "{notice}");
    assert!(!apps[1].world().contains_resource::<NetGame>());
    // The check on its own, with both failure shapes.
    let start = start_for(&data.maps[0].def);
    assert!(check_map(&start, &data.maps[0].def).is_ok());
    assert!(check_map(&start, &data.maps[1].def).unwrap_err().contains("fx_four_corners"));
    let mut edited = data.maps[0].def.as_ref().clone();
    edited.bounds.1.x = sta_core::Fixed::from_int(1);
    assert!(check_map(&start, &edited).unwrap_err().contains("fx_two_bases"));
}

/// A host's `Start` for `map`; only the map reference matters to `check_map`.
fn start_for(map: &sta_content::MapDef) -> sta_net::proto::Start {
    sta_net::proto::Start {
        seed: 0,
        map: sta_net::proto::MapRef { name: map.name.clone(), digest: sta_net::proto::map_digest(map).unwrap() },
        options: Default::default(),
        seats: Vec::new(),
        host: PlayerId(0),
        turn: Default::default(),
        rules: sta_net::proto::RulesIdentity::digest(net_data().content.digest()),
    }
}
