//! Windowless tests of the stock screens: the layout of each screen in a window of
//! every shape and UI scale, the setup dropdowns and their `GameSetup`, and the
//! texts from the mounted `label.map` (install-gated).

use std::sync::Arc;

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::window::{PrimaryWindow, WindowResolution};
use sta_content::{RaceDef, Races};
use sta_sim::setup::{Control, GameSetup};

use super::layout::{self, Slot, region};
use super::*;
use crate::app::{AppState, configure_sets};
use crate::frontend::screens::{Act, ScreenRoot, ScreensConfig, ScreensPlugin, StartGame};
use crate::ui::theme::{Dropdown, DropdownOption, Tab, TabBar, Theme, kit_click};

/// What the screens sent out.
#[derive(Resource, Default)]
struct Seen {
    start: Vec<GameSetup>,
}

fn record(mut seen: ResMut<Seen>, mut a: MessageReader<StartGame>) {
    seen.start.extend(a.read().map(|m| m.0.clone()));
}

fn app_with(data: ScreenData, window: Option<(u32, u32)>, scale: f32) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin));
    configure_sets(&mut app);
    app.add_plugins(crate::ui::profile::UiProfilePlugin);
    app.insert_resource(data)
        .insert_resource(UiScale(scale))
        .insert_resource(ScreensConfig { own_camera: false, fit_scale: false, start: Screen::Main, ..default() })
        .init_resource::<Seen>()
        .add_systems(Last, record)
        .add_plugins(ScreensPlugin);
    if let Some((w, h)) = window {
        app.world_mut().spawn((Window { resolution: WindowResolution::new(w, h), ..default() }, PrimaryWindow));
    }
    app.world_mut().resource_mut::<NextState<AppState>>().set(AppState::FrontEnd);
    settle(&mut app);
    app
}

fn app() -> App {
    app_with(ScreenData::fake(), None, 1.0)
}

fn settle(app: &mut App) {
    for _ in 0..4 {
        app.update();
    }
}

fn go(app: &mut App, s: Screen) {
    app.world_mut().resource_mut::<NextState<Screen>>().set(s);
    settle(app);
}

fn race(name: &str, slot: i64) -> RaceDef {
    RaceDef {
        stem: name.into(), name: name.into(), display_key: None, slot: Some(slot), interface: None, interface_sprites: None,
        single_player_music: None, multiplayer_music: None,
        standard_units: Vec::new(), minimal_units: Vec::new(), stock: [0; 4],
        lots_stock: [0; 4], normal_crew: None, lots_crew: None,
    }
}

fn four_races() -> ScreenData {
    let mut data = ScreenData::fake();
    data.races = Arc::new(Races::from_defs(vec![race("fed", 0), race("kli", 1), race("rom", 2), race("bor", 3)]));
    data
}

/// Slots out of order and a race that is not playable.
fn mixed_races() -> ScreenData {
    let mut data = ScreenData::fake();
    let mut observer = race("observer", 0);
    observer.slot = None;
    data.races = Arc::new(Races::from_defs(vec![race("rom", 2), observer, race("fed", 0), race("bor", 3), race("kli", 1)]));
    data
}

fn placed(app: &mut App) -> Vec<(Slot, Entity)> {
    app.world_mut().query::<(Entity, &Placed)>().iter(app.world()).map(|(e, p)| (p.0, e)).collect()
}

fn node_rect(app: &App, e: Entity) -> Rect {
    let n = app.world().get::<Node>(e).unwrap();
    let px = |v: Val| match v {
        Val::Px(p) => p,
        other => panic!("not px: {other:?}"),
    };
    Rect::new(px(n.left), px(n.top), px(n.left) + px(n.width), px(n.top) + px(n.height))
}

fn dropdown_of(app: &mut App, row: usize, col: Col) -> Entity {
    app.world_mut().query::<(Entity, &SeatDrop)>().iter(app.world()).find(|(_, d)| d.row == row && d.col == col).map(|(e, _)| e).unwrap()
}

fn pick(app: &mut App, field: Entity, index: usize) {
    kit_click(app.world_mut(), field);
    settle(app);
    let row = app.world_mut().query::<(Entity, &DropdownOption)>().iter(app.world()).find(|(_, o)| o.0 == index).map(|(e, _)| e).unwrap();
    kit_click(app.world_mut(), row);
    settle(app);
}

fn act(app: &mut App, want: Act) -> Entity {
    app.world_mut().query::<(Entity, &Act)>().iter(app.world()).find(|(_, a)| **a == want).unwrap_or_else(|| panic!("no {want:?}")).0
}

fn press(app: &mut App, want: Act) {
    let e = act(app, want);
    crate::ui::theme::click(app.world_mut(), e);
    settle(app);
}

/// Windows: 21:9 (Cedric's 3440x1440), 16:9, 16:10 and 4:3, at the UI scales he uses.
const WINDOWS: [(&str, u32, u32); 4] = [("21:9", 3440, 1440), ("16:9", 1920, 1080), ("16:10", 1920, 1200), ("4:3", 1600, 1200)];
const SCALES: [f32; 3] = [1.0, 1.25, 1.333];

fn overlap(a: Rect, b: Rect) -> bool {
    a.min.x < b.max.x - 0.01 && b.min.x < a.max.x - 0.01 && a.min.y < b.max.y - 0.01 && b.min.y < a.max.y - 0.01
}

/// Each control of `screen` is in its stock region, inside the window, clear of the
/// others, and its widget fits the region, for every window shape and UI scale.
fn check_layout(screen: Screen, want: &[Slot]) {
    for (name, w, h) in WINDOWS {
        for scale in SCALES {
            let mut app = app_with(four_races(), Some((w, h)), scale);
            go(&mut app, screen);
            let ctx = format!("{screen:?} {name} x{scale}");
            let avail = Vec2::new(w as f32 / scale, h as f32 / scale);
            let b = layout::fit_box(avail);
            assert_eq!(app.world().resource::<StockBox>().0, b, "{ctx}: the box");
            assert!(b.x <= avail.x && b.y <= avail.y, "{ctx}: the box fits the window");
            let aspect = b.x / b.y;
            assert!((layout::MIN_ASPECT - 0.01..=layout::MAX_ASPECT + 0.01).contains(&aspect), "{ctx}: aspect {aspect}");
            let theme = app.world().resource::<Theme>().clone();
            let all = placed(&mut app);
            for slot in want {
                assert!(all.iter().any(|(s, _)| s == slot), "{ctx}: no {slot:?}");
            }
            let mut rects = Vec::new();
            for (slot, e) in &all {
                let r = node_rect(&app, *e);
                let want_r = region(*slot).px(b);
                for (got, exp) in [(r.min.x, want_r.min.x), (r.min.y, want_r.min.y), (r.max.x, want_r.max.x), (r.max.y, want_r.max.y)] {
                    assert!((got - exp).abs() < 0.01, "{ctx}: {slot:?} {r:?} is not its region {want_r:?}");
                }
                assert!(r.min.x >= 0.0 && r.min.y >= 0.0 && r.max.x <= b.x + 0.01 && r.max.y <= b.y + 0.01, "{ctx}: {slot:?} leaves the box");
                // The widget inside fits the region (its fixed sizes against the region's).
                if let Some(kid) = app.world().get::<Children>(*e).and_then(|k| k.first().copied())
                    && let Some(n) = app.world().get::<Node>(kid)
                {
                    if let Val::Px(kh) = n.height {
                        assert!(kh <= r.height() + 0.5, "{ctx}: {slot:?} widget {kh}px high in a {}px region", r.height());
                    }
                    if let Val::Px(kw) = n.width {
                        assert!(kw <= r.width() + 0.5, "{ctx}: {slot:?} widget {kw}px wide in a {}px region", r.width());
                    }
                }
                rects.push((*slot, r));
            }
            for (i, (sa, ra)) in rects.iter().enumerate() {
                for (sb, rb) in &rects[i + 1..] {
                    assert!(!overlap(*ra, *rb), "{ctx}: {sa:?} overlaps {sb:?}");
                }
            }
            // The row pitch holds a dropdown at the theme's height.
            if screen == Screen::Setup {
                assert!(region(Slot::SeatSide(0)).px(b).height() + 0.01 >= theme.button_h, "{ctx}: rows are too tight");
            }
        }
    }
}

#[test]
fn main_menu_layout_in_every_window_and_scale() {
    check_layout(
        Screen::Main,
        &[Slot::MainSinglePlayer, Slot::MainInstantAction, Slot::MainMultiplayer, Slot::MainOptions, Slot::MainReplayIntro, Slot::MainCredits, Slot::MainExit, Slot::MainMods],
    );
}

#[test]
fn single_player_layout_in_every_window_and_scale() {
    check_layout(Screen::SinglePlayer, &[Slot::Card(0), Slot::Card(1), Slot::Card(2), Slot::Card(3), Slot::SpBack, Slot::SpLoad]);
}

#[test]
fn setup_layout_in_every_window_and_scale() {
    let mut want = vec![Slot::SetupTitle, Slot::HeadControl, Slot::HeadSide, Slot::HeadTeam, Slot::MapInfo, Slot::Preview, Slot::SelectMap, Slot::Resources, Slot::Officers, Slot::SetupBack, Slot::Launch];
    // The fake map has two seats.
    for i in 0..2 {
        want.extend([Slot::SeatControl(i), Slot::SeatSide(i), Slot::SeatTeam(i)]);
    }
    check_layout(Screen::Setup, &want);
}

#[test]
fn options_layout_in_every_window_and_scale() {
    check_layout(Screen::Settings, &[Slot::OptionsTitle, Slot::OptionsBody, Slot::OptionsBack]);
}

#[test]
fn the_stock_arrangement_holds_in_a_window() {
    // Same controls in the same places as the stock screen, in relative terms: the
    // table's columns left to right, the table left of the map column, the map
    // preview right of the settings, the bottom row last; main menu: Single Player
    // top left, Instant Action top right, Multiplayer bottom left, the small
    // buttons at the right.
    let mut app = app_with(four_races(), Some((3440, 1440)), 1.333);
    go(&mut app, Screen::Setup);
    let at = |app: &App, slot: Slot| node_rect(app, placed_one(app, slot));
    fn placed_one(app: &App, slot: Slot) -> Entity {
        let mut q = app.world().try_query::<(Entity, &Placed)>().unwrap();
        q.iter(app.world()).find(|(_, p)| p.0 == slot).unwrap().0
    }
    let (c, s, t) = (at(&app, Slot::SeatControl(0)), at(&app, Slot::SeatSide(0)), at(&app, Slot::SeatTeam(0)));
    assert!(c.max.x <= s.min.x && s.max.x <= t.min.x);
    assert!(t.max.x <= at(&app, Slot::MapInfo).min.x);
    assert!(at(&app, Slot::Resources).max.x <= at(&app, Slot::Preview).min.x);
    assert!(at(&app, Slot::Preview).max.y <= at(&app, Slot::Launch).min.y);
    let b_scale = app.world().resource::<StockBox>().0.y / 1080.0;
    go(&mut app, Screen::Main);
    let sp = at(&app, Slot::MainSinglePlayer);
    let ia = at(&app, Slot::MainInstantAction);
    let mp = at(&app, Slot::MainMultiplayer);
    assert!(sp.max.x <= ia.min.x && (sp.center().y - ia.center().y).abs() < 40.0 * b_scale);
    assert!(mp.min.y >= sp.max.y);
    assert!(at(&app, Slot::MainOptions).min.x > mp.max.x);
}

#[test]
fn the_default_layout_is_stock_and_plain_keeps_the_old_screens() {
    assert_eq!(ScreensConfig::default().layout, ScreenLayout::Stock);
    let mut app = app();
    assert!(!placed(&mut app).is_empty());
    app.world_mut().resource_mut::<ScreensConfig>().layout = ScreenLayout::Plain;
    settle(&mut app);
    assert!(placed(&mut app).is_empty());
    assert_eq!(app.world_mut().query::<&ScreenRoot>().iter(app.world()).count(), 1);
    // Map select and mod select are UI-07's in both.
    app.world_mut().resource_mut::<ScreensConfig>().layout = ScreenLayout::Stock;
    go(&mut app, Screen::MapSelect);
    assert!(placed(&mut app).is_empty());
}

#[test]
fn the_faction_dropdown_lists_the_playable_factions_in_skirmish_order() {
    let mut app = app_with(mixed_races(), None, 1.0);
    go(&mut app, Screen::Setup);
    let field = dropdown_of(&mut app, 0, Col::Race);
    let options = app.world().get::<Dropdown>(field).unwrap().options.clone();
    let ids = app.world().resource::<ScreenData>().races();
    assert!(!ids.iter().any(|i| i == "observer"), "a race without a skirmish slot is not offered: {ids:?}");
    assert_eq!(ids.len(), 4);
    // The dropdown shows the same factions in the same order (these have no display key).
    assert_eq!(options, ids);
    // Each seat has one, and each shows its own seat's side.
    let two = app.world().resource::<SetupDraft>().seats.clone();
    for (i, seat) in two.iter().enumerate() {
        let d = dropdown_of(&mut app, i, Col::Race);
        let d = app.world().get::<Dropdown>(d).unwrap();
        assert_eq!(d.value(), Some(seat.race.as_str()));
    }
}

#[test]
fn setup_dropdowns_edit_the_draft_and_launch_round_trips() {
    let mut app = app_with(four_races(), None, 1.0);
    go(&mut app, Screen::Setup);
    // Seat 1: side Romulan (index 2), team 1, control Idle; seat 0: Klingon.
    let f = dropdown_of(&mut app, 1, Col::Race);
    pick(&mut app, f, 2);
    let f = dropdown_of(&mut app, 0, Col::Race);
    pick(&mut app, f, 1);
    let f = dropdown_of(&mut app, 1, Col::Team);
    pick(&mut app, f, 0);
    let f = dropdown_of(&mut app, 1, Col::Control);
    pick(&mut app, f, 2);
    let d = app.world().resource::<SetupDraft>().clone();
    assert_eq!((d.seats[0].race.as_str(), d.seats[1].race.as_str()), ("kli", "rom"));
    assert_eq!((d.seats[0].team, d.seats[1].team), (0, 0));
    assert_eq!((d.seats[0].control, d.seats[1].control), (SeatControl::Human, SeatControl::Idle));
    // The screen shows the choices it was rebuilt with.
    let field = dropdown_of(&mut app, 1, Col::Race);
    let shown = app.world().get::<Dropdown>(field).unwrap().value().map(str::to_owned);
    assert_eq!(shown.as_deref(), Some("rom"));

    press(&mut app, Act::Launch);
    let setup = app.world().resource::<Seen>().start.last().cloned().expect("Launch sends a setup");
    assert_eq!(setup.players.len(), 2);
    assert_eq!((setup.players[0].race.as_str(), setup.players[1].race.as_str()), ("kli", "rom"));
    assert!(matches!(setup.players[0].control, Control::Human));
    assert!(matches!(setup.players[1].control, Control::Idle));
    assert_eq!((setup.players[0].team.0, setup.players[1].team.0), (0, 0));
    // The setup reads back as the same draft, and the sim takes it.
    let data = app.world().resource::<ScreenData>().clone();
    assert_eq!(SetupDraft::from_setup(&setup, &data), Some(d));
    let sim = sta_sim::Sim::from_setup(&setup);
    assert_eq!(sta_sim::WorldSource::view(&sim).players.len(), 2);
}

#[test]
fn a_launch_without_a_playable_faction_stays_disabled() {
    let mut data = ScreenData::fake();
    data.races = Arc::new(Races::from_defs(vec![]));
    let mut app = app_with(data, None, 1.0);
    go(&mut app, Screen::Setup);
    assert!(app.world_mut().query::<&Act>().iter(app.world()).all(|a| *a != Act::Launch));
}

#[test]
fn control_choices_keep_exactly_one_player() {
    let mut app = app_with(four_races(), None, 1.0);
    go(&mut app, Screen::Setup);
    let mut d = app.world().resource::<SetupDraft>().clone();
    set_control(&mut d, 1, SeatControl::Human);
    assert_eq!((d.seats[0].control, d.seats[1].control), (SeatControl::Computer, SeatControl::Human));
    set_control(&mut d, 1, SeatControl::Idle);
    assert_eq!((d.seats[0].control, d.seats[1].control), (SeatControl::Human, SeatControl::Idle));
    // A lone player cannot step off.
    let mut one = SetupDraft { seats: vec![d.seats[0].clone()], ..d.clone() };
    set_control(&mut one, 0, SeatControl::Computer);
    assert_eq!(one.seats[0].control, SeatControl::Human);
}

#[test]
fn the_preview_shows_the_start_positions_of_the_picked_map() {
    let mut app = app();
    go(&mut app, Screen::Setup);
    let data = app.world().resource::<ScreenData>().clone();
    let draft = app.world().resource::<SetupDraft>().clone();
    let theme = app.world().resource::<Theme>().clone();
    let browse = app.world().resource::<crate::frontend::screens::Browse>().clone();
    let settings = app.world().resource::<crate::frontend::screens::Settings>().clone();
    let selected = app.world().resource::<crate::frontend::screens::SelectedMod>().clone();
    let scroll = *app.world().resource::<crate::frontend::screens::ListScroll>();
    let ctx = crate::frontend::screens::view::Ctx {
        theme: &theme, data: &data, strings: None, draft: &draft, browse: &browse, settings: &settings, selected: &selected, scroll: &scroll,
        layout: ScreenLayout::Stock, stock_box: Vec2::new(1440.0, 1080.0), options_tab: 0,
    };
    let dots = preview_dots(&ctx);
    assert_eq!(dots.len(), data.maps[0].slots());
    assert!(dots.iter().all(|(x, y)| (0.0..=1.0).contains(x) && (0.0..=1.0).contains(y)));
    // Two bases apart: the dots differ.
    assert!(dots.windows(2).all(|w| w[0] != w[1]));
}

#[test]
fn main_menu_buttons_go_where_they_say() {
    let mut app = app();
    press(&mut app, Act::Goto(Screen::SinglePlayer));
    assert_eq!(*app.world().resource::<State<Screen>>().get(), Screen::SinglePlayer);
    press(&mut app, Act::Goto(Screen::Main));
    press(&mut app, Act::Goto(Screen::Setup));
    assert_eq!(*app.world().resource::<State<Screen>>().get(), Screen::Setup);
    press(&mut app, Act::PickMap);
    assert_eq!(*app.world().resource::<State<Screen>>().get(), Screen::MapSelect);
}

#[test]
fn options_tabs_switch_and_settings_follow() {
    use crate::frontend::screens::{SettingKey, view::{SettingSlider, SettingToggle}};
    use crate::ui::theme::{SliderChanged, ToggleChanged};
    let mut app = app();
    go(&mut app, Screen::Settings);
    let bar = app.world_mut().query_filtered::<Entity, With<TabBar>>().single(app.world()).unwrap();
    assert_eq!(app.world().get::<TabBar>(bar).unwrap().selected, 0);
    let tab2 = app.world_mut().query::<(Entity, &Tab)>().iter(app.world()).find(|(_, t)| t.0 == 2).map(|(e, _)| e).unwrap();
    kit_click(app.world_mut(), tab2);
    settle(&mut app);
    assert_eq!(app.world().get::<TabBar>(bar).unwrap().selected, 2);
    // A slider and a toggle move their settings.
    let slider = app.world_mut().query::<(Entity, &SettingSlider)>().iter(app.world()).find(|(_, k)| k.0 == SettingKey::MusicVolume).map(|(e, _)| e).unwrap();
    app.world_mut().write_message(SliderChanged { entity: slider, value: 12.0 });
    let toggle = app.world_mut().query::<(Entity, &SettingToggle)>().iter(app.world()).find(|(_, k)| k.0 == SettingKey::Vsync).map(|(e, _)| e).unwrap();
    app.world_mut().write_message(ToggleChanged { entity: toggle, on: false });
    settle(&mut app);
    let s = app.world().resource::<crate::frontend::screens::Settings>();
    assert_eq!(s.music, 12.0);
    assert!(!s.vsync);
}

/// The texts come from the install's own `label.map`: the stock captions on the
/// stock buttons. Needs `STA64_TEST_INSTALL` (and `STA64_TEST_MOD`).
#[test]
fn captions_come_from_the_installs_label_map() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
        eprintln!("SKIP: set STA64_TEST_INSTALL (and STA64_TEST_MOD) for the install's label.map captions");
        return;
    };
    let active = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), active.as_deref()).unwrap();
    let strings = crate::ui::i18n::Strings::load(&vfs, "english");
    let native = |k: &str| strings.native_label(k).map(str::to_owned);
    let mut app = app();
    app.insert_resource(strings.clone());
    go(&mut app, Screen::Setup);
    let texts: Vec<String> = app.world_mut().query::<&crate::ui::sprite_font::SpriteText>().iter(app.world()).map(|t| t.text.clone()).collect();
    let mut found = 0;
    for key in ["launch", "select_map", "previous_menu"] {
        if let Some(text) = native(key) {
            found += 1;
            assert!(texts.iter().any(|t| t.eq_ignore_ascii_case(&text)), "{key}: {text} not on the screen: {texts:?}");
        }
    }
    assert!(found > 0, "the install's label.map has none of the keys");
}

#[test]
fn the_options_tab_survives_a_rebuild() {
    let mut app = app();
    go(&mut app, Screen::Settings);
    let tab2 = app.world_mut().query::<(Entity, &Tab)>().iter(app.world()).find(|(_, t)| t.0 == 2).map(|(e, _)| e).unwrap();
    kit_click(app.world_mut(), tab2);
    settle(&mut app);
    // A box change (window resize) rebuilds the screen.
    app.world_mut().resource_mut::<StockBox>().0 = Vec2::new(1200.0, 900.0);
    settle(&mut app);
    let bar = app.world_mut().query_filtered::<Entity, With<TabBar>>().single(app.world()).unwrap();
    assert_eq!(app.world().get::<TabBar>(bar).unwrap().selected, 2);
}

#[test]
fn a_dropdown_keeps_keyboard_focus_through_the_rebuild_its_choice_causes() {
    use crate::ui::theme::{KitFocus, KitKey, kit_key};
    let mut app = app_with(four_races(), None, 1.0);
    go(&mut app, Screen::Setup);
    let field = dropdown_of(&mut app, 0, Col::Race);
    app.world_mut().resource_mut::<KitFocus>().0 = Some(field);
    kit_key(app.world_mut(), KitKey::code(KeyCode::ArrowDown));
    settle(&mut app);
    assert_eq!(app.world().resource::<SetupDraft>().seats[0].race, "kli");
    let now = dropdown_of(&mut app, 0, Col::Race);
    assert_ne!(now, field, "the screen was rebuilt");
    assert_eq!(app.world().resource::<KitFocus>().0, Some(now));
}
