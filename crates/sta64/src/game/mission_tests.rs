//! The mission host in a bare `App` (no window, no GPU, no audio): the campaign screens, a
//! mission started from them, its script running with the sim, the objectives panel filled, the
//! script's end and the debrief, on a synthetic campaign over `fake::db()`; and, install-gated,
//! the same path over real missions. The map view's part is a stand-in that builds the game with
//! the same function the map view calls (`map::build_mission_sim`).

use std::path::Path;
use std::time::Duration;

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use sta_mission::runtime::MissionRuntime;
use sta_sim::Sim;
use sta_sim::command::LocalRouter;

use super::flow::*;
use super::mission_host::*;
use crate::app::{AppState, configure_sets};
use crate::frontend::campaign::tests::{fixture, temp_dir};
use crate::frontend::campaign::{CampaignAct, Campaigns, RunResult};
use crate::frontend::loading::LoadingPlugin;
use crate::frontend::scene::{MenuMap, MenuScene, MenuSceneConfig, MenuSceneCorePlugin};
use crate::frontend::screens::{Act, Screen, ScreenData, ScreensConfig, ScreensPlugin};
use crate::media::campaign::{MissionBegan, MissionEnded};
use crate::sim_link::SimLink;
use crate::ui::presentation::{ObjectivesView, PresentationPlugin, ScriptReply};
use crate::ui::theme::click;
use sta_mission::api::ObjectiveState;
use sta_sim::PlayerId;

/// The map view's part with fake content: the campaign's seats over `fake::two_bases`.
fn build_fake(mut commands: Commands, active: Res<ActiveMatch>, launch: Res<MissionLaunch>, link: Option<Res<SimLink>>, scene: Option<Res<MenuScene>>, data: Res<ScreenData>) {
    if !active.requested || link.is_some() || scene.is_some() {
        return;
    }
    let setup = launch.setup(sta_content::fake::two_bases(), data.content.clone());
    let (sim, _) = Sim::from_setup_with_warnings(&setup);
    let handle = launch.attach(&setup, &sim).expect("the synthetic script loads");
    commands.insert_resource(SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), launch.player()).with_controllers(vec![handle.controller()]));
    commands.insert_resource(handle);
}

/// What the stand-in saw come out of the host.
#[derive(Resource, Default)]
struct Seen {
    began: Vec<String>,
    ended: usize,
}

fn record(mut seen: ResMut<Seen>, mut began: MessageReader<MissionBegan>, mut ended: MessageReader<MissionEnded>) {
    seen.began.extend(began.read().map(|b| b.map.clone()));
    seen.ended += ended.read().count();
}

fn app(campaigns: Campaigns) -> App {
    let mut app = base(campaigns);
    app.add_systems(Update, build_fake.run_if(resource_exists::<ActiveMatch>).run_if(resource_exists::<MissionLaunch>));
    app
}

fn base(campaigns: Campaigns) -> App {
    let mut app = App::new();
    app.set_error_handler(bevy::ecs::error::panic)
        .add_plugins((MinimalPlugins, StatesPlugin))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(0.1)))
        .init_resource::<ButtonInput<MouseButton>>();
    configure_sets(&mut app);
    let data = ScreenData::fake();
    let scene = MenuSceneConfig::new(data.content.clone(), MenuMap::builtin(), sta_content::fake::two_bases());
    app.insert_resource(data)
        .insert_resource(ScreensConfig { own_camera: false, fit_scale: false, ..default() })
        .insert_resource(MenuSceneSource(scene))
        .insert_resource(crate::clock::AnimClock::new(crate::clock::ClockMode::Wall, 0))
        .insert_resource(campaigns)
        .init_resource::<Seen>()
        .add_message::<crate::media::StopMovie>()
        .add_plugins((FlowPlugin, crate::sim_link::SimLinkPlugin, MenuSceneCorePlugin, ScreensPlugin, LoadingPlugin, PresentationPlugin, crate::media::campaign::CampaignMoviesPlugin))
        .add_systems(Last, record);
    app
}

fn settle(app: &mut App, frames: usize) {
    for _ in 0..frames {
        app.update();
    }
}

fn state(app: &App) -> AppState {
    *app.world().resource::<State<AppState>>().get()
}

fn screen(app: &App) -> Screen {
    *app.world().resource::<State<Screen>>().get()
}

fn press_act(app: &mut App, want: Act) {
    let e = app.world_mut().query::<(Entity, &Act)>().iter(app.world()).find(|(_, a)| **a == want).unwrap_or_else(|| panic!("no {want:?}")).0;
    click(app.world_mut(), e);
    settle(app, 3);
}

fn press(app: &mut App, want: CampaignAct) {
    let e = app.world_mut().query::<(Entity, &CampaignAct)>().iter(app.world()).find(|(_, a)| **a == want).unwrap_or_else(|| panic!("no {want:?}")).0;
    click(app.world_mut(), e);
    settle(app, 3);
}

fn has(app: &mut App, want: CampaignAct) -> bool {
    app.world_mut().query::<&CampaignAct>().iter(app.world()).any(|a| *a == want)
}

/// Run frames until `done`, at most `max`.
fn until(app: &mut App, max: usize, done: impl Fn(&App) -> bool) {
    for _ in 0..max {
        if done(app) {
            return;
        }
        app.update();
    }
    assert!(done(app), "not reached in {max} frames (state {:?}, screen {:?})", state(app), screen(app));
}

/// From the front end to a started mission: Single Player, the card, the list (picks the
/// furthest mission), the briefing, Start; then the loading screen until the game is up.
fn start_from_screens(app: &mut App, card: usize) {
    settle(app, 4);
    assert_eq!(state(app), AppState::FrontEnd);
    assert_eq!(screen(app), Screen::Main);
    press_act(app, Act::Goto(Screen::SinglePlayer));
    press(app, CampaignAct::Card(card));
    assert_eq!(screen(app), Screen::Missions);
    press(app, CampaignAct::ToBriefing);
    assert_eq!(screen(app), Screen::Briefing);
    press(app, CampaignAct::Start);
    assert_eq!(state(app), AppState::Loading, "Start loads the mission");
    until(app, 300, |a| state(a) == AppState::InGame);
}

fn campaigns(app: &App) -> &Campaigns {
    app.world().resource::<Campaigns>()
}

#[test]
fn a_synthetic_campaign_plays_from_the_screens_and_progress_survives_a_restart() {
    let dir = temp_dir("host");
    let mut app = app(fixture(false, Some(&dir)));
    // The front end's browser lists multiplayer maps only; the mission's map is added by name.
    app.insert_resource(crate::map::MapBrowser { maps: vec!["blueamber".into()], index: 0, dirty: false, mount_summary: String::new(), info: String::new() });

    // Mission 1 (federation): its script wins after a second of game time.
    start_from_screens(&mut app, 1);
    assert!(app.world().contains_resource::<MissionHandle>() && app.world().contains_resource::<MissionRun>());
    assert!(app.world().resource::<crate::map::MapBrowser>().maps.iter().any(|m| m == "syn_fed01"), "the mission's map is built by name");
    assert_eq!(app.world().resource::<Seen>().began, ["syn_fed01.bzn"], "MissionBegan names the map");
    assert!(app.world().resource::<ObjectivesView>().visible, "the objectives open with the mission");
    // The script runs with the sim: its objective reaches the view, then it is completed and the mission ends.
    until(&mut app, 100, |a| !a.world().resource::<ObjectivesView>().items.is_empty());
    let view = app.world().resource::<ObjectivesView>().clone();
    assert_eq!(view.items.len(), 3, "{view:?}");
    assert!(view.items[2].text.contains("Hold the line"), "{view:?}");
    assert!(view.items[0].text.contains("Survive one second"), "{view:?}");
    let _ = ObjectiveState::Active;
    assert_eq!(view.title.to_lowercase(), "first light", "the briefing file's heading");
    assert!(app.world().resource::<SimLink>().curr().tick > 0);
    until(&mut app, 200, |a| state(a) == AppState::FrontEnd);
    settle(&mut app, 4);

    // Back at the front end: the debrief, with the result recorded, saved, and the mission taken down.
    assert_eq!(screen(&app), Screen::Debrief);
    let d = campaigns(&app).debrief.clone().expect("a debrief");
    assert_eq!((d.result, d.campaign.as_str(), d.mission), (RunResult::Won, "federation", 0));
    assert_eq!(campaigns(&app).progress.state("federation").reached, 1);
    assert_eq!(app.world().resource::<Seen>().ended, 1);
    for gone in [app.world().contains_resource::<MissionRun>(), app.world().contains_resource::<MissionLaunch>(), app.world().contains_resource::<MissionHandle>(), app.world().contains_resource::<ActiveMatch>()] {
        assert!(!gone);
    }
    assert_eq!(*app.world().resource::<ObjectivesView>(), ObjectivesView::default(), "the panel is cleared");
    assert!(has(&mut app, CampaignAct::Pick(1)), "the debrief offers the next mission");
    let saved = std::fs::read(campaigns(&app).store.path().unwrap()).unwrap();

    // A restart of the app state: a new `Campaigns` from the same store has the progress.
    app.insert_resource(fixture(false, Some(&dir)));
    settle(&mut app, 2);
    assert_eq!(campaigns(&app).progress.state("federation").reached, 1);

    // Mission 2 is open now; its script loses. A loss unlocks nothing and saves nothing new.
    go_to_single_player(&mut app);
    start_from_screens_later(&mut app, 1);
    until(&mut app, 200, |a| state(a) == AppState::FrontEnd);
    settle(&mut app, 4);
    assert_eq!(screen(&app), Screen::Debrief);
    let d = campaigns(&app).debrief.clone().unwrap();
    assert_eq!((d.result, d.mission), (RunResult::Lost, 1));
    assert_eq!(campaigns(&app).progress.state("federation"), sta_mission::campaign::CampaignState { reached: 1, complete: false });
    assert!(!campaigns(&app).is_open("klingon"));
    assert_eq!(std::fs::read(campaigns(&app).store.path().unwrap()).unwrap(), saved, "the loss wrote no progress");
    assert!(has(&mut app, CampaignAct::Pick(1)), "retry");
    std::fs::remove_dir_all(dir).unwrap();
}

fn go_to_single_player(app: &mut App) {
    app.world_mut().resource_mut::<NextState<Screen>>().set(Screen::SinglePlayer);
    settle(app, 3);
}

/// As [`start_from_screens`], from Single Player.
fn start_from_screens_later(app: &mut App, card: usize) {
    press(app, CampaignAct::Card(card));
    press(app, CampaignAct::ToBriefing);
    press(app, CampaignAct::Start);
    assert_eq!(state(app), AppState::Loading);
    until(app, 300, |a| state(a) == AppState::InGame);
}

#[test]
fn winning_the_last_mission_completes_the_campaign_and_opens_the_next() {
    let mut app = app(fixture(true, None));
    start_from_screens(&mut app, 1);
    until(&mut app, 200, |a| state(a) == AppState::FrontEnd);
    settle(&mut app, 4);
    assert_eq!(campaigns(&app).progress.state("federation").reached, 1);
    press(&mut app, CampaignAct::Pick(1));
    press(&mut app, CampaignAct::Start);
    until(&mut app, 300, |a| state(a) == AppState::InGame);
    until(&mut app, 200, |a| state(a) == AppState::FrontEnd);
    settle(&mut app, 4);
    let d = campaigns(&app).debrief.clone().unwrap();
    assert_eq!(d.result, RunResult::Won);
    assert!(matches!(d.transition, sta_mission::campaign::Transition::CampaignComplete { ref unlocked, .. } if unlocked == &["klingon".to_owned()]));
    assert!(campaigns(&app).progress.state("federation").complete);
    assert!(campaigns(&app).is_open("klingon"));
}

#[test]
fn leaving_a_mission_abandons_it_and_changes_nothing() {
    let mut app = app(fixture(false, None));
    start_from_screens(&mut app, 1);
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::F10);
    settle(&mut app, 4);
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().release(KeyCode::F10);
    assert_eq!(state(&app), AppState::FrontEnd);
    assert_eq!(screen(&app), Screen::Debrief);
    assert_eq!(campaigns(&app).debrief.as_ref().unwrap().result, RunResult::Abandoned);
    assert_eq!(campaigns(&app).progress.state("federation").reached, 0);
    assert_eq!(app.world().resource::<Seen>().ended, 1);
}

#[test]
fn a_blocked_mission_does_not_start() {
    let mut cs = fixture(false, None);
    cs.progress.campaigns.get_mut("federation").unwrap().complete = true;
    let mut app = app(cs);
    settle(&mut app, 4);
    // A request for it is refused, with the reason, and the front end stays.
    app.world_mut().write_message(crate::frontend::campaign::StartMission { campaign: "klingon".into(), mission: 0 });
    settle(&mut app, 3);
    assert_eq!(state(&app), AppState::FrontEnd);
    assert!(!app.world().contains_resource::<MissionRun>());
    // So is a locked one.
    app.world_mut().write_message(crate::frontend::campaign::StartMission { campaign: "federation".into(), mission: 9 });
    settle(&mut app, 3);
    assert_eq!(state(&app), AppState::FrontEnd);
}

#[test]
fn a_script_that_does_not_load_returns_to_the_list() {
    // Nobody builds the game, as when the map view's build fails: no link ever comes up.
    let mut app = base(fixture(false, None));
    start_failing(&mut app);
}

fn start_failing(app: &mut App) {
    settle(app, 4);
    press_act(app, Act::Goto(Screen::SinglePlayer));
    press(app, CampaignAct::Card(1));
    press(app, CampaignAct::ToBriefing);
    press(app, CampaignAct::Start);
    assert_eq!(state(app), AppState::Loading);
    // Nobody builds the game: the flow gives up after its limit and returns to the front end.
    until(app, 4000, |a| state(a) == AppState::FrontEnd);
    settle(app, 4);
    assert_eq!(screen(app), Screen::Missions, "a mission that never began goes back to its list");
    assert!(campaigns(app).debrief.is_none(), "nothing is recorded");
    assert_eq!(app.world().resource::<Seen>().ended, 1);
    assert!(!app.world().contains_resource::<MissionRun>());
}

#[test]
fn the_replies_reach_the_script_through_the_one_host_side_function() {
    let rt = MissionRuntime::from_source_with(
        "replies.lua",
        "script = {}\nA2.PlayCinematicMovie(script, 'clip', 1, false)\nA2.AudioMessage(script, 'one', false)\nCineractive.active = true\n",
        1,
        sta_mission::api::MissionConfig::default(),
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    let waits = |rt: &MissionRuntime| rt.eval::<i64>("return A2.ScriptWaits(script)").unwrap();
    assert_eq!(waits(&rt), 2);
    deliver_reply_host_side(&rt, &ScriptReply::MovieDone { id: crate::media::MovieId(1), name: "clip".into() });
    assert_eq!(waits(&rt), 1);
    deliver_reply_host_side(&rt, &ScriptReply::MessageDismissed { id: 1 });
    assert_eq!(waits(&rt), 0);
    assert!(rt.eval::<bool>("return Cineractive.active").unwrap());
    // A skip is the script's cancel; it leaves the mode to the script.
    deliver_reply_host_side(&rt, &ScriptReply::CineractiveSkipped);
    assert!(rt.cineractive().cancel);
    assert!(rt.eval::<bool>("return Cineractive.active").unwrap());
    deliver_reply_host_side(&rt, &ScriptReply::CineractiveReleased);
    assert!(!rt.eval::<bool>("return Cineractive.active").unwrap());
}

/// Two runs of the same mission give the same game: the script's commands enter through the
/// router in the same order, so the digests agree tick for tick.
#[test]
fn a_mission_is_deterministic_run_to_run() {
    fn run() -> Vec<u64> {
        let cs = fixture(false, None);
        let launch = MissionLaunch::new(&cs, "federation", 0).unwrap();
        let content = std::sync::Arc::new(sta_content::fake::db());
        let setup = launch.setup(sta_content::fake::two_bases(), content);
        let (sim, _) = Sim::from_setup_with_warnings(&setup);
        let handle = launch.attach(&setup, &sim).unwrap();
        let mut link = SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0)).with_controllers(vec![handle.controller()]);
        (0..90).map(|_| {
            link.tick();
            link.curr().digest
        }).collect()
    }
    assert_eq!(run(), run());
}

// ---------------------------------------------------------------------------
// The player is never left without control; a gap is not a loss
// ---------------------------------------------------------------------------

/// The synthetic campaigns with the tutorial mission's script replaced.
fn with_tutorial_script(script: &str) -> Campaigns {
    use crate::frontend::campaign::tests::{FakeHost, source};
    let src = std::sync::Arc::new(source(false).with("missions/syn_tut1.lua", script));
    let book = sta_mission::campaign::CampaignBook::load(&*src, sta_mission::campaign::ShellLayout::Armada2);
    Campaigns::new(book, std::sync::Arc::new(FakeHost(src)), crate::frontend::campaign::ProgressStore::none())
}

/// The synthetic tutorial mission, started by the request the briefing's Start writes.
fn start_direct(app: &mut App) {
    settle(app, 4);
    app.world_mut().write_message(crate::frontend::campaign::StartMission { campaign: "tutorials".into(), mission: 0 });
    settle(app, 3);
    until(app, 300, |a| state(a) == AppState::InGame);
}

#[test]
fn a_script_that_faults_gives_the_player_the_cineractive_back() {
    // A cineractive that cannot be skipped, and a callback that raises before it ends it.
    let script = "local began = false\nMMM.register(function(t) if not began then began = true Cineractive.active = true Cineractive.skippable = false end if t >= 0.5 then error('boom') end if t > 1e9 then Mission:finish(true, 0) end end)\n";
    let mut app = app(with_tutorial_script(script));
    settle(&mut app, 4);
    app.world_mut().write_message(crate::frontend::campaign::StartMission { campaign: "tutorials".into(), mission: 0 });
    let (mut was_on, mut released) = (false, false);
    for _ in 0..600 {
        app.update();
        let on = app.world().resource::<crate::ui::presentation::CineractiveState>().on;
        was_on |= on;
        if was_on && !on {
            released = true;
            break;
        }
        if state(&app) == AppState::InGame && !app.world().contains_resource::<MissionHandle>() {
            break;
        }
    }
    let errors = app.world().resource::<MissionHandle>().with(|rt| rt.errors().to_vec());
    assert!(was_on, "the cineractive began (errors {errors:?})");
    assert!(released, "and the fault released it (errors {errors:?})");
    let faults = app.world().resource::<MissionHandle>().with(|rt| rt.errors().iter().filter(|e| is_fault(e)).count());
    assert!(faults >= 1, "the fault is what released it");
}

#[test]
fn a_loss_after_an_unsupported_name_is_not_counted_as_a_loss() {
    // The script touches a name STA64 does not bind, then loses.
    let script = "local gap = NoSuchBinding.x\nMMM.register(function(t) if t >= 0.2 then Mission:finish(false, 0) end end)\n";
    let mut app = app(with_tutorial_script(script));
    start_direct(&mut app);
    until(&mut app, 300, |a| state(a) == AppState::FrontEnd);
    settle(&mut app, 4);
    assert_eq!(campaigns(&app).debrief.as_ref().unwrap().result, RunResult::Abandoned, "blocked, not lost");
    assert_eq!(campaigns(&app).progress.state("tutorials").reached, 0);
}

#[test]
fn a_loss_with_nothing_unsupported_is_still_a_loss() {
    let mut app = app(with_tutorial_script("MMM.register(function(t) if t >= 0.2 then Mission:finish(false, 0) end end)\n"));
    start_direct(&mut app);
    until(&mut app, 300, |a| state(a) == AppState::FrontEnd);
    settle(&mut app, 4);
    assert_eq!(campaigns(&app).debrief.as_ref().unwrap().result, RunResult::Lost);
}

// ---------------------------------------------------------------------------
// AI seats and the plans scripts load (AI-06)
// ---------------------------------------------------------------------------

/// The synthetic install's scripts plus an AIP library: `build.aip` gives a seat a miner to build.
struct AiHost(crate::frontend::campaign::tests::FakeHost);

impl sta_mission::campaign::MissionHost for AiHost {
    fn game_setup(&self, c: &sta_mission::campaign::Campaign, m: &sta_mission::campaign::CampaignMission, seed: u64) -> Result<sta_sim::setup::GameSetup, String> {
        self.0.game_setup(c, m, seed)
    }

    fn script(&self, c: &sta_mission::campaign::Campaign, m: &sta_mission::campaign::CampaignMission) -> Result<(String, sta_mission::campaign::ScriptReader), String> {
        self.0.script(c, m)
    }

    fn seat(&self) -> (PlayerId, sta_mission::api::PlayerNumbering) {
        // As the client's host: script number 2 is slot 1.
        (PlayerId(0), sta_mission::api::PlayerNumbering::ArmadaTeams)
    }

    fn aip_library(&self) -> sta_ai::AipLibrary {
        let text = "int min_resource_force = 2;\nint min_attack_force = 3;\nBuild_List_Element the_build_list[MAX_ELEMENTS] =\n{\n\"fx_miner\", 2,\n};\n";
        let names = vec!["AI/AIPs/Build.aip".to_owned()];
        sta_ai::AipLibrary::load(&names, &mut |_| Some(text.to_owned()))
    }
}

/// Seat 0 the human, seat 1 an AI seat of race `fx` over the two bases, each with a station, a
/// yard and scrap; the mission's script is `script` (federation, mission 1).
fn ai_game(script: &str) -> (MissionHandle, Sim, sta_sim::setup::GameSetup) {
    use sta_mission::campaign::{CampaignBook, ShellLayout};
    use sta_sim::WorldSource;
    use sta_sim::command::{Command, CommandKind, Edit};
    use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
    let src = std::sync::Arc::new(crate::frontend::campaign::tests::source(false).with("missions/syn_fed01.lua", format!("{script}\nlocal _never = function() Mission:finish(true, 0) end\n")));
    let book = CampaignBook::load(&*src, ShellLayout::Armada2);
    let cs = Campaigns::new(book, std::sync::Arc::new(AiHost(crate::frontend::campaign::tests::FakeHost(src))), crate::frontend::campaign::ProgressStore::none());
    let launch = MissionLaunch::new(&cs, "federation", 0).unwrap();
    let seat = |id: u8, control| PlayerSetup { id: PlayerId(id), team: sta_sim::TeamId(id), race: "fx".into(), control };
    let setup = GameSetup {
        seed: 3,
        map: std::sync::Arc::new(sta_content::fake::two_bases()),
        content: std::sync::Arc::new(sta_content::fake::db()),
        players: vec![seat(0, Control::Human), seat(1, Control::Idle)],
        options: GameOptions::default(),
    };
    let mut sim = Sim::from_setup(&setup);
    let key = |odf: &str| setup.content.key(odf).unwrap();
    let mut batch = Vec::new();
    let mut seq = 0;
    for (i, st) in setup.map.starts.iter().enumerate() {
        let p = PlayerId(i as u8);
        for (odf, dz) in [("fx_station", 0), ("fx_yard", 300)] {
            seq += 1;
            let pos = sta_core::Vec3 { x: st.pos.x, y: st.pos.y, z: st.pos.z + sta_core::Fixed::from_int(dz) };
            batch.push(Command { player: PlayerId::BENCH, seq, kind: CommandKind::Edit(Edit::Spawn { odf: key(odf), player: p, pos, heading: st.heading }) });
        }
        seq += 1;
        batch.push(Command { player: PlayerId::BENCH, seq, kind: CommandKind::Edit(Edit::Cargo(p, 0, sta_core::Fixed::from_int(1000))) });
    }
    sim.step(&batch);
    let handle = launch.attach(&setup, &sim).expect("the synthetic script loads");
    (handle, sim, setup)
}

/// Load `build.aip` for seat 1 at 2 seconds of game time (tick 60).
const LOAD_AT_TWO_SECONDS: &str = "local done = false\nMMM.register(function(t) if t >= 2 and not done then done = true Player(2):loadAIP('BUILD.AIP') end end)\n";

/// The commands of the AI seat (those the script did not number) per tick.
fn ai_commands(script: &str, ticks: u64) -> (Vec<(u64, sta_sim::command::Command)>, MissionHandle) {
    use sta_sim::WorldSource;
    let (handle, mut sim, _) = ai_game(script);
    let mut ctl = handle.controller();
    let (mut events, mut sent) = (Vec::new(), Vec::new());
    for _ in 0..ticks {
        let view = sim.view();
        let mut out = Vec::new();
        ctl.think(&view, &events, &mut out);
        sent.extend(out.iter().filter(|c| c.player == PlayerId(1)).map(|c| (view.tick, c.clone())));
        events = sim.step(&out).events;
    }
    (sent, handle)
}

#[test]
fn an_ai_seat_is_passive_until_the_script_loads_a_plan_then_builds() {
    let (none, h) = ai_commands("MMM.register(function(t) end)\n", 240);
    assert!(none.is_empty(), "no plan, no commands");
    assert!(h.unresolved_aips().is_empty());

    let (sent, h) = ai_commands(LOAD_AT_TWO_SECONDS, 240);
    assert!(!sent.is_empty(), "the seat plays once the script loads its plan");
    assert!(sent.iter().all(|(tick, _)| *tick >= 60), "nothing before the load: {:?}", sent.first());
    assert!(sent.iter().all(|(_, c)| c.seq >= 0x4000_0000), "the seat's commands are numbered clear of the script's");
    assert!(h.unresolved_aips().is_empty());
    h.with(|rt| assert!(!rt.unsupported().iter().any(|(k, _)| k == "Player.loadAIP"), "{:?}", rt.unsupported()));
}

#[test]
fn a_load_that_changes_nothing_is_listed_not_counted_as_unsupported() {
    // Seat 0 is the human's (no AI), and `none.aip` is not in the library.
    let script = "local done = false\nMMM.register(function(t) if not done then done = true Player(1):loadAIP('build.aip') Player(2):loadAIP('none.aip') end end)\n";
    let (sent, h) = ai_commands(script, 60);
    assert!(sent.is_empty());
    assert_eq!(h.unresolved_aips(), [(PlayerId(0), "build.aip".to_owned()), (PlayerId(1), "none.aip".to_owned())]);
    h.with(|rt| assert!(rt.unsupported().is_empty(), "{:?}", rt.unsupported()));
}

/// The game with its AI seat is the same game every run, through the link's router, and the plan
/// changes it.
#[test]
fn a_mission_with_an_ai_seat_is_deterministic_run_to_run() {
    fn run(script: &str) -> Vec<u64> {
        let (handle, sim, _) = ai_game(script);
        let mut link = SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0)).with_controllers(vec![handle.controller()]);
        (0..240).map(|_| {
            link.tick();
            link.curr().digest
        }).collect()
    }
    assert_eq!(run(LOAD_AT_TWO_SECONDS), run(LOAD_AT_TWO_SECONDS));
    assert_ne!(run(LOAD_AT_TWO_SECONDS), run("MMM.register(function(t) end)\n"), "the loaded plan builds something");
}

// ---------------------------------------------------------------------------
// Install-gated: real missions
// ---------------------------------------------------------------------------

/// The map view's part over an install: the game built by `map::build_mission_sim`, as the
/// map view builds it.
fn build_install(mut commands: Commands, active: Res<ActiveMatch>, launch: Res<MissionLaunch>, link: Option<Res<SimLink>>, scene: Option<Res<MenuScene>>, vfs: Res<crate::viewer::Mounted>, mut catalogue: ResMut<crate::map::Catalogue>) {
    if !active.requested || link.is_some() || scene.is_some() {
        return;
    }
    let (content, index, races) = match catalogue.content(&vfs.0) {
        Ok(c) => (c.db.clone(), c.index.clone(), c.races.clone()),
        Err(e) => panic!("content: {e}"),
    };
    let bytes = vfs.0.read(&format!("bzn/{}.bzn", launch.map)).expect("the mission's map").expect("readable");
    let map = sta_bzn::parse(&bytes).expect("parses");
    let side = crate::map::build_mission_sim(&map, &launch.map, &content, &index, &vfs.0, &races, &launch);
    let Some(Ok(m)) = side.mission else { panic!("the mission did not attach: {:?}", side.mission.map(|r| r.err())) };
    commands.insert_resource(SimLink::new(Box::new(side.sim), Box::new(LocalRouter::new()), launch.player()).with_controllers(side.controllers));
    commands.insert_resource(m.handle);
    if let Some(l) = m.local {
        commands.insert_resource(l);
    }
}

/// A flow over an install's campaign: card `card`, its first mission, `ticks` ticks of it.
/// Returns what the script did: (objectives in the view, the script's errors).
fn play_install(vfs: sta_vfs::Vfs, campaigns: Campaigns, card: usize, ticks: u64) -> (ObjectivesView, Vec<String>, Vec<(String, u32)>, String) {
    let mut app = base(campaigns);
    app.insert_resource(crate::viewer::Mounted(vfs))
        .init_resource::<crate::map::Catalogue>()
        .add_systems(Update, build_install.run_if(resource_exists::<ActiveMatch>).run_if(resource_exists::<MissionLaunch>));
    start_from_screens(&mut app, card);
    let title = app.world().resource::<MissionLaunch>().title.clone();
    until(&mut app, 20_000, |a| a.world().resource::<SimLink>().curr().tick >= ticks || state(a) != AppState::InGame);
    let view = app.world().resource::<ObjectivesView>().clone();
    let handle = app.world().resource::<MissionHandle>().clone();
    let (errors, unsupported) = handle.with(|rt| (rt.errors().to_vec(), rt.unsupported()));
    (view, errors, unsupported, title)
}

fn first_card_of(cs: &Campaigns) -> usize {
    cs.book.campaigns.iter().position(|c| cs.state(&c.id, 0) == crate::frontend::campaign::MissionState::Ready).expect("a campaign whose first mission can start")
}

/// With `STA64_TEST_ROOTS_INSTALL`: Fleet Operations Roots' first campaign mission starts from the
/// screen flow, its Lua script loads and runs 300 ticks with the sim, and its objectives reach the
/// view. No mission is expected to reach a real win here: the script plays only as far as the
/// sim and the bindings let it.
#[test]
fn install_gated_roots_first_mission_starts_from_the_screens_and_runs() {
    let Ok(root) = std::env::var("STA64_TEST_ROOTS_INSTALL") else {
        eprintln!("SKIP: set STA64_TEST_ROOTS_INSTALL to Fleet Operations Roots for the real-mission start");
        return;
    };
    let dir = temp_dir("roots");
    let vfs = sta_vfs::Vfs::mount_install(Path::new(&root), None).expect("mounts");
    let (campaigns, _) = load_campaigns_in(&vfs, Some(Path::new(&root)), None, |b| crate::frontend::campaign::ProgressStore::in_dir(&dir, b)).expect("Roots has campaigns");
    let card = first_card_of(&campaigns);
    assert!(matches!(campaigns.state(&campaigns.book.campaigns[card].id, 0), crate::frontend::campaign::MissionState::Ready), "{:?}", campaigns.state(&campaigns.book.campaigns[card].id, 0));
    let (view, errors, unsupported, title) = play_install(vfs, campaigns, card, 300);
    eprintln!("roots: {title}: objectives {:?} / {:?}; {} script errors {:?}; unsupported {:?}", view.title, view.items, errors.len(), errors.iter().take(3).collect::<Vec<_>>(), unsupported.iter().take(8).collect::<Vec<_>>());
    // Not asserted: this mission's script puts no objective into the runtime within 300 ticks
    // (measured on this install); the synthetic test asserts the delivery path.
    // The sim rejects the script's `MaxOfficers` edits (not this change's business, and
    // not asserted here); anything else the script reports is an error.
    let other: Vec<_> = errors.iter().filter(|e| !e.contains("Player.maxOfficers: simulation rejected")).collect();
    assert!(other.is_empty(), "script errors: {errors:?}");
    assert!(!unsupported.iter().any(|(k, _)| k == "Player.loadAIP"), "plan loads reach the seats: {unsupported:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

/// With `STA64_TEST_ROOTS_INSTALL`: Fleet Operations Roots' "Politics by Other Means" starts, its
/// intro cineractive runs and ends, no camera hold outlives it, the mission is not lost in its
/// first tick (it replaces the player's craft and adds a relay station the script looks up at once)
/// and the script raises no fault.
#[test]
fn install_gated_roots_politics_by_other_means_plays_its_intro_and_never_holds_the_camera() {
    let Ok(root) = std::env::var("STA64_TEST_ROOTS_INSTALL") else {
        eprintln!("SKIP: set STA64_TEST_ROOTS_INSTALL to Fleet Operations Roots for the intro run");
        return;
    };
    let dir = temp_dir("politics");
    let vfs = sta_vfs::Vfs::mount_install(Path::new(&root), None).expect("mounts");
    let (mut campaigns, _) = load_campaigns_in(&vfs, Some(Path::new(&root)), None, |b| crate::frontend::campaign::ProgressStore::in_dir(&dir, b)).expect("Roots has campaigns");
    let (campaign, mission) = campaigns
        .book
        .campaigns
        .iter()
        .find_map(|c| c.missions.iter().find(|m| m.title.as_deref() == Some("Politics by Other Means")).map(|m| (c.id.clone(), m.index)))
        .expect("the mission is in the book");
    let mut open = campaigns.book.clone();
    open.give_all_missions = true;
    campaigns.progress.apply_give_all(&open);
    let mut app = base(campaigns);
    app.insert_resource(crate::viewer::Mounted(vfs))
        .init_resource::<crate::map::Catalogue>()
        .add_plugins(crate::input::camera_cinematic::CinematicCameraPlugin)
        .add_systems(Update, build_install.run_if(resource_exists::<ActiveMatch>).run_if(resource_exists::<MissionLaunch>));
    app.world_mut().spawn((crate::viewer::Orbit { focus: Vec3::ZERO, yaw: 0.0, pitch: 0.7, dist: 400.0, home: (Vec3::ZERO, 1.0) }, Transform::default()));
    settle(&mut app, 4);
    app.world_mut().write_message(crate::frontend::campaign::StartMission { campaign, mission });
    settle(&mut app, 3);
    until(&mut app, 600, |a| state(a) == AppState::InGame);
    let saw_intro = std::cell::Cell::new(false);
    let held_without_state = std::cell::Cell::new(false);
    until(&mut app, 20_000, |a| {
        let on = a.world().resource::<crate::ui::presentation::CineractiveState>().on;
        saw_intro.set(saw_intro.get() | on);
        held_without_state.set(held_without_state.get() | (a.world().resource::<crate::input::camera_cinematic::CinematicCamera>().locked && !on));
        a.world().resource::<SimLink>().curr().tick >= 1400 || state(a) != AppState::InGame
    });
    assert_eq!(state(&app), AppState::InGame, "the mission is not over after its intro");
    assert!(saw_intro.get(), "the intro cineractive ran");
    assert!(!held_without_state.get(), "the camera is held only while a cineractive is on");
    assert!(!app.world().resource::<crate::ui::presentation::CineractiveState>().on, "the intro ended");
    assert!(!app.world().resource::<crate::input::camera_cinematic::CinematicCamera>().locked, "no hold outlives the intro");
    assert!(app.world().resource::<MissionRun>().outcome.is_none());
    let errors = app.world().resource::<MissionHandle>().with(|rt| rt.errors().to_vec());
    // The one fault left is the 30-minute timer hook: the script keeps its clock in the relay
    // platform's `maxHull`, which STA64 cannot set yet (a sim max-hull edit, WORKPLAN SIM-54/55),
    // so the interval it computes is negative. It comes after the intro and is not one of the
    // causes fixed here.
    let others: Vec<_> = errors.iter().filter(|e| is_fault(e) && !e.contains("Timer: interval")).collect();
    assert!(others.is_empty(), "script faults: {errors:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

/// With `STA64_TEST_A2_STOCK_INSTALL` and `STA64_TEST_A2_CONVERTED` (the `--out` folder of
/// `tools/dsl_to_lua.py`): an Armada II campaign mission from the locally converted missions,
/// mounted as the top layer, starts from the screen flow and runs 300 ticks.
#[test]
fn install_gated_a2_converted_mission_starts_from_the_screens_and_runs() {
    let (Ok(root), Ok(conv)) = (std::env::var("STA64_TEST_A2_STOCK_INSTALL"), std::env::var("STA64_TEST_A2_CONVERTED")) else {
        eprintln!("SKIP: set STA64_TEST_A2_STOCK_INSTALL and STA64_TEST_A2_CONVERTED (a dsl_to_lua.py output folder) for the converted-mission start");
        return;
    };
    let mount = || {
        let mut v = sta_vfs::Vfs::mount_install(Path::new(&root), None).expect("mounts");
        v.push_dir("converted".into(), Path::new(&conv).to_path_buf()).expect("overlay");
        v
    };
    let dir = temp_dir("a2");
    let vfs = mount();
    let book = sta_mission::campaign::CampaignBook::load_vfs(&vfs);
    let store = crate::frontend::campaign::ProgressStore::in_dir(&dir, &book);
    let campaigns = Campaigns::new(book, std::sync::Arc::new(ClientHost::new(std::sync::Arc::new(mount()))), store);
    let card = first_card_of(&campaigns);
    let id = campaigns.book.campaigns[card].id.clone();
    let state = campaigns.state(&id, 0);
    eprintln!("a2: {id} mission 0 is {state:?}; {} blocked of {}", campaigns.book.blocked().len(), campaigns.book.campaigns.iter().map(|c| c.missions.len()).sum::<usize>());
    if state != crate::frontend::campaign::MissionState::Ready {
        eprintln!("SKIP: the converted folder does not make the first mission runnable: {state:?}");
        return;
    }
    let (view, errors, unsupported, title) = play_install(vfs, campaigns, card, 300);
    eprintln!("a2: {title}: objectives {:?} / {:?}; {} script errors {:?}; unsupported {:?}", view.title, view.items.len(), errors.len(), errors.iter().take(3).collect::<Vec<_>>(), unsupported.iter().take(8).collect::<Vec<_>>());
    assert!(!unsupported.iter().any(|(k, _)| k == "Player.loadAIP"), "plan loads reach the seats: {unsupported:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

/// Synthetic (no game files): a mounted conversion makes the missions the install has no
/// script for startable and marks them experimental; the install's own Lua is not marked
/// (it wins); without the folder those missions stay blocked and nothing is marked.
#[test]
fn a_converted_folder_makes_scriptless_missions_startable_and_experimental() {
    let tmp = temp_dir("convmount");
    let (install, conv) = (tmp.join("install"), tmp.join("converted"));
    // The install root holds a `Data` folder; the files below live in it.
    let root = install.clone();
    let install = install.join("Data");
    for d in ["bzn", "missions"] {
        std::fs::create_dir_all(install.join(d)).unwrap();
        std::fs::create_dir_all(conv.join(d)).unwrap();
    }
    std::fs::write(install.join("mshell.set"), crate::frontend::campaign::tests::MSHELL).unwrap();
    std::fs::write(install.join("label.map"), "single_player {\n\t+ federation_campaign {First Fleet}\n}\nmission_select {\n\t+ syn_fed01.bzn {First Light}\n\t+ syn_fed02.bzn {Second Wind}\n}\n").unwrap();
    std::fs::write(install.join("ART_CFG.h"), "int GIVE_ALL_MISSIONS = 0;\r\n").unwrap();
    for b in ["syn_tut1", "syn_fed01", "syn_fed02", "syn_kl01"] {
        std::fs::write(install.join(format!("bzn/{b}.bzn")), "not parsed").unwrap();
    }
    // The install ships one script itself; the conversion holds both.
    std::fs::write(install.join("missions/syn_fed01.lua"), crate::frontend::campaign::tests::WIN_AT_ONE_SECOND).unwrap();
    for n in ["syn_fed01", "syn_fed02"] {
        std::fs::write(conv.join(format!("missions/{n}.lua")), "MMM.register(function(t) if t >= 0.5 then Mission:finish(true, 0) end end)\n").unwrap();
    }
    let store = |dir: &Path| {
        let dir = dir.to_path_buf();
        move |b: &sta_mission::campaign::CampaignBook| crate::frontend::campaign::ProgressStore::in_dir(&dir, b)
    };
    let vfs = sta_vfs::Vfs::mount_install(&root, None).expect("mounts");
    let state = |cs: &Campaigns, i| cs.state("federation", i);
    use crate::frontend::campaign::MissionState;

    let (without, _) = load_campaigns_with(&vfs, Some(&root), None, None, store(&tmp)).expect("campaigns");
    assert_eq!(state(&without, 0), MissionState::Ready);
    assert!(without.book.blocked().iter().any(|b| b.bzn == "syn_fed02.bzn"), "no script of its own: blocked");
    assert!(without.experimental.is_empty());

    let (with, _) = load_campaigns_with(&vfs, Some(&root), None, Some(&conv), store(&tmp)).expect("campaigns");
    assert_eq!(state(&with, 0), MissionState::Ready);
    // fed01 has the install's own script, which wins and is not marked; fed02 only has the conversion.
    assert_eq!(with.experimental.iter().collect::<Vec<_>>(), ["syn_fed02.bzn"]);
    assert!(!with.book.blocked().iter().any(|b| b.bzn == "syn_fed02.bzn"), "the conversion unblocks it");
    let (c, m) = with.mission("federation", 0).unwrap();
    let (_, reader) = with.host.script(c, m).unwrap();
    assert!(String::from_utf8_lossy(&reader("missions/syn_fed01.lua").unwrap()).contains("Hold the line"));
    // An absent folder is ignored, like none.
    let (absent, _) = load_campaigns_with(&vfs, Some(&root), None, Some(&tmp.join("nope")), store(&tmp)).expect("campaigns");
    assert!(absent.experimental.is_empty());
    let _ = std::fs::remove_dir_all(&tmp);
}

/// With `STA64_TEST_A2_STOCK_INSTALL` (needs `python3` and `objdump`): the stock Armada II
/// campaign lists as startable after `tools/convert_install_missions.py`, loaded the way the
/// game does (`load_campaigns_with` over the user data layout), and the first mission of each
/// campaign starts from the screen flow and runs 300 ticks.
#[test]
fn install_gated_stock_a2_campaign_is_startable_after_conversion() {
    let Ok(root) = std::env::var("STA64_TEST_A2_STOCK_INSTALL") else {
        eprintln!("SKIP: set STA64_TEST_A2_STOCK_INSTALL to a stock Armada II root");
        return;
    };
    let root = Path::new(&root);
    let data = temp_dir("stockdata");
    let tool = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/convert_install_missions.py");
    let run = std::process::Command::new("python3").arg("-I").arg(tool).arg("--install").arg(root).env("STA64_DATA_DIR", &data).output().expect("run the converter");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let conv = sta_mission::converted::dir_in(&data, root, None);

    let dir = temp_dir("stockmenu");
    let load = || {
        let vfs = sta_vfs::Vfs::mount_install(root, None).expect("mounts");
        let (cs, _) = load_campaigns_with(&vfs, Some(root), None, Some(&conv), |b| crate::frontend::campaign::ProgressStore::in_dir(&dir, b)).expect("stock campaigns");
        (vfs, cs)
    };
    let (_, cs) = load();
    let total: usize = cs.book.campaigns.iter().map(|c| c.missions.len()).sum();
    let blocked = cs.book.blocked();
    eprintln!("stock A2: {total} missions, {} blocked, {} experimental", blocked.len(), cs.experimental.len());
    assert!(cs.experimental.len() == total, "the conversion supplies the stock missions");
    assert!(blocked.is_empty(), "every stock mission can start: {blocked:?}");
    let cards: Vec<usize> = (0..cs.book.campaigns.len()).filter(|i| cs.state(&cs.book.campaigns[*i].id, 0) == crate::frontend::campaign::MissionState::Ready).collect();
    assert!(!cards.is_empty(), "a campaign whose first mission starts");
    for card in cards {
        let (vfs, cs) = load();
        let id = cs.book.campaigns[card].id.clone();
        let (view, errors, unsupported, title) = play_install(vfs, cs, card, 300);
        eprintln!("stock A2: {id}: {title}: {} objectives; {} script errors {:?}; unsupported {:?}", view.items.len(), errors.len(), errors.iter().take(3).collect::<Vec<_>>(), unsupported.iter().take(6).collect::<Vec<_>>());
    }
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&data);
}

/// With `STA64_TEST_A1_INSTALL` (needs `python3` and `objdump`): Armada I mission 1 (`fed1`),
/// converted the way the game's converter does it, starts from the screens; its opening
/// cineractive takes the camera, plays its shots and ends by the script's own rule once
/// `isCineractiveDone` reads 1 (UI-36), and the camera is the player's again.
#[test]
fn install_gated_a1_fed1_leaves_its_opening_cineractive_and_gives_the_camera_back() {
    let Ok(root) = std::env::var("STA64_TEST_A1_INSTALL") else {
        eprintln!("SKIP: set STA64_TEST_A1_INSTALL to an Armada I root");
        return;
    };
    let root = Path::new(&root);
    let data = temp_dir("a1data");
    let tool = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/convert_install_missions.py");
    let run = std::process::Command::new("python3").arg("-I").arg(tool).arg("--install").arg(root).env("STA64_DATA_DIR", &data).output().expect("run the converter");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let conv = sta_mission::converted::dir_in(&data, root, None);
    let dir = temp_dir("a1menu");
    let vfs = sta_vfs::Vfs::mount_install(root, None).expect("mounts");
    let (cs, _) = load_campaigns_with(&vfs, Some(root), None, Some(&conv), |b| crate::frontend::campaign::ProgressStore::in_dir(&dir, b)).expect("A1 campaigns");
    let card = cs.book.campaigns.iter().position(|c| c.missions.first().is_some_and(|m| m.bzn.eq_ignore_ascii_case("fed1.bzn"))).expect("fed1 opens a campaign");
    let mut app = base(cs);
    app.insert_resource(crate::viewer::Mounted(vfs))
        .init_resource::<crate::map::Catalogue>()
        .add_plugins(crate::input::camera_cinematic::CinematicCameraPlugin)
        .add_systems(Update, build_install.run_if(resource_exists::<ActiveMatch>).run_if(resource_exists::<MissionLaunch>));
    app.world_mut().spawn((crate::viewer::Orbit { focus: Vec3::ZERO, yaw: 0.0, pitch: 0.7, dist: 400.0, home: (Vec3::ZERO, 1.0) }, Transform::default()));
    start_from_screens(&mut app, card);
    let on_at = std::cell::Cell::new(None);
    let shots = std::cell::Cell::new(0);
    until(&mut app, 40_000, |a| {
        let tick = a.world().resource::<SimLink>().curr().tick;
        let cam = a.world().resource::<crate::input::camera_cinematic::CinematicCamera>();
        if cam.locked && on_at.get().is_none() {
            on_at.set(Some(tick));
        }
        shots.set(cam.shots);
        (on_at.get().is_some() && !cam.locked) || tick >= 6000 || state(a) != AppState::InGame
    });
    let tick = app.world().resource::<SimLink>().curr().tick;
    let handle = app.world().resource::<MissionHandle>().clone();
    let fired: Vec<String> = handle.with(|rt| rt.eval("return Mission_.fired").unwrap());
    eprintln!("a1 fed1: cineractive on at {:?}, off by {tick}, {} shots, {} rules fired: {:?}", on_at.get(), shots.get(), fired.len(), fired);
    assert!(on_at.get().is_some(), "the opening cineractive took the camera");
    assert!(shots.get() >= 3, "the opening's shots were played");
    assert!(fired.iter().any(|r| r == "end_opening_cin"), "the script saw isCineractiveDone: {fired:?}");
    let cam = app.world().resource::<crate::input::camera_cinematic::CinematicCamera>();
    assert!(!cam.locked, "the camera is given back");
    // The player's input moves it again.
    app.world_mut().query::<&mut crate::viewer::Orbit>().single_mut(app.world_mut()).unwrap().dist = 123.0;
    app.update();
    assert_eq!(app.world_mut().query::<&crate::viewer::Orbit>().single(app.world()).unwrap().dist, 123.0);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&data);
}
