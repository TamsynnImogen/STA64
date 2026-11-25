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
use crate::ui::theme::{Dropdown, DropdownOption, Tab, TabBar, Theme, WidgetState, kit_click};

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
    app.add_plugins((crate::ui::display::DisplayPlugin, crate::ui::graphics::GraphicsPlugin));
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
        single_player_music: None, multiplayer_music: None, music_tracks: Vec::new(),
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
                let want_r = if screen == Screen::Setup {
                    layout::setup_rect(*slot, b, app.world().resource::<SetupDraft>().seats.len())
                } else {
                    layout::placed_rect(*slot, b)
                };
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
                let preview = layout::placed_rect(Slot::Preview, b);
                assert!((preview.width() - preview.height()).abs() < 0.01, "{ctx}: map preview is stretched");
            }
        }
    }
}

#[test]
fn main_menu_layout_in_every_window_and_scale() {
    check_layout(
        Screen::Main,
        &[Slot::MainTitle, Slot::MainDivider, Slot::MainSinglePlayer, Slot::MainInstantAction, Slot::MainMultiplayer, Slot::MainMapEditor, Slot::MainMods, Slot::MainOptions, Slot::MainReplayIntro, Slot::MainCredits, Slot::MainExit],
    );
}

#[test]
fn single_player_layout_in_every_window_and_scale() {
    check_layout(Screen::SinglePlayer, &[Slot::Card(0), Slot::Card(1), Slot::Card(2), Slot::Card(3), Slot::SpBack, Slot::SpLoad]);
}

#[test]
fn setup_layout_in_every_window_and_scale() {
    let mut want = vec![Slot::SetupTitle, Slot::HeadControl, Slot::HeadSide, Slot::HeadTeam, Slot::RandomColour, Slot::MapInfo, Slot::Preview, Slot::SelectMap, Slot::PreviewVisibility, Slot::StartMarkers, Slot::Resources, Slot::DefaultSettings, Slot::SaveSettings, Slot::LoadSettings, Slot::Fog, Slot::RandomPlacement, Slot::Advanced, Slot::SetupBack, Slot::Launch];
    // The fake map has two seats.
    for i in 0..2 {
        want.extend([Slot::SeatColour(i), Slot::SeatControl(i), Slot::SeatSide(i), Slot::SeatTeam(i)]);
    }
    check_layout(Screen::Setup, &want);
}

#[test]
fn setup_keeps_real_seat_rows_and_all_sixteen_colours_in_one_strip() {
    for count in [2, 3, 8] {
        let mut data = four_races();
        let mut map = data.maps[1].def.as_ref().clone();
        map.starts = (0..count).map(|i| {
            let mut start = map.starts[i % map.starts.len()].clone();
            start.slot = i as u8;
            start
        }).collect();
        data.maps = vec![crate::frontend::screens::data::MapEntry::new(map)];
        let palette = (1..=16).map(|i| format!("mpcolor{i:02} = {} 0.3 0.6", i as f32 / 16.0)).collect::<Vec<_>>().join("\n");
        data.colours = crate::viewer::TeamColours(crate::viewer::TeamColours::parse(&palette));
        let mut app = app_with(data, None, 1.0);
        go(&mut app, Screen::Setup);
        let all = placed(&mut app);
        assert_eq!(all.iter().filter(|(slot, _)| matches!(slot, Slot::SeatControl(_))).count(), count);
        assert_eq!(all.iter().filter(|(slot, _)| matches!(slot, Slot::Palette(_))).count(), 16);
        for i in 0..16 {
            let r = layout::setup_region(Slot::Palette(i), count).rect();
            assert!(r.min.x >= layout::setup_region(Slot::RandomColour, count).rect().max.x);
            assert!(r.max.x <= 454.0);
            assert!(r.min.y >= region(Slot::SeatControl(count - 1)).rect().max.y);
            assert_eq!(r.min.y, layout::setup_region(Slot::Palette(0), count).rect().min.y);
        }
    }
}

#[test]
fn setup_minimap_toggle_hides_the_image_child_but_keeps_its_frame() {
    let mut app = app_with(four_races(), None, 1.0);
    go(&mut app, Screen::Setup);
    assert!(placed(&mut app).iter().any(|(slot, _)| *slot == Slot::Preview));
    press(&mut app, Act::TogglePreview);
    assert!(!app.world().resource::<SetupDraft>().preview_visible);
    let mut q = app.world_mut().query_filtered::<&Visibility, With<crate::frontend::screens::preview::AuthoredPreview>>();
    assert_eq!(*q.single(app.world()).unwrap(), Visibility::Hidden);
    assert!(placed(&mut app).iter().any(|(slot, _)| *slot == Slot::Preview));
    press(&mut app, Act::CycleStartMarkers);
    assert_eq!(app.world().resource::<SetupDraft>().start_markers.label(), "Show Available");
}

#[test]
fn setup_colour_and_observer_controls_follow_the_local_seat() {
    let mut app = app_with(four_races(), None, 1.0);
    go(&mut app, Screen::Setup);
    let local = dropdown_of(&mut app, 0, Col::Team);
    let remote = dropdown_of(&mut app, 1, Col::Team);
    let teams = app.world().resource::<SetupDraft>().seats.len();
    assert_eq!(app.world().get::<Dropdown>(local).unwrap().options.last().map(String::as_str), Some("Observer"));
    assert_eq!(app.world().get::<Dropdown>(remote).unwrap().options.len(), teams);
    assert!(app.world_mut().query::<&Act>().iter(app.world()).any(|a| *a == Act::RandomTeamColour));
    assert!(placed(&mut app).iter().any(|(slot, _)| *slot == Slot::SeatColour(0)));
    press(&mut app, Act::PickTeamColour(1));
    assert_eq!(app.world().resource::<SetupDraft>().colour, Some(1));
    let team = dropdown_of(&mut app, 0, Col::Team);
    pick(&mut app, team, teams);
    assert!(app.world().resource::<SetupDraft>().observing);
    let team = dropdown_of(&mut app, 0, Col::Team);
    assert_eq!(app.world().get::<Dropdown>(team).unwrap().value(), Some("Observer"));
    assert!(app.world_mut().query::<&Act>().iter(app.world()).any(|a| matches!(a, Act::PickTeamColour(_) | Act::RandomTeamColour)));
    let data = app.world().resource::<ScreenData>().clone();
    let before = app.world().resource::<SetupDraft>().effective_colours(&data).owner_colour(sta_sim::PlayerId(0));
    press(&mut app, Act::RandomTeamColour);
    let draft = app.world().resource::<SetupDraft>();
    assert_ne!(draft.active_swatch(&data), Some(1));
    assert_ne!(draft.effective_colours(&data).owner_colour(sta_sim::PlayerId(0)), before);
    let team = dropdown_of(&mut app, 0, Col::Team);
    pick(&mut app, team, 0);
    assert!(!app.world().resource::<SetupDraft>().observing);
    assert_eq!(app.world().resource::<SetupDraft>().seats[0].control, SeatControl::Human);
}

#[test]
fn advanced_contains_only_modal_actions() {
    let mut app = app();
    go(&mut app, Screen::Advanced);
    let actions: Vec<_> = app.world_mut().query::<&Act>().iter(app.world()).copied().collect();
    assert!(actions.contains(&Act::CloseAdvanced(true)));
    assert!(actions.contains(&Act::CloseAdvanced(false)));
    assert!(actions.contains(&Act::DefaultOptions));
    assert!(!actions.iter().any(|a| matches!(a, Act::Launch | Act::PickMap | Act::PickTeamColour(_))));
    assert_eq!(app.world_mut().query::<&Dropdown>().iter(app.world()).count(), 0);
}

#[test]
fn options_layout_in_every_window_and_scale() {
    check_layout(Screen::Settings, &[Slot::OptionsTitle, Slot::OptionsBody, Slot::OptionsBack]);
}

#[test]
fn advanced_layout_in_every_window_and_scale() {
    check_layout(Screen::Advanced, &[Slot::AdvancedTitle, Slot::AdvancedResources, Slot::AdvancedOfficers, Slot::AdvancedSpeed, Slot::AdvancedDefault, Slot::AdvancedClose, Slot::AdvancedCancel]);
}

#[test]
fn observer_is_selected_in_remembered_team_row_and_advanced_options_reset() {
    let mut app = app_with(four_races(), None, 1.0);
    go(&mut app, Screen::Setup);
    assert!(region(Slot::SeatControl(layout::TABLE_ROWS - 1)).rect().max.y <= layout::setup_region(Slot::RandomColour, layout::TABLE_ROWS).rect().min.y);
    assert!(!app.world().resource::<SetupDraft>().observing);
    let team = dropdown_of(&mut app, 0, Col::Team);
    pick(&mut app, team, 2);
    assert!(app.world().resource::<SetupDraft>().observing);
    assert!(app.world().resource::<SetupDraft>().seats.iter().all(|seat| seat.control != SeatControl::Human));
    let field = dropdown_of(&mut app, 0, Col::Control);
    assert_eq!(app.world().get::<Dropdown>(field).unwrap().options.len(), SeatControl::choices(true).len());
    let idle = SeatControl::choices(true).iter().position(|control| *control == SeatControl::Idle).unwrap();
    pick(&mut app, field, idle);
    assert_eq!(app.world().resource::<SetupDraft>().seats[0].control, SeatControl::Idle);
    let team = dropdown_of(&mut app, 0, Col::Team);
    pick(&mut app, team, 0);
    assert_eq!(app.world().resource::<SetupDraft>().seats.iter().filter(|seat| seat.control == SeatControl::Human).count(), 1);

    press(&mut app, Act::Goto(Screen::Advanced));
    press(&mut app, Act::CycleResources);
    assert_eq!(app.world().resource::<SetupDraft>().options.starting_resources, sta_content::StartingResources::Lots);
    assert_eq!(app.world().resource::<SetupDraft>().speed, crate::clock::MatchSpeed::Normal);
    press(&mut app, Act::CycleMatchSpeed);
    assert_eq!(app.world().resource::<SetupDraft>().speed, crate::clock::MatchSpeed::Normal.next());
    press(&mut app, Act::DefaultOptions);
    assert_eq!(app.world().resource::<SetupDraft>().options, sta_sim::setup::GameOptions::default());
    assert_eq!(app.world().resource::<SetupDraft>().speed, crate::clock::MatchSpeed::Normal);
    press(&mut app, Act::CloseAdvanced(true));
    assert_eq!(*app.world().resource::<State<Screen>>().get(), Screen::Setup);
}

#[test]
fn advanced_officer_choice_uses_authored_rule_and_default_resets_it() {
    let mut data = four_races();
    let (rules, warnings) = sta_content::Rules::from_header(
        "int MP_OFFICER_LIST_SIZE = 2; Officer MP_OFFICER_LIMIT[2] = {\"ol_normal\", 600, \"ol_more\", 900};",
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    data.content = Arc::new(data.content.as_ref().clone().with_rules(rules));
    let mut app = app_with(data, None, 1.0);
    go(&mut app, Screen::Advanced);
    press(&mut app, Act::CycleOfficerLimit);
    assert_eq!(app.world().resource::<SetupDraft>().options.officer_limit, 1);
    press(&mut app, Act::DefaultOptions);
    assert_eq!(app.world().resource::<SetupDraft>().options.officer_limit, 0);
}

#[test]
fn plain_advanced_speed_cycles_and_resets() {
    let mut app = app();
    app.world_mut().resource_mut::<ScreensConfig>().layout = ScreenLayout::Plain;
    settle(&mut app);
    go(&mut app, Screen::Advanced);
    assert!(placed(&mut app).is_empty());
    press(&mut app, Act::CycleMatchSpeed);
    assert_eq!(app.world().resource::<SetupDraft>().speed, crate::clock::MatchSpeed::Fast);
    press(&mut app, Act::DefaultOptions);
    assert_eq!(app.world().resource::<SetupDraft>().speed, crate::clock::MatchSpeed::Normal);
}

#[test]
fn the_stock_arrangement_holds_in_a_window() {
    // Same controls in the same places as the stock screen, in relative terms: the
    // table's columns left to right, the table left of the map column, the map
    // Preview right of the settings and the bottom row last. The main menu has
    // a kicker, title and divider above one left-aligned navigation column.
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
    assert!(at(&app, Slot::Preview).max.y <= at(&app, Slot::Resources).min.y);
    assert!(at(&app, Slot::Preview).max.y <= at(&app, Slot::Launch).min.y);
    let b_scale = app.world().resource::<StockBox>().0.y / 1080.0;
    go(&mut app, Screen::Main);
    let order = [Slot::MainTitle, Slot::MainDivider, Slot::MainSinglePlayer, Slot::MainInstantAction, Slot::MainMultiplayer, Slot::MainMapEditor, Slot::MainMods, Slot::MainOptions, Slot::MainReplayIntro, Slot::MainCredits, Slot::MainExit];
    for pair in order.windows(2) {
        assert!(at(&app, pair[0]).max.y <= at(&app, pair[1]).min.y, "{pair:?} overlaps vertically");
    }
    let rows = &order[2..];
    assert!(rows.iter().all(|slot| (at(&app, *slot).min.x - at(&app, rows[0]).min.x).abs() < 0.01));
    assert!((at(&app, Slot::MainInstantAction).min.y - at(&app, Slot::MainSinglePlayer).min.y - 66.0 * b_scale).abs() < 0.1);
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
    assert_eq!(options[0], "Random");
    assert_eq!(options[1..], ids);
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
    pick(&mut app, f, 3);
    let f = dropdown_of(&mut app, 0, Col::Race);
    pick(&mut app, f, 2);
    let f = dropdown_of(&mut app, 1, Col::Team);
    pick(&mut app, f, 0);
    let f = dropdown_of(&mut app, 1, Col::Control);
    let idle = SeatControl::choices(false).iter().position(|control| *control == SeatControl::Idle).unwrap();
    pick(&mut app, f, idle);
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
fn both_setup_layouts_offer_mcp_and_use_installed_control_labels() {
    let header = "\"GUI_PLAYER\", \"Local Captain\", \"Computer Easy\", \"Cadet\", \"Computer Medium\", \"Officer\", \"Computer Hard\", \"Admiral\"";
    for layout in [ScreenLayout::Stock, ScreenLayout::Plain] {
        let mut app = app_with(four_races(), None, 1.0);
        app.insert_resource(crate::ui::i18n::Strings::from_game("english", header));
        app.world_mut().resource_mut::<ScreensConfig>().layout = layout;
        go(&mut app, Screen::Setup);

        let field = dropdown_of(&mut app, 1, Col::Control);
        let options = &app.world().get::<Dropdown>(field).unwrap().options;
        assert_eq!(options, &vec!["Local Captain", "Cadet", "Officer", "Admiral", "MCP", "Idle"], "{layout:?}");
        let mcp = SeatControl::choices(false).iter().position(|control| *control == SeatControl::Mcp).unwrap();
        pick(&mut app, field, mcp);
        assert_eq!(app.world().resource::<SetupDraft>().seats[1].control, SeatControl::Mcp, "{layout:?}");

        let field = dropdown_of(&mut app, 1, Col::Control);
        assert_eq!(app.world().get::<Dropdown>(field).unwrap().value(), Some("MCP"), "{layout:?}");

        let observer = app.world().resource::<SetupDraft>().seats.len();
        app.world_mut().resource_mut::<SetupDraft>().choose_team(0, observer);
        settle(&mut app);
        let field = dropdown_of(&mut app, 1, Col::Control);
        let options = &app.world().get::<Dropdown>(field).unwrap().options;
        assert_eq!(options.len(), SeatControl::choices(true).len(), "{layout:?}");
        assert!(!options.iter().any(|label| label == "Local Captain"), "{layout:?}");
    }
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
    let controls = app.world().resource::<super::controls::ControlsDraft>().clone();
    let selected = app.world().resource::<crate::frontend::screens::SelectedMod>().clone();
    let active_mod = app.world().resource::<crate::frontend::screens::ActiveMod>().clone();
    let scroll = *app.world().resource::<crate::frontend::screens::ListScroll>();
    let ctx = crate::frontend::screens::view::Ctx {
        theme: &theme, data: &data, strings: None, draft: &draft, browse: &browse, settings: &settings, controls: &controls, selected: &selected, active_mod: &active_mod, scroll: &scroll,
        layout: ScreenLayout::Stock, stock_box: Vec2::new(1440.0, 1080.0), options_tab: 0, graphics: None, campaigns: None, editor_maps: &[], editor_map_selected: 0,
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
fn main_menu_has_nine_art_buttons_with_labels_and_existing_routes() {
    use crate::ui::sprite_font::SpriteText;
    use crate::ui::gui_sprites::{GuiButton, GuiButtonLabel};
    use crate::ui::theme::UiButton;

    let mut app = app();
    let order = [Slot::MainSinglePlayer, Slot::MainInstantAction, Slot::MainMultiplayer, Slot::MainMapEditor, Slot::MainMods, Slot::MainOptions, Slot::MainReplayIntro, Slot::MainCredits, Slot::MainExit];
    let labels = ["SINGLE PLAYER", "INSTANT ACTION", "MULTIPLAYER", "MAP EDITOR", "MOD MANAGER", "OPTIONS", "REPLAY INTRO", "CREDITS", "EXIT"];
    for (slot, want) in order.into_iter().zip(labels) {
        let entity = placed(&mut app).into_iter().find(|(s, _)| *s == slot).unwrap().1;
        assert!(app.world().get::<UiButton>(entity).is_some(), "{slot:?}");
        assert_eq!(app.world().get::<BackgroundColor>(entity).unwrap().0, super::main_menu::BUTTON_BACK);
        let art = app.world().get::<GuiButton>(entity).unwrap();
        assert_eq!(art.names.iter().map(String::as_str).collect::<Vec<_>>(), ["comm_generic_button"]);
        assert!(art.keep_label);
        let label = app.world().get::<Children>(entity).unwrap().iter().find(|e| app.world().get::<GuiButtonLabel>(*e).is_some()).unwrap();
        let text = app.world().get::<SpriteText>(label).unwrap();
        assert_eq!(text.text, want, "{slot:?}");
        let disabled = matches!(slot, Slot::MainReplayIntro | Slot::MainCredits);
        assert_eq!(*app.world().get::<WidgetState>(entity).unwrap() == WidgetState::Disabled, disabled, "{slot:?}");
        assert_eq!(app.world().get::<Act>(entity).is_some(), !disabled, "{slot:?}");
    }
    let title = placed(&mut app).into_iter().find(|(s, _)| *s == Slot::MainTitle).unwrap().1;
    let title_text = app.world().get::<Children>(title).unwrap()[0];
    assert_eq!(app.world().get::<SpriteText>(title_text).unwrap().text, "STA64");
    assert!(!placed(&mut app).iter().any(|(s, _)| *s == Slot::MainKicker));

    press(&mut app, Act::PickMod);
    assert_eq!(*app.world().resource::<State<Screen>>().get(), Screen::ModSelect);
    go(&mut app, Screen::Main);
    press(&mut app, Act::Goto(Screen::Settings));
    assert_eq!(*app.world().resource::<State<Screen>>().get(), Screen::Settings);
}

#[test]
fn main_menu_uses_active_mod_display_name_and_wraps_long_names() {
    use crate::frontend::screens::{ActiveMod, SelectedMod};
    use crate::ui::sprite_font::{SpriteText, SpriteTextStyle};

    let mut data = ScreenData::fake();
    let name = "A Very Long Mod Name With Federation, Klingon, Romulan And More Ships";
    data.mods[0].name = name.into();
    let mut app = app_with(data, None, 1.0);
    app.world_mut().resource_mut::<SelectedMod>().0 = Some("FX_BALANCE".into());
    settle(&mut app);
    let title_text = |app: &mut App| {
        let title = placed(app).into_iter().find(|(s, _)| *s == Slot::MainTitle).unwrap().1;
        app.world().get::<Children>(title).unwrap()[0]
    };
    let text = title_text(&mut app);
    assert_eq!(app.world().get::<SpriteText>(text).unwrap().text, "STA64", "picker state does not confirm a mount");
    app.world_mut().resource_mut::<ActiveMod>().0 = Some("FX_BALANCE".into());
    settle(&mut app);
    let title = placed(&mut app).into_iter().find(|(s, _)| *s == Slot::MainTitle).unwrap().1;
    let text = app.world().get::<Children>(title).unwrap()[0];
    assert_eq!(app.world().get::<SpriteText>(text).unwrap().text, name);
    let style = app.world().get::<SpriteTextStyle>(text).unwrap();
    assert_eq!(style.wrap, Some(region(Slot::MainTitle).px(app.world().resource::<StockBox>().0).width()));
    assert!(app.world().get::<SpriteText>(text).unwrap().size < 68.0);
    assert_eq!(region(Slot::MainSinglePlayer).rect().min.y, 199.0);
}

#[test]
fn main_menu_focus_skips_unavailable_rows_and_keeps_dark_fallback() {
    use crate::ui::sprite_font::SpriteText;
    use crate::ui::theme::{KitFocus, focus_order, move_focus};

    let mut app = app();
    let order = focus_order(app.world_mut());
    let slots: Vec<_> = order.iter().map(|e| app.world().get::<Placed>(*e).unwrap().0).collect();
    assert_eq!(slots, [Slot::MainSinglePlayer, Slot::MainInstantAction, Slot::MainMultiplayer, Slot::MainMapEditor, Slot::MainMods, Slot::MainOptions, Slot::MainExit]);
    let first = order[0];
    let label = app.world().get::<Children>(first).unwrap().iter().find(|e| app.world().get::<main_menu::MainMenuLabel>(*e).is_some()).unwrap();
    let normal = app.world().get::<SpriteText>(label).unwrap().color;
    move_focus(app.world_mut(), false);
    settle(&mut app);
    assert_eq!(app.world().resource::<KitFocus>().0, Some(first));
    assert_ne!(app.world().get::<SpriteText>(label).unwrap().color, normal);
    assert_eq!(app.world().get::<BackgroundColor>(first).unwrap().0, super::main_menu::BUTTON_BACK);
    for _ in 0..order.len() {
        move_focus(app.world_mut(), false);
    }
    assert_eq!(app.world().resource::<KitFocus>().0, Some(first));
    let editor = placed(&mut app).into_iter().find(|(s, _)| *s == Slot::MainMapEditor).unwrap().1;
    crate::ui::theme::click(app.world_mut(), editor);
    settle(&mut app);
    assert_eq!(*app.world().resource::<State<Screen>>().get(), Screen::MapEditorPicker);
}

#[cfg(not(target_os = "android"))]
#[test]
fn options_tabs_switch_and_settings_follow() {
    use crate::frontend::screens::{SettingKey, WindowSize, view::{SettingSlider, SettingToggle, WindowSizeSelect}};
    use crate::ui::theme::{Dropdown, DropdownChanged, SliderChanged, ToggleChanged};
    let mut app = app_with(ScreenData::fake(), Some((1100, 700)), 1.0);
    go(&mut app, Screen::Settings);
    let bar = app.world_mut().query_filtered::<Entity, With<TabBar>>().single(app.world()).unwrap();
    assert_eq!(app.world().get::<TabBar>(bar).unwrap().selected, 0);
    let tab2 = app.world_mut().query::<(Entity, &Tab)>().iter(app.world()).find(|(_, t)| t.0 == 2).map(|(e, _)| e).unwrap();
    kit_click(app.world_mut(), tab2);
    settle(&mut app);
    assert_eq!(app.world().get::<TabBar>(bar).unwrap().selected, 2);
    let size = app.world_mut().query_filtered::<Entity, With<WindowSizeSelect>>().single(app.world()).unwrap();
    assert_eq!(app.world().get::<Dropdown>(size).unwrap().selected, 0);
    pick(&mut app, size, 3);
    assert_eq!(app.world().resource::<crate::frontend::screens::Settings>().window_size, WindowSize::R1600x900);
    let window = app.world_mut().query_filtered::<&Window, With<PrimaryWindow>>().single(app.world()).unwrap();
    assert_eq!((window.resolution.physical_width(), window.resolution.physical_height()), (1600, 900));
    let size = app.world_mut().query_filtered::<Entity, With<WindowSizeSelect>>().single(app.world()).unwrap();
    app.world_mut().write_message(DropdownChanged { entity: size, index: usize::MAX });
    settle(&mut app);
    assert_eq!(app.world().resource::<crate::frontend::screens::Settings>().window_size, WindowSize::R1600x900);
    // A slider and a toggle move their settings.
    let slider = app.world_mut().query::<(Entity, &SettingSlider)>().iter(app.world()).find(|(_, k)| k.0 == SettingKey::MusicVolume).map(|(e, _)| e).unwrap();
    app.world_mut().write_message(SliderChanged { entity: slider, value: 12.0 });
    let toggle = app.world_mut().query::<(Entity, &SettingToggle)>().iter(app.world()).find(|(_, k)| k.0 == SettingKey::Vsync).map(|(e, _)| e).unwrap();
    app.world_mut().write_message(ToggleChanged { entity: toggle, on: false });
    settle(&mut app);
    let s = app.world().resource::<crate::frontend::screens::Settings>();
    assert_eq!(s.music, 12.0);
    assert!(!s.vsync);
    go(&mut app, Screen::Main);
    go(&mut app, Screen::Settings);
    let size = app.world_mut().query_filtered::<Entity, With<WindowSizeSelect>>().single(app.world()).unwrap();
    assert_eq!(app.world().get::<Dropdown>(size).unwrap().selected, 3);
}

/// The Graphics tab's HUD layout dropdown starts centred, writes the setting and survives a rebuild.
#[cfg(not(target_os = "android"))]
#[test]
fn hud_layout_dropdown_follows_the_setting() {
    use crate::frontend::screens::view::HudFitSelect;
    use crate::ui::hud::stock::layout::HudFit;
    use crate::ui::theme::{Dropdown, DropdownChanged};
    let mut app = app_with(ScreenData::fake(), Some((1100, 700)), 1.0);
    go(&mut app, Screen::Settings);
    let bar = app.world_mut().query_filtered::<Entity, With<TabBar>>().single(app.world()).unwrap();
    let tab2 = app.world_mut().query::<(Entity, &Tab)>().iter(app.world()).find(|(_, t)| t.0 == 2).map(|(e, _)| e).unwrap();
    kit_click(app.world_mut(), tab2);
    settle(&mut app);
    assert_eq!(app.world().get::<TabBar>(bar).unwrap().selected, 2);
    let field = app.world_mut().query_filtered::<Entity, With<HudFitSelect>>().single(app.world()).unwrap();
    assert_eq!(app.world().get::<Dropdown>(field).unwrap().selected, 0);
    pick(&mut app, field, 1);
    assert_eq!(app.world().resource::<crate::frontend::screens::Settings>().hud_fit, HudFit::Stretched);
    app.world_mut().write_message(DropdownChanged { entity: field, index: usize::MAX });
    settle(&mut app);
    assert_eq!(app.world().resource::<crate::frontend::screens::Settings>().hud_fit, HudFit::Stretched);
    go(&mut app, Screen::Main);
    go(&mut app, Screen::Settings);
    let field = app.world_mut().query_filtered::<Entity, With<HudFitSelect>>().single(app.world()).unwrap();
    assert_eq!(app.world().get::<Dropdown>(field).unwrap().selected, 1);
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
fn controls_editor_keeps_page_text_and_applies_only_valid_bindings() {
    use crate::input::keys::{Action, KeyBindings};
    use crate::ui::theme::TextField;
    let mut app = app();
    go(&mut app, Screen::Settings);
    let tab = app.world_mut().query::<(Entity, &Tab)>().iter(app.world()).find(|(_, t)| t.0 == 3).unwrap().0;
    kit_click(app.world_mut(), tab);
    settle(&mut app);
    let field = |app: &mut App, action| app.world_mut().query::<(Entity, &controls::BindingField)>().iter(app.world()).find(|(_, f)| f.0 == action).unwrap().0;
    let button = |app: &mut App, act| app.world_mut().query::<(Entity, &controls::ControlsAct)>().iter(app.world()).find(|(_, a)| **a == act).unwrap().0;
    let add = field(&mut app, Action::AddModifier);
    app.world_mut().get_mut::<TextField>(add).unwrap().text = "KeyQ".into();
    let next = button(&mut app, controls::ControlsAct::Next);
    crate::ui::theme::click(app.world_mut(), next);
    settle(&mut app);
    assert_eq!(app.world().resource::<controls::ControlsDraft>().text[0], "KeyQ");
    for _ in 0..2 {
        let next = button(&mut app, controls::ControlsAct::Next);
        crate::ui::theme::click(app.world_mut(), next);
        settle(&mut app);
    }
    let reveal = field(&mut app, Action::RevealShroud);
    app.world_mut().get_mut::<TextField>(reveal).unwrap().text = "KeyH, NotAKey".into();
    let apply = button(&mut app, controls::ControlsAct::Apply);
    crate::ui::theme::click(app.world_mut(), apply);
    settle(&mut app);
    let live = app.world().resource::<KeyBindings>();
    assert_eq!(live.keys(Action::AddModifier), [KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    assert_eq!(live.keys(Action::RevealShroud), [KeyCode::KeyV]);
    assert!(app.world().resource::<controls::ControlsDraft>().error.as_ref().unwrap().contains("NotAKey"));
    let reveal = field(&mut app, Action::RevealShroud);
    app.world_mut().get_mut::<TextField>(reveal).unwrap().text = "KeyH".into();
    let apply = button(&mut app, controls::ControlsAct::Apply);
    crate::ui::theme::click(app.world_mut(), apply);
    settle(&mut app);
    let live = app.world().resource::<KeyBindings>();
    assert_eq!(live.keys(Action::AddModifier), [KeyCode::KeyQ]);
    assert_eq!(live.keys(Action::RevealShroud), [KeyCode::KeyH]);
    assert_eq!(KeyBindings::from_text(&live.to_text()), *live);
    go(&mut app, Screen::Main);
    go(&mut app, Screen::Settings);
    assert_eq!(app.world().resource::<controls::ControlsDraft>().text[0], "KeyQ");
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

#[cfg(not(target_os = "android"))]
#[test]
fn graphics_checkboxes_and_multisampling_apply_in_the_shared_panel() {
    use crate::frontend::screens::{Settings, SettingKey, view::{MsaaSelect, SettingToggle, WindowedMode}};
    use crate::render::{quality::RenderQuality, post::PostFx};
    let mut app = app_with(ScreenData::fake(), Some((1280, 720)), 1.0);
    app.insert_resource(RenderQuality::default()).insert_resource(PostFx::default());
    go(&mut app, Screen::Settings);
    let tab = app.world_mut().query::<(Entity, &Tab)>().iter(app.world()).find(|(_, t)| t.0 == 2).unwrap().0;
    kit_click(app.world_mut(), tab);
    settle(&mut app);
    let windowed = app.world_mut().query_filtered::<Entity, With<WindowedMode>>().single(app.world()).unwrap();
    kit_click(app.world_mut(), windowed);
    settle(&mut app);
    assert!(app.world().resource::<Settings>().fullscreen);
    let checkbox = |app: &mut App, key| app.world_mut().query::<(Entity, &SettingToggle)>().iter(app.world()).find(|(_, k)| k.0 == key).unwrap().0;
    let shadows = checkbox(&mut app, SettingKey::Shadows);
    kit_click(app.world_mut(), shadows);
    settle(&mut app);
    assert_eq!(app.world().resource::<Settings>().shadows, Some(false));
    assert!(!app.world().resource::<RenderQuality>().shadows);
    let bloom = checkbox(&mut app, SettingKey::Bloom);
    kit_click(app.world_mut(), bloom);
    settle(&mut app);
    assert!(app.world().resource::<PostFx>().bloom);
    let aa = app.world_mut().query_filtered::<Entity, With<MsaaSelect>>().single(app.world()).unwrap();
    pick(&mut app, aa, 2);
    assert_eq!(app.world().resource::<Settings>().msaa, Some(2));
    assert_eq!(app.world().resource::<RenderQuality>().msaa, 2);
}

#[test]
fn bloom_checkbox_shows_live_preset_and_reapplies_an_unchanged_saved_choice() {
    use crate::frontend::screens::{Settings, SettingKey, view::SettingToggle};
    use crate::render::post::PostFx;
    use crate::ui::theme::Checkbox;
    let mut app = app();
    app.insert_resource(PostFx::default());
    app.world_mut().resource_mut::<Settings>().bloom = Some(true);
    go(&mut app, Screen::Settings);
    app.world_mut().resource_mut::<PostFx>().bloom = false;
    settle(&mut app);
    let bloom = app.world_mut().query::<(Entity, &SettingToggle)>().iter(app.world()).find(|(_, k)| k.0 == SettingKey::Bloom).unwrap().0;
    assert!(!app.world().get::<Checkbox>(bloom).unwrap().on);
    kit_click(app.world_mut(), bloom);
    settle(&mut app);
    assert_eq!(app.world().resource::<Settings>().bloom, Some(true));
    assert!(app.world().resource::<PostFx>().bloom);
}
