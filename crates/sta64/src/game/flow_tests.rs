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
use super::LocalRace;
use crate::app::{AppState, configure_sets};
use crate::audio::{Call, CallLog, Mixer, MusicConfig, MusicContext, NullAudio};
use crate::frontend::loading::{LoadProgress, LoadingPlugin};
use crate::frontend::scene::{MenuMap, MenuScene, MenuSceneConfig, MenuSceneCorePlugin};
use crate::frontend::screens::{Act, Screen, ScreenData, ScreenRoot, ScreensConfig, ScreensPlugin};
use crate::map::MapSeat;
use crate::sim_link::SimLink;
use crate::ui::theme::click;

/// The map view's part, as a stand-in: once the build is asked for and the menu
/// scene is gone, make the sim from the setup and link it.
fn build_match(mut commands: Commands, active: Res<ActiveMatch>, link: Option<Res<SimLink>>, scene: Option<Res<MenuScene>>) {
    if !active.requested || link.is_some() || scene.is_some() {
        return;
    }
    let (sim, _) = Sim::from_setup_with_warnings(&active.setup);
    let link = SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), active.local.unwrap_or(sta_sim::PlayerId::BENCH));
    commands.insert_resource(if active.local.is_none() { link.with_observer() } else { link });
    commands.insert_resource(LocalRace { name: "built".into(), interface: None, interface_sprites: None, music: Vec::new() });
}

fn app() -> (App, CallLog) {
    let (null, log) = NullAudio::new();
    let mut app = App::new();
    app.set_error_handler(bevy::ecs::error::panic)
        .add_plugins((MinimalPlugins, StatesPlugin))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(0.1)))
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ButtonInput<KeyCode>>()
        .insert_non_send(Mixer::new(Box::new(null)))
        .insert_resource(crate::audio::AudioTestTones(true));
    configure_sets(&mut app);
    let data = ScreenData::fake();
    let scene = MenuSceneConfig::new(data.content.clone(), MenuMap::builtin(), sta_content::fake::two_bases());
    app.insert_resource(data)
        .insert_resource(ScreensConfig { own_camera: false, fit_scale: false, ..default() })
        .insert_resource(MenuSceneSource(scene))
        .insert_resource(MusicConfig { menu: vec!["audio-test:100".into()], calm: vec!["audio-test:200".into()], ..default() })
        .insert_resource(crate::clock::AnimClock::new(crate::clock::ClockMode::Wall, 0))
        .add_plugins((FlowPlugin, crate::sim_link::SimLinkPlugin, MenuSceneCorePlugin, ScreensPlugin, LoadingPlugin, crate::audio::AudioPlugin::default(), crate::input::InputPlugin, crate::ui::hud::HudPlugin))
        .add_systems(Update, build_match.run_if(resource_exists::<ActiveMatch>));
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
    let player = want.players.iter().find(|p| p.control == sta_sim::setup::Control::Human).unwrap().id;
    assert_eq!(app.world().resource::<ActiveMatch>().local, Some(player));
    assert_eq!(app.world().resource::<MapSeat>().0, Some(player.0));
    let link = app.world().resource::<SimLink>();
    assert_eq!(link.curr().players.len(), want.players.len());
    assert!(app.world_mut().query::<&ScreenRoot>().iter(app.world()).next().is_none());
    assert_eq!(*app.world().resource::<MusicContext>(), MusicContext::Game);
    assert_eq!(plays(&log).last().map(String::as_str), Some("audio-test:200"), "the menu tune loops until the match starts, then the game playlist");

    // Leave: back at the front end on the main screen, the match cleaned up, the menu music again.
    app.world_mut().insert_resource(crate::viewer::MapTeamColours(vec![[0, 0], [1, 1]]));
    app.world_mut().write_message(LeaveMatch);
    settle(&mut app, 4);
    assert_eq!(state(&app), AppState::FrontEnd);
    assert_eq!(screen(&app), Screen::Main);
    assert!(!app.world().contains_resource::<ActiveMatch>());
    assert!(!app.world().contains_resource::<LocalRace>());
    assert!(!app.world().contains_resource::<crate::viewer::MapTeamColours>(), "the map's team colours end with the match");
    assert_eq!(app.world().resource::<MapSeat>().0, None);
    assert!(app.world().contains_resource::<MenuScene>(), "the scene is open again");
    assert!(app.world().resource::<SimLink>().player() != player, "only the scene's link is left");
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
fn menu_screens_follow_authored_views_and_setup_subscreens_inherit_setup() {
    use crate::frontend::campaign::{CampaignAct, tests::fixture};

    fn campaign_press(app: &mut App, action: CampaignAct) {
        let world = app.world_mut();
        let entity = world.query::<(Entity, &CampaignAct)>().iter(world)
            .find(|(_, act)| **act == action).unwrap().0;
        click(world, entity);
        settle(app, 3);
    }

    let mut content = sta_content::ContentDb::builder();
    for odf in ["fscout", "fgalaxy", "fbase"] {
        let mut unit = sta_content::UnitDef::new(odf, "craft");
        unit.hull = sta_sim::Fixed::from_int(100);
        content = content.unit(unit);
    }
    let content = std::sync::Arc::new(content.build().unwrap());
    let mut vfs = sta_vfs::Vfs::default();
    vfs.push_dir("showcase".into(), concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/menu-showcase").into()).unwrap();
    let (mut app, _) = app();
    let map = MenuMap::from_vfs(&vfs, &content);
    app.insert_resource(MenuSceneSource(MenuSceneConfig::new(content, map, sta_content::MapDef::empty())))
        .insert_resource(fixture(false, None));

    settle(&mut app, 4);
    assert_eq!(screen(&app), Screen::Main);
    assert_eq!(app.world().resource::<MenuScene>().view(), "opening");

    press(&mut app, Act::Goto(Screen::SinglePlayer));
    assert_eq!(app.world().resource::<MenuScene>().view(), "fleet");
    campaign_press(&mut app, CampaignAct::Card(1));
    assert_eq!(screen(&app), Screen::Missions);
    assert_eq!(app.world().resource::<MenuScene>().view(), "campaign", "the Federation binding overrides the mission list's fleet view");
    campaign_press(&mut app, CampaignAct::ToBriefing);
    assert_eq!(screen(&app), Screen::Briefing);
    assert_eq!(app.world().resource::<MenuScene>().view(), "campaign");
    campaign_press(&mut app, CampaignAct::ToMissions);
    campaign_press(&mut app, CampaignAct::ToSinglePlayer);
    press(&mut app, Act::Goto(Screen::Main));
    assert_eq!(app.world().resource::<MenuScene>().view(), "opening");

    press(&mut app, Act::Goto(Screen::Setup)); // Instant action
    assert_eq!(app.world().resource::<MenuScene>().view(), "planning");
    press(&mut app, Act::Goto(Screen::Advanced));
    assert_eq!(screen(&app), Screen::Advanced);
    assert_eq!(app.world().resource::<MenuScene>().view(), "planning");
    press(&mut app, Act::CloseAdvanced(false));
    press(&mut app, Act::PickMap);
    assert_eq!(screen(&app), Screen::MapSelect);
    assert_eq!(app.world().resource::<MenuScene>().view(), "planning");
    let marks = app.world().resource::<crate::bench::marks::RenderMarks>().lines();
    assert_eq!(marks.iter().filter(|mark| mark.starts_with("render.menu.view view=planning ")).count(), 1);
    press(&mut app, Act::Goto(Screen::Setup));

    press(&mut app, Act::Goto(Screen::Main));
    assert_eq!(app.world().resource::<MenuScene>().view(), "opening");
    press(&mut app, Act::PickMod);
    assert_eq!(screen(&app), Screen::ModSelect);
    assert_eq!(app.world().resource::<MenuScene>().view(), "dock");
    press(&mut app, Act::Goto(Screen::Main));
    assert_eq!(screen(&app), Screen::Main);
    assert_eq!(app.world().resource::<MenuScene>().view(), "opening");
}

#[test]
fn instant_action_applies_the_chosen_palette_and_restores_the_mount() {
    use crate::viewer::{TeamColours, owner_hull_colour, owner_mark_colour};
    use sta_sim::PlayerId;

    let (mut app, _) = app();
    let original = app.world().resource::<ScreenData>().colours.clone();
    let first = original.owner_colour(PlayerId(0));
    let second = original.owner_colour(PlayerId(1));
    app.insert_resource(original);
    settle(&mut app, 4);
    press(&mut app, Act::Goto(Screen::Setup));
    press(&mut app, Act::PickTeamColour(1));
    press(&mut app, Act::Launch);
    settle(&mut app, 50);
    assert_eq!(state(&app), AppState::InGame);
    let palette = app.world().resource::<TeamColours>();
    assert_eq!(owner_hull_colour(Some(palette), None, PlayerId(0)), second);
    assert_eq!(owner_mark_colour(Some(palette), None, PlayerId(1)), first);
    let active = app.world().resource::<ActiveMatch>();
    assert_ne!(active.setup.players[0].team, active.setup.players[1].team);
    app.world_mut().write_message(LeaveMatch);
    settle(&mut app, 4);
    let restored = app.world().resource::<TeamColours>();
    assert_eq!(restored.owner_colour(PlayerId(0)), first);
    assert_eq!(restored.owner_colour(PlayerId(1)), second);
}

#[test]
fn observer_starts_without_taking_any_of_the_four_map_seats() {
    let (mut app, _) = app();
    settle(&mut app, 4);
    let data = ScreenData::fake();
    let mut draft = crate::frontend::screens::SetupDraft::new(&data);
    draft.set_map(&data, 1);
    draft.set_observing(true);
    let setup = draft.to_setup(&data).expect("observer setup");
    assert_eq!(setup.players.len(), 4);
    assert!(setup.players.iter().all(|p| matches!(p.control, sta_sim::setup::Control::Ai(_))));
    app.world_mut().write_message(crate::frontend::screens::StartGame(setup, crate::clock::MatchSpeed::Normal, None, Default::default()));
    for _ in 0..200 {
        app.update();
        if state(&app) == AppState::InGame {
            break;
        }
    }
    assert_eq!(state(&app), AppState::InGame);
    assert_eq!(app.world().resource::<ActiveMatch>().local, None);
    assert_eq!(app.world().resource::<MapSeat>().0, None);
    let link = app.world().resource::<SimLink>();
    assert!(link.is_observer());
    assert_eq!(link.curr().players.len(), 4);
}

/// Every request that reached the mixer's queue, for a test to read.
#[derive(Resource, Default)]
struct Requested(Vec<(String, crate::sim_link::Bus)>);

fn record(mut msgs: MessageReader<crate::sim_link::PlaySound>, mut out: ResMut<Requested>) {
    out.0.extend(msgs.read().map(|m| (m.name.clone(), m.bus)));
}

/// The front end's scene links a sim of ships for show. Selecting one (a click that
/// landed on a menu button over the backdrop) or ordering it must not make a unit
/// speak: no voice is requested outside a match.
#[test]
fn the_front_end_never_plays_unit_voices() {
    use crate::sim_link::{Bus, OrderIssued, OrderKind, Selection, SelectionChanged};
    let (mut app, _) = app();
    app.init_resource::<Requested>().add_systems(Update, record);
    settle(&mut app, 4);
    assert!(app.world().contains_resource::<MenuScene>());
    let ships: Vec<sta_sim::EntityId> = app.world().resource::<SimLink>().curr().units.iter().map(|u| u.id).collect();
    assert!(!ships.is_empty());
    app.world_mut().resource_mut::<Selection>().0 = ships.clone();
    app.world_mut().write_message(SelectionChanged);
    app.world_mut().write_message(OrderIssued { units: ships, kind: OrderKind::Move });
    settle(&mut app, 3);
    let voices: Vec<_> = app.world().resource::<Requested>().0.iter().filter(|(_, b)| *b == Bus::Voice).collect();
    assert!(voices.is_empty(), "voices in the front end: {voices:?}");
}

/// Menu buttons click with the shell sound (`cfgSOUND_SHELL_BUTTON_CLICK`), the match's HUD with the HUD one.
#[test]
fn the_context_follows_the_app_state() {
    use crate::audio::{UiContext, UiSound, UiSounds};
    let (mut app, _) = app();
    settle(&mut app, 4);
    assert_eq!(state(&app), AppState::FrontEnd);
    let sounds = app.world().resource::<UiSounds>().clone();
    let ctx = *app.world().resource::<UiContext>();
    assert_eq!(ctx, UiContext::Shell);
    assert_eq!(sounds.name(UiSound::Click, ctx), sounds.shell_click);
    press(&mut app, Act::Goto(Screen::SinglePlayer));
    press(&mut app, Act::Goto(Screen::Main));
    press(&mut app, Act::Goto(Screen::Setup));
    press(&mut app, Act::Launch);
    for _ in 0..200 {
        if state(&app) == AppState::InGame {
            break;
        }
        app.update();
    }
    settle(&mut app, 3);
    let ctx = *app.world().resource::<UiContext>();
    assert_eq!(ctx, UiContext::Hud);
    assert_eq!(sounds.name(UiSound::Click, ctx), sounds.hud_click);
}

/// Mod switches (campaign mount, campaign-less mount, and back) never leave a system without a
/// resource it reads every frame: the media tracker reads the movie catalogue unconditionally,
/// and remounting to a campaign-less mod (Fleet Ops 4.0) used to remove it.
#[test]
fn remounting_between_campaign_and_campaign_less_content_keeps_the_resources_systems_read() {
    use crate::frontend::campaign::Campaigns;
    use crate::media::campaign::{CampaignMoviesPlugin, MissionMovies};
    use crate::media::mission::Requested;
    use crate::media::{MovieFinished, StopMovie};

    fn apply(app: &mut App, loaded: Option<(Campaigns, MissionMovies)>) {
        let mut commands = app.world_mut().commands();
        apply_campaigns(&mut commands, loaded);
        app.world_mut().flush();
        for _ in 0..3 {
            app.update();
        }
    }
    fn with_campaigns() -> Option<(Campaigns, MissionMovies)> {
        let cs = crate::frontend::campaign::tests::fixture(false, None);
        let movies = MissionMovies::from_book_with(&cs.book, |_| None);
        Some((cs, movies))
    }

    let mut app = App::new();
    app.set_error_handler(bevy::ecs::error::panic)
        .add_plugins(MinimalPlugins)
        .add_message::<Requested>()
        .add_message::<MovieFinished>()
        .add_message::<StopMovie>()
        .add_plugins(CampaignMoviesPlugin);
    configure_sets(&mut app);
    app.update();
    // Campaigns -> none -> campaigns -> none -> none (a mod with no movies after one with none).
    apply(&mut app, with_campaigns());
    assert!(app.world().contains_resource::<Campaigns>());
    assert!(!app.world().resource::<MissionMovies>().missions.is_empty());
    apply(&mut app, None);
    assert!(!app.world().contains_resource::<Campaigns>());
    assert!(app.world().resource::<MissionMovies>().missions.is_empty(), "the old mod's movies do not survive");
    apply(&mut app, with_campaigns());
    assert!(app.world().contains_resource::<Campaigns>());
    apply(&mut app, None);
    apply(&mut app, None);
    assert!(app.world().contains_resource::<MissionMovies>());
}

#[test]
fn a_failed_remount_keeps_the_active_mod_identity() {
    use crate::frontend::screens::{ActiveMod, ModChosen, SelectedMod};
    let (mut app, _) = app();
    let root = std::env::temp_dir().join(format!("sta64_absent_mod_install_{}", std::process::id()));
    assert!(!root.exists());
    let vfs = sta_vfs::Vfs::default();
    let browser = crate::map::browser(&vfs, None, String::new());
    app.insert_resource(crate::viewer::Mounted(vfs))
        .insert_resource(crate::map::Catalogue::default())
        .insert_resource(browser)
        .insert_resource(FlowConfig { install_root: Some(root) })
        .insert_resource(ActiveMod(Some("fx_balance".into())))
        .insert_resource(SelectedMod(Some("missing".into())));
    settle(&mut app, 3);
    app.world_mut().write_message(ModChosen(Some("missing".into())));
    settle(&mut app, 3);
    assert_eq!(app.world().resource::<ActiveMod>().0.as_deref(), Some("fx_balance"));
    assert_eq!(app.world().resource::<SelectedMod>().0.as_deref(), Some("missing"));
}

#[test]
fn a_successful_mod_switch_rebuilds_the_menu_scene_and_switches_back() {
    use crate::frontend::scene::{MenuCamera, MenuScenePlugin};
    use crate::frontend::screens::{ActiveMod, ModChosen, SelectedMod};
    use crate::sim_link::SimEntities;
    use crate::viewer::Mounted;

    fn camera(app: &mut App) -> Entity {
        let world = app.world_mut();
        world.query_filtered::<Entity, With<MenuCamera>>().single(world).unwrap()
    }

    fn ship(app: &App) -> (Entity, sta_sim::Fixed) {
        let world = app.world();
        let link = world.resource::<SimLink>();
        let [unit] = link.curr().units.as_slice() else { panic!("expected one scene ship") };
        (world.resource::<SimEntities>().0[&unit.id], unit.pos.x)
    }

    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("sta64_menu_mod_switch_{}_{nonce}", std::process::id()));
    let data = root.join("Data");
    let mod_dir = data.join("Mods/Scene Mod");
    std::fs::create_dir_all(data.join("odf/ships")).unwrap();
    std::fs::create_dir_all(data.join("Menu")).unwrap();
    std::fs::create_dir_all(mod_dir.join("RefitScene/Menu")).unwrap();
    std::fs::write(
        data.join("odf/ships/fcruiser.odf"),
        "[GameObjectClass]\nclassLabel = \"craft\"\nunitName = \"Regression Cruiser\"\nrace = \"unitedfederation\"\nmaxHealth = 4000\nmaxSpeed = 140\n",
    )
    .unwrap();
    std::fs::write(data.join("Menu/map.ini"), "[View.base_view]\nEye=0,0,400\n[Ship.base_ship]\nOdf=fcruiser\nAt=10,0,0\n").unwrap();
    std::fs::write(mod_dir.join("info.ini"), "[Mod]\nName=Scene Mod\n").unwrap();
    std::fs::write(
        mod_dir.join("RefitScene/Menu/map.ini"),
        "[View.mod_view]\nEye=0,0,800\n[Ship.mod_ship]\nOdf=fcruiser\nAt=900,0,0\n",
    )
    .unwrap();

    let (mut app, _) = app();
    let vfs = sta_vfs::Vfs::mount_install(&root, None).expect("synthetic base install mounts");
    let mut catalogue = crate::map::Catalogue::default();
    let base = front_end_data(&vfs, &mut catalogue, Some(&root)).unwrap();
    let browser = crate::map::browser(&vfs, None, String::new());
    app.insert_resource(Mounted(vfs))
        .insert_resource(catalogue)
        .insert_resource(base.screens)
        .insert_resource(MenuSceneSource(base.scene))
        .insert_resource(browser)
        .insert_resource(FlowConfig { install_root: Some(root.clone()) })
        .add_plugins(AssetPlugin::default())
        .init_asset::<Mesh>()
        .init_asset::<Image>()
        .init_asset::<StandardMaterial>()
        .init_resource::<GlobalAmbientLight>()
        .add_plugins(MenuScenePlugin);
    settle(&mut app, 4);
    assert_eq!(state(&app), AppState::FrontEnd);
    assert_eq!(app.world().resource::<MenuScene>().view(), "base_view");
    let base_camera = camera(&mut app);
    let (base_ship, base_x) = ship(&app);
    assert_eq!(base_x, sta_sim::Fixed::from_int(10));

    app.world_mut().write_message(ModChosen(Some("Scene Mod".into())));
    settle(&mut app, 4);
    assert_eq!(state(&app), AppState::FrontEnd, "switching mods does not leave the front end");
    assert_eq!(app.world().resource::<ActiveMod>().0.as_deref(), Some("Scene Mod"));
    assert_eq!(app.world().resource::<SelectedMod>().0.as_deref(), Some("Scene Mod"));
    assert_eq!(app.world().resource::<MenuScene>().view(), "mod_view", "the mod's RefitScene menu map opens");
    let mod_camera = camera(&mut app);
    let (mod_ship, mod_x) = ship(&app);
    assert_eq!(mod_x, sta_sim::Fixed::from_int(900));
    assert!(app.world().get_entity(base_camera).is_err());
    assert!(app.world().get_entity(base_ship).is_err());

    app.world_mut().write_message(ModChosen(None));
    settle(&mut app, 4);
    assert_eq!(state(&app), AppState::FrontEnd, "switching back also stays in the front end");
    assert_eq!(app.world().resource::<ActiveMod>().0, None);
    assert_eq!(app.world().resource::<SelectedMod>().0, None);
    assert_eq!(app.world().resource::<MenuScene>().view(), "base_view", "the base install's direct menu map is restored");
    camera(&mut app);
    assert_eq!(ship(&app).1, base_x);
    assert!(app.world().get_entity(mod_camera).is_err());
    assert!(app.world().get_entity(mod_ship).is_err());

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn front_end_map_names_resolve_localized_keys_and_keep_name_fallbacks() {
    use sta_bzn::{Map, StartLoc};

    fn map(description: &str, file_name: &str) -> Map {
        Map {
            version: 3013,
            binary: true,
            mission_type: "mp".into(),
            description: description.into(),
            file_name: file_name.into(),
            start_count: 2,
            start_locs: vec![
                StartLoc { kind: "empty".into(), team: -1, ..default() },
                StartLoc { pos: [10.0, 0.0, 0.0], kind: "player".into(), team: 1, ..default() },
                StartLoc { pos: [-10.0, 0.0, 0.0], kind: "player".into(), team: 2, ..default() },
            ],
            ..default()
        }
    }

    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("sta64_localized_map_names_{}_{nonce}", std::process::id()));
    let data = root.join("Data");
    let child = data.join("Mods/Child");
    std::fs::create_dir_all(data.join("bzn")).unwrap();
    std::fs::create_dir_all(&child).unwrap();
    std::fs::write(data.join("Dynamic_Localized_Strings.h"),
        "\"EXPLICIT_MAP_KEY\", \"Base explicit title\", \"CHILD_OVERRIDE_MAP_NAME\", \"Base derived title\", \"EMPTY_LOCALIZED_MAP_NAME\", \"\"").unwrap();
    std::fs::write(child.join("info.ini"), "[Mod]\nName=Child\n").unwrap();
    std::fs::write(child.join("Dynamic_Localized_Strings.h"),
        "\"EXPLICIT_MAP_KEY\", \"Child explicit title\"").unwrap();

    for (stem, description, file_name) in [
        ("explicit", "EXPLICIT_MAP_KEY", "explicit.bzn"),
        ("child_override", "A literal description", "child_override.bzn"),
        ("literal", "A custom literal title", "literal.bzn"),
        ("empty_localized", "Fallback literal title", "empty_localized.bzn"),
        ("bare", "", "bare.bzn"),
    ] {
        let bytes = sta_bzn::write::write(&map(description, file_name)).unwrap();
        std::fs::write(data.join(format!("bzn/{stem}.bzn")), bytes).unwrap();
    }

    let vfs = sta_vfs::Vfs::mount_install(&root, Some("Child")).expect("synthetic child install mounts");
    let mut catalogue = crate::map::Catalogue::default();
    let front_end = front_end_data(&vfs, &mut catalogue, Some(&root)).unwrap();
    let titles: Vec<_> = front_end.screens.maps.iter().map(|entry| (entry.def.name.as_str(), entry.title())).collect();
    assert_eq!(titles, [
        ("bare", "bare"),
        ("child_override", "Base derived title"),
        ("empty_localized", "Fallback literal title"),
        ("explicit", "Child explicit title"),
        ("literal", "A custom literal title"),
    ]);

    std::fs::remove_dir_all(root).unwrap();
}

/// A remount drops what was cached by name from the previous mount: the texture cache keeps misses
/// (`None`), so a texture only the new mod has (`PA_ECFR0` of STA2 Classic) stayed "not found".
#[test]
fn a_remount_invalidates_the_name_caches() {
    use crate::render::textures::TextureCache;
    use crate::render::units::UnitModels;
    use crate::viewer::Mounted;
    let (mut app, _) = app();
    app.insert_resource(Mounted(sta_vfs::Vfs::default()))
        .init_resource::<TextureCache>()
        .init_resource::<UnitModels>()
        .insert_resource(crate::render::sprites::SpriteTables(std::sync::Arc::new(sta_assets::load_sprite_table(&sta_vfs::Vfs::default()))));
    settle(&mut app, 3);
    app.world_mut().resource_mut::<TextureCache>().map.insert("pa_ecfr0".into(), None);
    settle(&mut app, 2);
    assert!(app.world().resource::<TextureCache>().map.contains_key("pa_ecfr0"), "an unchanged mount keeps its cache");
    // What `remount` does last.
    app.world_mut().resource_mut::<Mounted>().0 = sta_vfs::Vfs::default();
    settle(&mut app, 2);
    assert!(app.world().resource::<TextureCache>().map.is_empty(), "the old mount's misses do not survive");
}
