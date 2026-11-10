//! The front end flow in a bare `App` (no window, no GPU): main menu, single
//! player, game setup, start, loading, the match built from the setup, and back
//! to the front end. The screens run on the fake scene (as UI-07's own tests do),
//! the menu scene and the music through their real core systems, and the match is
//! built by a stand-in for the map view that makes the sim from the same setup.

use std::time::Duration;

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use sta_sim::Sim;
use sta_sim::command::LocalRouter;

use super::flow::*;
use super::{LocalRace, SkirmishCfg};
use crate::app::{AppState, Opponent, configure_sets};
use crate::audio::{Call, CallLog, Mixer, MusicConfig, MusicContext, NullAudio};
use crate::frontend::loading::{LoadProgress, LoadingPlugin};
use crate::frontend::scene::{MenuMap, MenuScene, MenuSceneConfig, MenuSceneCorePlugin};
use crate::frontend::screens::{Act, Screen, ScreenData, ScreenRoot, ScreensConfig, ScreensPlugin};
use crate::map::MapSeat;
use crate::sim_link::SimLink;
use crate::ui::theme::click;

/// The map view's part, as a stand-in: once the build is asked for and the menu
/// scene is gone, make the sim from the setup and link it.
fn build_match(mut commands: Commands, active: Res<ActiveMatch>, link: Option<Res<SimLink>>, scene: Option<Res<MenuScene>>, cfg: Res<SkirmishCfg>) {
    if !active.requested || link.is_some() || scene.is_some() {
        return;
    }
    let (sim, _) = Sim::from_setup_with_warnings(&active.setup);
    commands.insert_resource(SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), sta_sim::PlayerId(cfg.seat)));
    commands.insert_resource(LocalRace { name: "built".into(), interface: None, music: None });
}

fn app() -> (App, CallLog) {
    let (null, log) = NullAudio::new();
    let mut app = App::new();
    app.set_error_handler(bevy::ecs::error::panic)
        .add_plugins((MinimalPlugins, StatesPlugin))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(0.1)))
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ButtonInput<KeyCode>>()
        .insert_non_send(Mixer::new(Box::new(null)));
    configure_sets(&mut app);
    let data = ScreenData::fake();
    let scene = MenuSceneConfig::new(data.content.clone(), MenuMap::builtin(), sta_content::fake::two_bases());
    app.insert_resource(data)
        .insert_resource(ScreensConfig { own_camera: false, fit_scale: false, ..default() })
        .insert_resource(MenuSceneSource(scene))
        .insert_resource(MusicConfig { menu: vec!["audio-test:100".into()], calm: vec!["audio-test:200".into()], ..default() })
        .insert_resource(crate::clock::AnimClock::new(crate::clock::ClockMode::Wall, 0))
        .add_plugins((FlowPlugin, crate::sim_link::SimLinkPlugin, MenuSceneCorePlugin, ScreensPlugin, LoadingPlugin, crate::audio::AudioPlugin::default(), crate::input::InputPlugin, crate::ui::hud::HudPlugin))
        .add_systems(Update, build_match.run_if(resource_exists::<ActiveMatch>).run_if(resource_exists::<SkirmishCfg>));
    (app, log)
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

fn press(app: &mut App, want: Act) {
    let e = app.world_mut().query::<(Entity, &Act)>().iter(app.world()).find(|(_, a)| **a == want).unwrap_or_else(|| panic!("no {want:?} control")).0;
    click(app.world_mut(), e);
    settle(app, 3);
}

fn plays(log: &CallLog) -> Vec<String> {
    log.drain().into_iter().filter_map(|c| if let Call::Play { name, .. } = c { Some(name) } else { None }).collect()
}

#[test]
fn front_end_to_match_and_back() {
    let (mut app, log) = app();

    // Launch: the front end, main screen, the scene open, the menu playlist.
    settle(&mut app, 4);
    assert_eq!(state(&app), AppState::FrontEnd);
    assert_eq!(screen(&app), Screen::Main);
    assert!(app.world().contains_resource::<MenuScene>());
    assert_eq!(*app.world().resource::<MusicContext>(), MusicContext::Menu);
    assert_eq!(plays(&log).first().map(String::as_str), Some("audio-test:100"));

    // Single player -> game setup -> start.
    press(&mut app, Act::Goto(Screen::SinglePlayer));
    assert_eq!(screen(&app), Screen::SinglePlayer);
    press(&mut app, Act::Goto(Screen::Main));
    press(&mut app, Act::Goto(Screen::Setup)); // Instant action
    assert_eq!(screen(&app), Screen::Setup);
    let want = app.world().resource::<crate::frontend::screens::SetupDraft>().to_setup(app.world().resource::<ScreenData>()).expect("a setup");
    press(&mut app, Act::Launch);

    // Loading: the screen is up, the menu scene is closed, the match is built from the setup.
    assert_eq!(state(&app), AppState::Loading);
    assert!(app.world().resource::<LoadProgress>().is_active());
    assert!(app.world().resource::<ActiveMatch>().setup.players.len() == want.players.len());
    settle(&mut app, 2);
    assert!(!app.world().contains_resource::<MenuScene>());
    for _ in 0..200 {
        if state(&app) == AppState::InGame {
            break;
        }
        app.update();
    }
    assert_eq!(state(&app), AppState::InGame);
    settle(&mut app, 30); // every match system runs with the match's resources, no error handler panic

    // In the match: built from the setup's seats and map, menu screens gone, game music.
    let cfg = app.world().resource::<SkirmishCfg>().clone();
    assert_eq!(cfg.seat, want.players.iter().find(|p| p.control == sta_sim::setup::Control::Human).unwrap().id.0);
    assert_eq!(cfg.opponent, Opponent::Skirmish);
    assert_eq!(app.world().resource::<MapSeat>().0, Some(cfg.seat));
    let link = app.world().resource::<SimLink>();
    assert_eq!(link.curr().players.len(), want.players.len());
    assert!(app.world_mut().query::<&ScreenRoot>().iter(app.world()).next().is_none());
    assert_eq!(*app.world().resource::<MusicContext>(), MusicContext::Game);
    assert_eq!(plays(&log).last().map(String::as_str), Some("audio-test:200"), "the menu tune loops until the match starts, then the game playlist");

    // Leave: back at the front end on the main screen, the match cleaned up, the menu music again.
    app.world_mut().write_message(LeaveMatch);
    settle(&mut app, 4);
    assert_eq!(state(&app), AppState::FrontEnd);
    assert_eq!(screen(&app), Screen::Main);
    assert!(!app.world().contains_resource::<ActiveMatch>());
    assert!(!app.world().contains_resource::<SkirmishCfg>());
    assert!(!app.world().contains_resource::<LocalRace>());
    assert_eq!(app.world().resource::<MapSeat>().0, None);
    assert!(app.world().contains_resource::<MenuScene>(), "the scene is open again");
    assert!(app.world().resource::<SimLink>().player() != sta_sim::PlayerId(cfg.seat), "only the scene's link is left");
    assert_eq!(*app.world().resource::<MusicContext>(), MusicContext::Menu);
    assert_eq!(plays(&log).last().map(String::as_str), Some("audio-test:100"));

    // And a second match starts the same way.
    press(&mut app, Act::Goto(Screen::Setup));
    press(&mut app, Act::Launch);
    for _ in 0..200 {
        if state(&app) == AppState::InGame {
            break;
        }
        app.update();
    }
    assert_eq!(state(&app), AppState::InGame);
    settle(&mut app, 30);
    // F10 leaves again, and the front end is clean to start a third.
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::F10);
    settle(&mut app, 4);
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().release(KeyCode::F10);
    assert_eq!(state(&app), AppState::FrontEnd);
    settle(&mut app, 10);
}

#[test]
fn a_setup_without_a_human_does_not_start() {
    use crate::ui::notices::Notice;
    let (mut app, _) = app();
    settle(&mut app, 4);
    let setup = ScreenData::fake();
    let mut s = crate::frontend::screens::SetupDraft::new(&setup);
    for seat in &mut s.seats {
        seat.control = crate::frontend::screens::SeatControl::Computer;
    }
    let setup = s.to_setup(&setup).expect("setup");
    assert!(skirmish_of(&setup).is_err());
    app.world_mut().write_message(crate::frontend::screens::StartGame(setup));
    settle(&mut app, 3);
    assert_eq!(state(&app), AppState::FrontEnd);
    let _ = app.world().resource::<Messages<Notice>>();
}
