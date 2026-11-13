//! The in-game menu in a bare `App` (no window or GPU unless a layout test makes a window, no audio
//! device): open and close, the pause, the options panel's settings, Exit and what it leaves behind,
//! and the layout in every window shape and UI scale. The mission abandon is in
//! `game::mission_tests`.

use std::collections::BTreeMap;
use std::time::Duration;

use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use bevy::window::{PrimaryWindow, WindowResolution};
use sta_sim::Sim;
use sta_sim::command::LocalRouter;

use super::*;
use crate::app::configure_sets;
use crate::audio::{Mixer, MusicConfig, NullAudio};
use crate::frontend::loading::LoadingPlugin;
use crate::frontend::scene::{MenuMap, MenuScene, MenuSceneConfig, MenuSceneCorePlugin};
use crate::frontend::screens::{Act, Screen, ScreenData, ScreensConfig, ScreensPlugin, SettingKey};
use crate::game::flow::{ActiveMatch, FlowPlugin, MenuSceneSource};
use crate::game::{LocalRace, SkirmishCfg};
use crate::ui::theme::click;

/// The map view's part, as in `game::flow_tests`: make the sim from the setup once the scene is gone.
fn build_match(mut commands: Commands, active: Res<ActiveMatch>, link: Option<Res<SimLink>>, scene: Option<Res<MenuScene>>, cfg: Res<SkirmishCfg>) {
    if !active.requested || link.is_some() || scene.is_some() {
        return;
    }
    let (sim, _) = Sim::from_setup_with_warnings(&active.setup);
    commands.insert_resource(SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), sta_sim::PlayerId(cfg.seat)));
    commands.insert_resource(LocalRace { name: "built".into(), interface: None, interface_sprites: None, music: None });
}

fn app() -> App {
    let (null, _log) = NullAudio::new();
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
        .insert_resource(AnimClock::new(crate::clock::ClockMode::Wall, 0))
        .init_resource::<crate::render::particles::Particles>()
        .add_plugins((
            FlowPlugin,
            crate::sim_link::SimLinkPlugin,
            MenuSceneCorePlugin,
            ScreensPlugin,
            LoadingPlugin,
            crate::audio::AudioPlugin::default(),
            crate::input::InputPlugin,
            crate::ui::hud::HudPlugin,
            InGameMenuPlugin,
        ))
        .add_systems(OnEnter(AppState::FrontEnd), crate::map::clear_match)
        .add_systems(Update, build_match.run_if(resource_exists::<ActiveMatch>).run_if(resource_exists::<SkirmishCfg>));
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

fn until(app: &mut App, max: usize, done: impl Fn(&App) -> bool) {
    for _ in 0..max {
        if done(app) {
            return;
        }
        app.update();
    }
    panic!("not reached in {max} frames (state {:?})", state(app));
}

fn press_act(app: &mut App, want: Act) {
    let e = app.world_mut().query::<(Entity, &Act)>().iter(app.world()).find(|(_, a)| **a == want).unwrap_or_else(|| panic!("no {want:?}")).0;
    click(app.world_mut(), e);
    settle(app, 3);
}

/// Instant Action from the front end's screens to a running match.
fn start_match(app: &mut App) {
    settle(app, 4);
    press_act(app, Act::Goto(Screen::Setup));
    press_act(app, Act::Launch);
    until(app, 300, |a| state(a) == AppState::InGame);
    settle(app, 3);
}

fn key(app: &mut App, k: KeyCode) {
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(k);
    // One frame: nothing in this bare app clears `just_pressed`, so a second would be a second press.
    settle(app, 1);
    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
    keys.release(k);
    keys.clear();
    settle(app, 1);
}

fn menu(app: &App) -> InGameMenu {
    *app.world().resource::<InGameMenu>()
}

fn button(app: &mut App, act: MenuAct) {
    let e = app.world_mut().query::<(Entity, &MenuAct)>().iter(app.world()).find(|(_, a)| **a == act).unwrap_or_else(|| panic!("no {act:?} button")).0;
    click(app.world_mut(), e);
    settle(app, 3);
}

fn tick_digest(app: &App) -> (u64, u64) {
    let link = app.world().resource::<SimLink>();
    (link.curr().tick, link.curr().digest)
}

/// Every (tick, digest) the match shows for `frames` frames.
fn run_recording(app: &mut App, frames: usize, into: &mut BTreeMap<u64, u64>) {
    for _ in 0..frames {
        app.update();
        let (t, d) = tick_digest(app);
        into.insert(t, d);
    }
}

fn menu_roots(app: &mut App) -> usize {
    app.world_mut().query::<&MenuRoot>().iter(app.world()).count()
}

#[test]
fn escape_opens_and_closes_the_menu_and_steps_back_one_page() {
    let mut app = app();
    assert!(!menu(&app).open, "closed before a match");
    start_match(&mut app);
    assert!(!menu(&app).open);
    key(&mut app, KeyCode::Escape);
    assert_eq!(menu(&app), InGameMenu { open: true, page: MenuPage::Main });
    assert_eq!(menu_roots(&mut app), 1);
    button(&mut app, MenuAct::Options);
    assert_eq!(menu(&app).page, MenuPage::Options);
    key(&mut app, KeyCode::Escape);
    assert_eq!(menu(&app), InGameMenu { open: true, page: MenuPage::Main }, "Esc goes back a page");
    key(&mut app, KeyCode::Escape);
    assert!(!menu(&app).open);
    assert_eq!(menu_roots(&mut app), 0);
    // Resume closes it too, and the open request is the HUD button's way in.
    app.world_mut().write_message(OpenInGameMenu);
    settle(&mut app, 2);
    assert!(menu(&app).open);
    button(&mut app, MenuAct::Resume);
    assert!(!menu(&app).open);
    assert_eq!(state(&app), AppState::InGame);
}

#[test]
fn the_menu_is_not_opened_in_the_front_end_or_by_a_rebound_key() {
    let mut app = app();
    settle(&mut app, 4);
    key(&mut app, KeyCode::Escape);
    assert!(!menu(&app).open, "the front end has its own screens");
    start_match(&mut app);
    app.world_mut().resource_mut::<KeyBindings>().rebind(Action::GameMenu, KeyCode::F9);
    key(&mut app, KeyCode::Escape);
    assert!(!menu(&app).open, "Esc is no longer the key");
    key(&mut app, KeyCode::F9);
    assert!(menu(&app).open);
}

#[test]
fn single_player_pauses_and_resumes_on_the_same_digests() {
    let mut app = app();
    start_match(&mut app);
    // A reference run of the same match, never interrupted.
    let mut reference = BTreeMap::new();
    let mut other = self::app();
    start_match(&mut other);
    run_recording(&mut other, 40, &mut reference);

    let mut seen = BTreeMap::new();
    run_recording(&mut app, 10, &mut seen);
    key(&mut app, KeyCode::Escape);
    assert!(app.world().resource::<AnimClock>().paused());
    let held = tick_digest(&app);
    settle(&mut app, 25);
    assert_eq!(tick_digest(&app), held, "no tick while the menu is open");
    button(&mut app, MenuAct::Resume);
    assert!(!app.world().resource::<AnimClock>().paused());
    run_recording(&mut app, 30, &mut seen);
    assert!(tick_digest(&app).0 > held.0, "the sim runs again");
    // Ticks of the two runs line up by number; every one both saw has the same digest.
    let both: Vec<_> = seen.iter().filter(|(t, _)| reference.contains_key(*t)).collect();
    assert!(both.len() > 20, "{} common ticks", both.len());
    for (t, d) in both {
        assert_eq!(reference[t], *d, "tick {t}");
    }
}

#[test]
fn a_networked_match_is_never_paused() {
    let mut app = app();
    start_match(&mut app);
    app.insert_resource(NetworkedMatch);
    let before = tick_digest(&app).0;
    key(&mut app, KeyCode::Escape);
    assert!(menu(&app).open);
    assert!(!app.world().resource::<AnimClock>().paused());
    settle(&mut app, 10);
    assert!(tick_digest(&app).0 > before, "lockstep goes on behind the menu");
}

#[test]
fn the_options_panel_changes_a_volume_and_the_settings_store_saves_it() {
    let dir = crate::frontend::campaign::tests::temp_dir("ingame-menu-settings");
    let path = dir.join(crate::ui::settings::FILE_NAME);
    let mut app = app();
    app.add_plugins(crate::ui::settings::SettingsPlugin { path: Some(path.clone()), install_root: None, lcars_forced: false, intro_forced: false });
    app.world_mut().resource_mut::<crate::ui::settings::SettingsStore>().delay = 0.0;
    app.init_resource::<crate::audio::Volumes>();
    start_match(&mut app);
    key(&mut app, KeyCode::Escape);
    button(&mut app, MenuAct::Options);
    let slider = app.world_mut().query::<(Entity, &crate::frontend::screens::view::SettingSlider)>().iter(app.world()).find(|(_, k)| k.0 == SettingKey::MusicVolume).map(|(e, _)| e).expect("the panel has the music slider");
    app.world_mut().write_message(crate::ui::theme::SliderChanged { entity: slider, value: 12.0 });
    settle(&mut app, 4);
    assert_eq!(app.world().resource::<Settings>().music, 12.0);
    assert!((app.world().resource::<crate::audio::Volumes>().music - 0.12).abs() < 1e-6, "the slider reaches the mixer's volume");
    let text = std::fs::read_to_string(&path).expect("the settings file was written");
    let (stored, warnings) = crate::ui::settings::Stored::parse(&text);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(stored.settings.music, 12.0, "{text}");
    assert_eq!(stored.bindings.keys(Action::GameMenu), [KeyCode::Escape]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Match-scoped things a test can plant: a map entity, a unit entity and a sim id's entity.
fn plant(app: &mut App) {
    let e = app.world_mut().spawn(crate::map::MatchEntity).id();
    let u = app.world_mut().spawn(crate::render::units::FromView).id();
    app.world_mut().resource_mut::<crate::sim_link::SimEntities>().0.insert(sta_sim::EntityId(9999), u);
    let _ = e;
    app.world_mut().resource_mut::<crate::input::select::ControlGroups>().groups[0] = vec![sta_sim::EntityId(1)];
    app.world_mut().resource_mut::<crate::input::orders::OrderTargets>().0.insert(sta_sim::EntityId(1), sta_sim::Vec3::from_f32([0.0; 3]));
}

fn match_scoped(app: &mut App) -> usize {
    let w = app.world_mut();
    w.query::<&crate::map::MatchEntity>().iter(w).count()
        + w.query::<&crate::render::units::FromView>().iter(w).filter(|_| w.get_resource::<MenuScene>().is_none()).count()
        + w.query::<&MenuRoot>().iter(w).count()
}

#[test]
fn exit_to_the_main_menu_leaves_nothing_of_the_match_and_the_next_match_is_a_fresh_one() {
    let mut app = app();
    start_match(&mut app);
    let mut first = BTreeMap::new();
    run_recording(&mut app, 30, &mut first);
    plant(&mut app);
    assert!(match_scoped(&mut app) >= 2);
    key(&mut app, KeyCode::Escape);
    button(&mut app, MenuAct::AskExit);
    assert_eq!(menu(&app).page, MenuPage::ConfirmExit);
    assert_eq!(state(&app), AppState::InGame, "asks first");
    button(&mut app, MenuAct::Confirm);
    settle(&mut app, 4);
    assert_eq!(state(&app), AppState::FrontEnd);
    assert_eq!(*app.world().resource::<State<Screen>>().get(), Screen::Main);
    // Everything of the match is gone.
    assert_eq!(menu(&app), InGameMenu::default());
    assert!(!app.world().resource::<AnimClock>().paused());
    assert_eq!(app.world_mut().query::<&crate::map::MatchEntity>().iter(app.world()).count(), 0);
    assert!(app.world().resource::<crate::sim_link::SimEntities>().0.get(&sta_sim::EntityId(9999)).is_none());
    assert_eq!(menu_roots(&mut app), 0);
    assert!(app.world().resource::<crate::input::select::ControlGroups>().groups.iter().all(Vec::is_empty));
    assert!(app.world().resource::<crate::input::orders::OrderTargets>().0.is_empty());
    assert!(!app.world().contains_resource::<ActiveMatch>());
    assert!(!app.world().contains_resource::<SkirmishCfg>());
    assert!(app.world().resource::<crate::sim_link::Selection>().0.is_empty());
    assert!(app.world().contains_resource::<MenuScene>(), "the menu scene is back");

    // A second match: the first ticks have the digests of the first one (a fresh run's).
    start_match(&mut app);
    assert!(!menu(&app).open);
    let mut second = BTreeMap::new();
    run_recording(&mut app, 30, &mut second);
    let common: Vec<_> = second.iter().filter(|(t, _)| first.contains_key(*t)).collect();
    assert!(common.len() > 20, "{} common ticks", common.len());
    for (t, d) in common {
        assert_eq!(first[t], *d, "tick {t} differs from the first match's");
    }
    // The menu works in the second match.
    key(&mut app, KeyCode::Escape);
    assert!(menu(&app).open);
}

#[test]
fn quit_to_desktop_asks_and_then_exits_the_app() {
    let mut app = app();
    start_match(&mut app);
    key(&mut app, KeyCode::Escape);
    button(&mut app, MenuAct::AskQuit);
    assert_eq!(menu(&app).page, MenuPage::ConfirmQuit);
    key(&mut app, KeyCode::Escape);
    assert_eq!(menu(&app).page, MenuPage::Main, "Esc on a question is No");
    button(&mut app, MenuAct::AskQuit);
    #[derive(Resource, Default)]
    struct Exits(usize);
    app.init_resource::<Exits>().add_systems(Last, |mut e: ResMut<Exits>, mut m: MessageReader<AppExit>| e.0 += m.read().count());
    button(&mut app, MenuAct::Confirm);
    assert_eq!(app.world().resource::<Exits>().0, 1, "AppExit was written once");
    assert_eq!(state(&app), AppState::InGame, "quitting does not leave the match first");
}

#[test]
fn switching_mod_after_exiting_a_match_and_starting_another_works() {
    use crate::frontend::screens::ModChosen;
    let mut app = app();
    start_match(&mut app);
    key(&mut app, KeyCode::Escape);
    button(&mut app, MenuAct::AskExit);
    button(&mut app, MenuAct::Confirm);
    settle(&mut app, 4);
    assert_eq!(state(&app), AppState::FrontEnd);
    // The choice of a mod, as the screen writes it; without a `FlowConfig` the remount waits, which
    // must not take any system of the menu down.
    app.world_mut().write_message(ModChosen(None));
    settle(&mut app, 4);
    start_match(&mut app);
    key(&mut app, KeyCode::Escape);
    assert!(menu(&app).open);
    button(&mut app, MenuAct::Resume);
    assert!(!menu(&app).open);
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

const WINDOWS: [(&str, u32, u32); 4] = [("21:9", 3440, 1440), ("16:9", 1920, 1080), ("16:10", 1920, 1200), ("4:3", 1600, 1200)];
const SCALES: [f32; 3] = [1.0, 1.25, 1.333];

fn overlap(a: Rect, b: Rect) -> bool {
    a.min.x < b.max.x - 0.01 && b.min.x < a.max.x - 0.01 && a.min.y < b.max.y - 0.01 && b.min.y < a.max.y - 0.01
}

fn node_rect(app: &App, e: Entity) -> Rect {
    let n = app.world().get::<Node>(e).unwrap();
    let px = |v: Val| match v {
        Val::Px(p) => p,
        other => panic!("not px: {other:?}"),
    };
    Rect::new(px(n.left), px(n.top), px(n.left) + px(n.width), px(n.top) + px(n.height))
}

fn layout_app(w: u32, h: u32, scale: f32) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin));
    configure_sets(&mut app);
    app.insert_state(AppState::InGame)
        .insert_resource(UiScale(scale))
        .add_plugins(InGameMenuPlugin);
    app.world_mut().spawn((Window { resolution: WindowResolution::new(w, h), ..default() }, PrimaryWindow));
    settle(&mut app, 3);
    app
}

#[test]
fn every_page_fits_every_window_shape_and_ui_scale() {
    use crate::frontend::screens::stock::layout::fit_box;
    for (name, w, h) in WINDOWS {
        for scale in SCALES {
            let mut app = layout_app(w, h, scale);
            let b = fit_box(Vec2::new(w as f32 / scale, h as f32 / scale));
            assert_eq!(app.world().resource::<StockBox>().0, b, "{name} x{scale}: the box");
            let theme = app.world().resource::<Theme>().clone();
            for page in [MenuPage::Main, MenuPage::Options, MenuPage::ConfirmExit, MenuPage::ConfirmQuit] {
                let ctx = format!("{page:?} {name} x{scale}");
                *app.world_mut().resource_mut::<InGameMenu>() = InGameMenu { open: true, page };
                settle(&mut app, 3);
                let placed: Vec<(MenuSlot, Entity)> = app.world_mut().query::<(Entity, &MenuPlaced)>().iter(app.world()).map(|(e, p)| (p.0, e)).collect();
                for slot in slots_of(page) {
                    assert!(placed.iter().any(|(s, _)| s == slot), "{ctx}: no {slot:?}");
                }
                let mut rects = Vec::new();
                for (slot, e) in &placed {
                    let r = node_rect(&app, *e);
                    let want = region(*slot).px(b);
                    for (got, exp) in [(r.min.x, want.min.x), (r.min.y, want.min.y), (r.max.x, want.max.x), (r.max.y, want.max.y)] {
                        assert!((got - exp).abs() < 0.01, "{ctx}: {slot:?} {r:?} is not its region {want:?}");
                    }
                    assert!(r.min.x >= 0.0 && r.min.y >= 0.0 && r.max.x <= b.x + 0.01 && r.max.y <= b.y + 0.01, "{ctx}: {slot:?} leaves the box");
                    let is_panel = matches!(slot, MenuSlot::Panel | MenuSlot::OptionsPanel | MenuSlot::ConfirmPanel);
                    if !is_panel {
                        // Controls sit inside their panel and are tall enough for a button.
                        let panel = *placed.iter().find(|(s, _)| matches!(s, MenuSlot::Panel | MenuSlot::OptionsPanel | MenuSlot::ConfirmPanel)).map(|(_, e)| e).unwrap();
                        let pr = node_rect(&app, panel);
                        assert!(r.min.x >= pr.min.x && r.max.x <= pr.max.x && r.min.y >= pr.min.y && r.max.y <= pr.max.y + 0.01, "{ctx}: {slot:?} is outside its panel");
                        if matches!(slot, MenuSlot::Resume | MenuSlot::Options | MenuSlot::Exit | MenuSlot::Quit | MenuSlot::Yes | MenuSlot::No | MenuSlot::OptionsBack) {
                            assert!(r.height() + 0.01 >= theme.button_h, "{ctx}: {slot:?} is {}px high for a {}px button", r.height(), theme.button_h);
                        }
                        rects.push((*slot, r));
                    }
                }
                for (i, (sa, ra)) in rects.iter().enumerate() {
                    for (sb, rb) in &rects[i + 1..] {
                        assert!(!overlap(*ra, *rb), "{ctx}: {sa:?} overlaps {sb:?}");
                    }
                }
            }
        }
    }
}
