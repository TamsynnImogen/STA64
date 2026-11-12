//! Windowless: the screens in a bare `App` on the fake scene, reached by
//! state, driven by clicking their controls without a pointer.

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use sta_sim::setup::{Control, GameSetup};
use sta_sim::{PlayerId, TeamId};
use sta_content::{RaceDef, Races, Rules, StartingResources};
use std::sync::Arc;

use super::view::{MapList, ModList, SettingSlider, SettingToggle};
use super::*;
use crate::app::configure_sets;
use crate::ui::theme::{ButtonClicked, ListItem, Slider, SliderChanged, Toggle, ToggleChanged, UiButton, WidgetState, click};

/// What the screens sent out, kept (messages expire after two updates).
#[derive(Resource, Default)]
struct Seen {
    start: Vec<GameSetup>,
    chosen: Vec<ModChosen>,
    exits: usize,
}

fn record(mut seen: ResMut<Seen>, mut a: MessageReader<StartGame>, mut b: MessageReader<ModChosen>, mut c: MessageReader<AppExit>) {
    seen.start.extend(a.read().map(|m| m.0.clone()));
    seen.chosen.extend(b.read().cloned());
    seen.exits += c.read().count();
}

fn app() -> App {
    app_with_data(ScreenData::fake())
}

fn app_with_data(data: ScreenData) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin));
    configure_sets(&mut app);
    app.add_plugins(crate::ui::profile::UiProfilePlugin);
    app.insert_resource(data)
        .insert_resource(ScreensConfig { own_camera: false, fit_scale: false, start: Screen::Main })
        .init_resource::<Seen>()
        .add_systems(Last, record)
        .add_plugins(ScreensPlugin);
    app.world_mut().resource_mut::<NextState<AppState>>().set(AppState::FrontEnd);
    settle(&mut app);
    app
}

fn a2_install() -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("sta64-screen-a2-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(root.join("Data")).unwrap();
    root
}

fn settle(app: &mut App) {
    for _ in 0..3 {
        app.update();
    }
}

fn go(app: &mut App, s: Screen) {
    app.world_mut().resource_mut::<NextState<Screen>>().set(s);
    settle(app);
}

fn screen(app: &App) -> Screen {
    *app.world().resource::<State<Screen>>().get()
}

fn roots(app: &mut App) -> Vec<Screen> {
    app.world_mut().query::<&ScreenRoot>().iter(app.world()).map(|r| r.0).collect()
}

fn texts(app: &mut App) -> Vec<String> {
    // Kit text is a `SpriteText` (UI-16); its TTF `Text` child exists once the font plugin has run.
    let mut out: Vec<String> = app.world_mut().query::<&crate::ui::sprite_font::SpriteText>().iter(app.world()).map(|t| t.text.clone()).collect();
    out.extend(app.world_mut().query::<&Text>().iter(app.world()).map(|t| t.0.clone()));
    out
}

fn has_text(app: &mut App, s: &str) -> bool {
    texts(app).iter().any(|t| t.eq_ignore_ascii_case(s))
}

fn entity_with(app: &mut App, want: Act) -> Entity {
    app.world_mut().query::<(Entity, &Act)>().iter(app.world()).find(|(_, a)| **a == want).unwrap_or_else(|| panic!("no {want:?} control")).0
}

fn disabled_button_named(app: &mut App, name: &str) -> Entity {
    let buttons: Vec<_> = app.world_mut()
        .query_filtered::<(Entity, &WidgetState, &Children), With<UiButton>>()
        .iter(app.world())
        .map(|(entity, state, children)| (entity, *state, children[0]))
        .collect();
    let (entity, state, _) = buttons.into_iter()
        .find(|(_, _, child)| app.world().get::<crate::ui::sprite_font::SpriteText>(*child).is_some_and(|text| text.text.eq_ignore_ascii_case(name)))
        .unwrap_or_else(|| panic!("no {name} button"));
    assert_eq!(state, WidgetState::Disabled, "{name} should be disabled");
    entity
}

fn press(app: &mut App, want: Act) {
    let e = entity_with(app, want);
    click(app.world_mut(), e);
    settle(app);
}

fn count<C: Component>(app: &mut App) -> usize {
    app.world_mut().query::<&C>().iter(app.world()).count()
}

fn draft(app: &App) -> SetupDraft {
    app.world().resource::<SetupDraft>().clone()
}

fn started(app: &mut App) -> Vec<GameSetup> {
    app.world().resource::<Seen>().start.clone()
}

fn with_officer_rules(mut data: ScreenData, header: &str) -> ScreenData {
    let (rules, warnings) = Rules::from_header(header);
    assert!(warnings.is_empty(), "{warnings:?}");
    data.content = Arc::new(data.content.as_ref().clone().with_rules(rules));
    data
}

#[test]
fn entering_the_front_end_shows_the_main_screen() {
    let mut app = app();
    assert_eq!(screen(&app), Screen::Main);
    assert_eq!(roots(&mut app), [Screen::Main]);
    for s in ["Star Trek Armada", "Single player", "Instant action", "Multiplayer", "Mods", "Options", "Exit"] {
        assert!(has_text(&mut app, s), "{s}");
    }
    // Six entries; the lobby's Multiplayer is not there to click yet.
    assert_eq!(count::<UiButton>(&mut app), 6);
    let off = app.world_mut().query::<&WidgetState>().iter(app.world()).filter(|s| **s == WidgetState::Disabled).count();
    assert_eq!(off, 1);
}

#[test]
fn every_screen_is_reached_by_state_and_replaces_the_last() {
    let mut app = app();
    let want: [(Screen, &str, usize); 5] = [
        (Screen::SinglePlayer, "Single player", 6),
        (Screen::Setup, "Instant action", 2 * 3 + 5),
        (Screen::MapSelect, "Select map", 2),
        (Screen::Settings, "Options", 1),
        (Screen::ModSelect, "Mods", 2),
    ];
    for (s, title, buttons) in want {
        go(&mut app, s);
        assert_eq!(screen(&app), s);
        assert_eq!(roots(&mut app), [s], "one root, the screen's");
        assert!(has_text(&mut app, title), "{s:?} title");
        assert_eq!(count::<UiButton>(&mut app), buttons, "{s:?} buttons");
    }
    go(&mut app, Screen::Main);
    assert_eq!(roots(&mut app), [Screen::Main]);
}

#[test]
fn leaving_the_front_end_takes_the_screens_with_it() {
    let mut app = app();
    app.world_mut().resource_mut::<NextState<AppState>>().set(AppState::Loading);
    settle(&mut app);
    assert!(roots(&mut app).is_empty());
    // And it comes back at the configured screen.
    app.world_mut().resource_mut::<ScreensConfig>().start = Screen::Setup;
    app.world_mut().resource_mut::<NextState<AppState>>().set(AppState::FrontEnd);
    settle(&mut app);
    assert_eq!(roots(&mut app), [Screen::Setup]);
}

#[test]
fn the_buttons_lead_where_their_words_say() {
    let mut app = app();
    press(&mut app, Act::Goto(Screen::SinglePlayer));
    assert_eq!(screen(&app), Screen::SinglePlayer);
    press(&mut app, Act::Goto(Screen::Main));
    press(&mut app, Act::Goto(Screen::Setup));
    assert_eq!(screen(&app), Screen::Setup);
    press(&mut app, Act::PickMap);
    assert_eq!(screen(&app), Screen::MapSelect);
    press(&mut app, Act::Goto(Screen::Setup));
    press(&mut app, Act::Goto(Screen::Main));
    press(&mut app, Act::Goto(Screen::Settings));
    assert_eq!(screen(&app), Screen::Settings);
    press(&mut app, Act::Goto(Screen::Main));
    press(&mut app, Act::PickMod);
    assert_eq!(screen(&app), Screen::ModSelect);
    press(&mut app, Act::Goto(Screen::Main));
    assert_eq!(screen(&app), Screen::Main);
}

#[test]
fn exit_asks_the_app_to_quit() {
    let mut app = app();
    press(&mut app, Act::Exit);
    assert_eq!(app.world().resource::<Seen>().exits, 1);
}

#[test]
fn setup_round_trips_through_a_game_setup() {
    let data = ScreenData::fake();
    let mut d = SetupDraft::new(&data);
    d.set_map(&data, 1);
    d.cycle_control(2); // Computer -> Idle
    d.cycle_team(3);
    d.cycle_race(&data, 1);
    d.seed = 42;
    let setup = d.to_setup(&data).unwrap();
    assert_eq!(setup.seed, 42);
    assert_eq!(setup.map.name, "fx_four_corners");
    let ids: Vec<_> = setup.players.iter().map(|p| p.id).collect();
    assert_eq!(ids, [PlayerId(0), PlayerId(1), PlayerId(2), PlayerId(3)]);
    let control: Vec<_> = setup.players.iter().map(|p| p.control.clone()).collect();
    assert_eq!(control, [Control::Human, Control::Ai("skirmish".into()), Control::Idle, Control::Ai("skirmish".into())]);
    assert_eq!(setup.players[3].team, TeamId(0));
    assert_eq!(setup.players[2].team, TeamId(2));
    assert!(setup.players.iter().all(|p| p.race == "fx"));
    // And back: the same draft.
    assert_eq!(SetupDraft::from_setup(&setup, &data), Some(d));
    // The sim takes it as it is.
    let sim = sta_sim::Sim::from_setup(&setup);
    assert_eq!(sta_sim::WorldSource::view(&sim).players.len(), 4);
}

fn race(name: &str, slot: Option<i64>) -> RaceDef {
    RaceDef {
        stem: name.into(), name: name.into(), display_key: None, slot, interface: None, interface_sprites: None,
        single_player_music: None, multiplayer_music: None,
        standard_units: Vec::new(), minimal_units: Vec::new(), stock: [0; 4],
        lots_stock: [0; 4], normal_crew: None, lots_crew: None,
    }
}

#[test]
fn starting_choices_follow_authored_rows_into_launch_and_round_trip() {
    let data = with_officer_rules(
        ScreenData::fake(),
        r#"
        int MP_OFFICER_LIST_SIZE = 4;
        Officer MP_OFFICER_LIMIT[4] = {
            "ol_normal", 600, "ol_restricted", 300, "ol_more", 800, "ol_max", 999,
        };
    "#,
    );
    let mut app = app_with_data(data);
    go(&mut app, Screen::Setup);
    assert!(has_text(&mut app, "Resources: Normal"));
    assert!(has_text(&mut app, "ol_normal: 600"));
    assert_eq!(
        draft(&app).options.starting_resources,
        StartingResources::Normal
    );
    assert_eq!(draft(&app).options.officer_limit, 0);
    press(&mut app, Act::CycleResources);
    assert!(has_text(&mut app, "Resources: Lots"));
    for (index, caption) in [
        (1, "ol_restricted: 300"),
        (2, "ol_more: 800"),
        (3, "ol_max: 999"),
    ] {
        press(&mut app, Act::CycleOfficerLimit);
        assert_eq!(draft(&app).options.officer_limit, index);
        assert!(has_text(&mut app, caption), "missing {caption}");
    }
    press(&mut app, Act::Launch);
    let sent = started(&mut app);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].options.starting_resources, StartingResources::Lots);
    assert_eq!(sent[0].options.officer_limit, 3);
    assert_eq!(
        SetupDraft::from_setup(&sent[0], app.world().resource::<ScreenData>()),
        Some(draft(&app))
    );
    press(&mut app, Act::CycleOfficerLimit);
    assert_eq!(draft(&app).options.officer_limit, 0);
    press(&mut app, Act::CycleResources);
    assert_eq!(
        draft(&app).options.starting_resources,
        StartingResources::Normal
    );
}

#[test]
fn custom_officer_order_and_zero_are_preserved_and_mount_change_resets_choice() {
    let data = with_officer_rules(
        ScreenData::fake(),
        r#"
        int MP_OFFICER_LIST_SIZE = 3;
        Officer MP_OFFICER_LIMIT[3] = {"unusual", 17, "none", 0, "ol_normal", 600};
    "#,
    );
    let mut app = app_with_data(data);
    go(&mut app, Screen::Setup);
    assert!(has_text(&mut app, "unusual: 17"));
    press(&mut app, Act::CycleOfficerLimit);
    assert!(has_text(&mut app, "none: 0"));
    press(&mut app, Act::CycleOfficerLimit);
    assert!(has_text(&mut app, "ol_normal: 600"));
    // Map changes retain an authored choice while the same mount stays active.
    let mounted = app.world().resource::<ScreenData>().clone();
    app.world_mut()
        .resource_mut::<SetupDraft>()
        .set_map(&mounted, 1);
    settle(&mut app);
    assert_eq!(draft(&app).options.officer_limit, 2);
    // A new mount may have fewer rows; follow_data resets the stale index.
    app.world_mut().resource_mut::<ScreenData>().content = Arc::new(
        sta_content::fake::db().with_rules(
            Rules::from_header(
                "int MP_OFFICER_LIST_SIZE = 1; Officer MP_OFFICER_LIMIT[1] = {\"only\", 0};",
            )
            .0,
        ),
    );
    settle(&mut app);
    assert_eq!(draft(&app).options.officer_limit, 0);
    assert!(has_text(&mut app, "only: 0"));
}

#[test]
fn race_picker_uses_catalogue_slots_even_without_start_units() {
    let mut data = ScreenData::fake();
    // The fake database contains `fx` units, but the mounted race list does not.
    data.races = Arc::new(Races::from_defs(vec![
        race("later", Some(3)), race("excluded", None), race("zero", Some(0)),
        race("middle", Some(2)),
    ]));
    assert_eq!(data.races(), ["zero", "middle", "later"]);
    let mut draft = SetupDraft::new(&data);
    assert_eq!((draft.seats[0].race.as_str(), draft.seats[1].race.as_str()), ("zero", "middle"));
    draft.cycle_race(&data, 0);
    assert_eq!(draft.seats[0].race, "middle");
    draft.cycle_race(&data, 0);
    assert_eq!(draft.seats[0].race, "later");
    draft.cycle_race(&data, 0);
    assert_eq!(draft.seats[0].race, "zero");
    assert_eq!(draft.to_setup(&data).unwrap().players[0].race, "zero");
}

#[test]
fn explicit_empty_or_nonplayable_catalogue_cannot_launch_inferred_unit_race() {
    for catalogue in [Races::from_defs(vec![]), Races::from_defs(vec![race("observer", None)])] {
        let mut data = ScreenData::fake();
        data.races = Arc::new(catalogue);
        assert!(data.races().is_empty(), "fx units must not enter a mounted catalogue");
        let mut draft = SetupDraft::new(&data);
        assert!(draft.seats.iter().all(|s| s.race.is_empty()));
        draft.cycle_race(&data, 0);
        assert!(draft.seats[0].race.is_empty());
        assert!(draft.to_setup(&data).is_none());
    }
    let mut app = app();
    app.world_mut().resource_mut::<ScreenData>().races = Arc::new(Races::from_defs(vec![]));
    go(&mut app, Screen::Setup);
    let launch = disabled_button_named(&mut app, "Launch");
    click(app.world_mut(), launch);
    settle(&mut app);
    assert!(started(&mut app).is_empty());
}

#[test]
fn stale_seat_races_cannot_launch_after_catalogue_change() {
    let mut data = ScreenData::fake();
    let mut draft = SetupDraft::new(&data);
    assert_eq!(draft.seats[0].race, "fx");
    data.races = Arc::new(Races::from_defs(vec![race("new", Some(0))]));
    assert_eq!(data.races(), ["new"]);
    assert!(draft.to_setup(&data).is_none());
    for row in 0..draft.seats.len() {
        draft.cycle_race(&data, row);
    }
    assert_eq!(draft.seats.iter().map(|s| s.race.as_str()).collect::<Vec<_>>(), ["new", "new"]);
    assert!(draft.to_setup(&data).is_some());
}

#[test]
fn the_player_is_on_one_seat() {
    let data = ScreenData::fake();
    let mut d = SetupDraft::new(&data);
    let humans = |d: &SetupDraft| d.seats.iter().filter(|s| s.control == SeatControl::Human).count();
    assert_eq!(d.seats[0].control, SeatControl::Human);
    // Stepping off the player hands the role on.
    d.cycle_control(0);
    assert_eq!((humans(&d), d.seats[0].control, d.seats[1].control), (1, SeatControl::Computer, SeatControl::Human));
    // Stepping another seat round to Player takes it from the first.
    d.cycle_control(0);
    d.cycle_control(0);
    assert_eq!((humans(&d), d.seats[0].control), (1, SeatControl::Human));
    // A smaller map keeps the choices and the player.
    let mut big = SetupDraft::new(&ScreenData::fake());
    let mut data4 = ScreenData::fake();
    data4.maps.reverse();
    big.set_map(&data4, 0);
    assert_eq!(big.seats.len(), 4);
    big.cycle_control(3);
    big.cycle_control(3);
    assert_eq!(big.seats[3].control, SeatControl::Human);
    big.set_map(&data4, 1);
    assert_eq!(big.seats.len(), 2);
    assert_eq!(humans(&big), 1, "the player's seat was cut: seat 0 takes the role");
}

#[test]
fn game_setup_screen_edits_the_draft_and_launch_starts_the_game() {
    let mut app = app();
    go(&mut app, Screen::Setup);
    assert_eq!(draft(&app).seats.len(), 2);
    // Player 2: Computer -> Idle -> Player (player 1 becomes a computer).
    press(&mut app, Act::Cycle(1, Col::Control));
    assert_eq!(draft(&app).seats[1].control, SeatControl::Idle);
    assert!(has_text(&mut app, "idle"));
    press(&mut app, Act::Cycle(1, Col::Control));
    assert_eq!((draft(&app).seats[0].control, draft(&app).seats[1].control), (SeatControl::Computer, SeatControl::Human));
    // Stepping off the player hands the role back to player 1.
    press(&mut app, Act::Cycle(1, Col::Control));
    assert_eq!((draft(&app).seats[0].control, draft(&app).seats[1].control), (SeatControl::Human, SeatControl::Computer));
    press(&mut app, Act::Cycle(1, Col::Team));
    assert_eq!(draft(&app).seats[1].team, 0);
    assert!(has_text(&mut app, "team 1"));

    press(&mut app, Act::Launch);
    let sent = started(&mut app);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].players.len(), 2);
    assert_eq!(sent[0].players[0].control, Control::Human);
    assert_eq!(sent[0].players[1].control, Control::Ai("skirmish".into()));
    assert_eq!(sent[0].players[1].team, TeamId(0));
    // The front end is the host's to leave: still on the screen.
    assert_eq!(screen(&app), Screen::Setup);
}

#[test]
fn map_select_picks_the_map_and_the_seats_follow() {
    let mut app = app();
    go(&mut app, Screen::Setup);
    assert!(has_text(&mut app, "Map: Two Bases  (2 players)"));
    press(&mut app, Act::PickMap);
    assert_eq!(count::<ListItem>(&mut app), 2);
    // Row 1 chosen by a click on the list; nothing changes in the draft until Select map.
    let row = app.world_mut().query::<(Entity, &ListItem)>().iter(app.world()).find(|(_, i)| i.0 == 1).unwrap().0;
    click(app.world_mut(), row);
    settle(&mut app);
    assert_eq!(app.world().resource::<Browse>().map, 1);
    assert_eq!(draft(&app).map, 0);
    assert_eq!(count::<MapList>(&mut app), 1);
    press(&mut app, Act::TakeMap);
    assert_eq!(screen(&app), Screen::Setup);
    assert_eq!(draft(&app).map, 1);
    assert_eq!(draft(&app).seats.len(), 4);
    assert!(has_text(&mut app, "Map: Four Corners  (4 players)"));
    // Player 4's row is there.
    assert!(has_text(&mut app, "Player 4"));
}

#[test]
fn no_maps_means_nothing_to_launch() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin));
    configure_sets(&mut app);
    app.insert_resource(ScreensConfig { own_camera: false, fit_scale: false, start: Screen::Setup })
        .init_resource::<Seen>()
        .add_systems(Last, record)
        .add_plugins(ScreensPlugin);
    app.world_mut().resource_mut::<NextState<AppState>>().set(AppState::FrontEnd);
    settle(&mut app);
    assert_eq!(screen(&app), Screen::Setup);
    assert!(has_text(&mut app, "No maps found."));
    let launch = disabled_button_named(&mut app, "Launch");
    click(app.world_mut(), launch);
    settle(&mut app);
    assert!(started(&mut app).is_empty());
}

#[test]
fn settings_sliders_and_toggles_write_the_settings() {
    let mut app = app();
    go(&mut app, Screen::Settings);
    assert_eq!(count::<Slider>(&mut app), 5);
    assert_eq!(count::<Toggle>(&mut app), 4);
    let music = app.world_mut().query::<(Entity, &SettingSlider)>().iter(app.world()).find(|(_, k)| k.0 == SettingKey::MusicVolume).unwrap().0;
    app.world_mut().get_mut::<Slider>(music).unwrap().value = 25.0;
    app.world_mut().write_message(SliderChanged { entity: music, value: 25.0 });
    let fs = app.world_mut().query::<(Entity, &SettingToggle)>().iter(app.world()).find(|(_, k)| k.0 == SettingKey::Fullscreen).unwrap().0;
    click(app.world_mut(), fs);
    settle(&mut app);
    let s = app.world().resource::<Settings>();
    assert_eq!((s.music, s.fullscreen, s.master), (25.0, true, 80.0));
    // Leaving and coming back shows what was set.
    go(&mut app, Screen::Main);
    go(&mut app, Screen::Settings);
    let music = app.world_mut().query::<(&Slider, &SettingSlider)>().iter(app.world()).find(|(_, k)| k.0 == SettingKey::MusicVolume).unwrap().0.value;
    assert_eq!(music, 25.0);
    let on = app.world_mut().query::<(&Toggle, &SettingToggle)>().iter(app.world()).find(|(_, k)| k.0 == SettingKey::Fullscreen).unwrap().0.on;
    assert!(on);
}

#[test]
fn lcars_setting_updates_the_shared_ui_preference() {
    let mut app = app();
    go(&mut app, Screen::Settings);
    assert!(has_text(&mut app, "LCARS interface"));
    assert!(has_text(&mut app, "Off uses the game/mod default."));
    assert!(!app.world().resource::<Settings>().lcars_ui);
    assert!(!app.world().resource::<crate::ui::profile::UiPreference>().lcars);

    let toggle = app.world_mut().query::<(Entity, &SettingToggle)>().iter(app.world()).find(|(_, k)| k.0 == SettingKey::LcarsUi).unwrap().0;
    app.world_mut().write_message(ToggleChanged { entity: toggle, on: true });
    settle(&mut app);
    assert!(app.world().resource::<Settings>().lcars_ui);
    assert!(app.world().resource::<crate::ui::profile::UiPreference>().lcars);
}

#[test]
fn lcars_toggle_syncs_profile_theme_and_settings_screen_in_one_update() {
    let root = a2_install();
    let vfs = sta_vfs::Vfs::mount_install(&root, None).unwrap();
    let mut app = app();
    app.world_mut().insert_resource(crate::viewer::Mounted(vfs));
    settle(&mut app);
    assert_eq!(*app.world().resource::<crate::ui::profile::UiProfile>(), crate::ui::profile::UiProfile::Armada2);
    go(&mut app, Screen::Settings);
    let old_roots: Vec<Entity> = app.world_mut().query_filtered::<Entity, With<ScreenRoot>>().iter(app.world()).collect();
    let old_accent = app.world().resource::<crate::ui::theme::Theme>().accent;
    let toggle = app.world_mut().query::<(Entity, &SettingToggle)>().iter(app.world()).find(|(_, k)| k.0 == SettingKey::LcarsUi).unwrap().0;
    app.world_mut().write_message(ToggleChanged { entity: toggle, on: true });
    app.update();

    assert!(app.world().resource::<Settings>().lcars_ui);
    assert!(app.world().resource::<crate::ui::profile::UiPreference>().lcars);
    assert_eq!(*app.world().resource::<crate::ui::profile::UiProfile>(), crate::ui::profile::UiProfile::Lcars);
    assert_eq!(app.world().resource::<crate::ui::theme::Theme>().accent, crate::ui::theme::Theme::default().accent);
    assert_ne!(old_accent, app.world().resource::<crate::ui::theme::Theme>().accent);
    let new_roots: Vec<Entity> = app.world_mut().query_filtered::<Entity, With<ScreenRoot>>().iter(app.world()).collect();
    assert_eq!(new_roots.len(), 1);
    assert_ne!(old_roots, new_roots, "theme change rebuilds the settings screen in the same update");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn mod_select_lists_the_mods_and_reports_the_choice() {
    let mut app = app();
    press(&mut app, Act::PickMod);
    assert_eq!(count::<ListItem>(&mut app), 3, "the base game and two mods");
    assert!(has_text(&mut app, "Balance Pack"));
    assert!(has_text(&mut app, "Fake Install"), "the base entry is named after the install");
    assert!(has_text(&mut app, "In use: Fake Install"));
    let first = app.world_mut().query::<&ListItem>().iter(app.world()).map(|i| i.0).min();
    assert_eq!(first, Some(0), "the list starts with the base entry");
    let row = app.world_mut().query::<(Entity, &ListItem)>().iter(app.world()).find(|(_, i)| i.0 == 2).unwrap().0;
    click(app.world_mut(), row);
    settle(&mut app);
    assert_eq!(count::<ModList>(&mut app), 1);
    assert!(has_text(&mut app, "Needs fx_balance"), "the picked mod's details show");
    assert!(has_text(&mut app, "Adds a cruiser."));
    press(&mut app, Act::UseMod);
    assert_eq!(screen(&app), Screen::Main);
    assert_eq!(app.world().resource::<SelectedMod>().0.as_deref(), Some("fx_ships"));
    assert_eq!(app.world().resource::<Seen>().chosen, [ModChosen(Some("fx_ships".into()))]);
    // Back on the screen the mod in use is picked; the base game is a choice too.
    press(&mut app, Act::PickMod);
    assert!(has_text(&mut app, "In use: fx_ships"));
    assert_eq!(app.world().resource::<Browse>().mod_row, 2);
    let base = app.world_mut().query::<(Entity, &ListItem)>().iter(app.world()).find(|(_, i)| i.0 == 0).unwrap().0;
    click(app.world_mut(), base);
    settle(&mut app);
    press(&mut app, Act::UseMod);
    assert_eq!(app.world().resource::<SelectedMod>().0, None);
    assert_eq!(app.world().resource::<Seen>().chosen.last(), Some(&ModChosen(None)), "choosing the base entry remounts with no mod");
    press(&mut app, Act::PickMod);
    assert!(has_text(&mut app, "In use: Fake Install"));
    assert_eq!(app.world().resource::<Browse>().mod_row, 0, "the base entry is current when no mod is mounted");
}

#[test]
fn installed_mods_come_from_the_data_folder() {
    let dir = std::env::temp_dir().join(format!("sta64-screens-mods-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let m = dir.join("Mods").join("Cool");
    std::fs::create_dir_all(&m).unwrap();
    std::fs::write(m.join("info.ini"), "[Mod]\nName=Cool Mod\nDescription=Very cool\nAuthors=Ann\nVersion=2.1\n").unwrap();
    std::fs::create_dir_all(dir.join("Mods").join("NotAMod")).unwrap();
    let (mods, notes) = ScreenData::mods_in(&dir);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(mods.len(), 1, "{notes:?}");
    assert_eq!((mods[0].folder.as_str(), mods[0].name.as_str(), mods[0].version.as_deref()), ("Cool", "Cool Mod", Some("2.1")));
    assert_eq!(notes.len(), 1);
}

#[test]
fn disabled_entries_do_not_click() {
    let mut app = app();
    let off = app.world_mut().query::<(Entity, &WidgetState)>().iter(app.world()).find(|(_, s)| **s == WidgetState::Disabled).unwrap().0;
    click(app.world_mut(), off);
    assert_eq!(app.world().resource::<Messages<ButtonClicked>>().len(), 0);
}

// --- Long lists: scroll, keys, close ---

fn many_maps_app(n: usize) -> App {
    let mut data = ScreenData::fake();
    let base = sta_content::fake::two_bases();
    data.maps = (0..n)
        .map(|i| {
            let mut m = base.clone();
            m.name = format!("fx_map_{i}");
            m.label = Some(format!("Map {i}"));
            MapEntry::new(m)
        })
        .collect();
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin));
    configure_sets(&mut app);
    app.insert_resource(data)
        .insert_resource(ScreensConfig { own_camera: false, fit_scale: false, start: Screen::Setup })
        .init_resource::<Seen>()
        .add_systems(Last, record)
        .add_plugins(ScreensPlugin);
    app.world_mut().resource_mut::<NextState<AppState>>().set(AppState::FrontEnd);
    settle(&mut app);
    press(&mut app, Act::PickMap);
    // No layout runs headless: say how tall the box was laid out.
    app.world_mut().resource_mut::<ListScroll>().view_h = 400.0;
    app
}

fn tap(app: &mut App, k: KeyCode) {
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(k);
    app.update();
    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
    keys.release(k);
    keys.clear();
    settle(app);
}

fn scroll_offset(app: &mut App) -> f32 {
    app.world_mut().query::<(&view::ScrollBox, &ScrollPosition)>().single(app.world()).unwrap().1.0.y
}

fn wheel(app: &mut App, unit: MouseScrollUnit, y: f32) {
    let window = app.world_mut().spawn_empty().id();
    app.world_mut().write_message(MouseWheel { unit, x: 0.0, y, window, phase: bevy::input::touch::TouchPhase::Moved });
    settle(app);
}

#[test]
fn a_long_map_list_is_bounded_by_the_window_and_scrolls() {
    let mut app = many_maps_app(250);
    assert_eq!(screen(&app), Screen::MapSelect);
    assert_eq!(count::<ListItem>(&mut app), 250);
    let node = app.world_mut().query::<(&view::ScrollBox, &Node)>().single(app.world()).unwrap().1.clone();
    assert_eq!(node.max_height, Val::Vh(view::LIST_MAX_VH), "the box is capped to a share of the window");
    const { assert!(view::LIST_MAX_VH < 100.0) };
    assert_eq!(node.overflow, Overflow::scroll_y());
    let row_h = app.world().resource::<crate::ui::theme::Theme>().list_row_h;
    assert!(250.0 * row_h > 400.0, "the list is taller than the box, so it must scroll");

    // End: the last row is picked and the box scrolled to show it.
    tap(&mut app, KeyCode::End);
    assert_eq!(app.world().resource::<Browse>().map, 249);
    let off = scroll_offset(&mut app);
    assert!((off - (250.0 * row_h - 400.0)).abs() < 0.01, "offset {off}");
    // Home: back at the top.
    tap(&mut app, KeyCode::Home);
    assert_eq!(app.world().resource::<Browse>().map, 0);
    assert_eq!(scroll_offset(&mut app), 0.0);
    // Down, past the bottom of the box: the pick stays in view.
    for _ in 0..40 {
        tap(&mut app, KeyCode::ArrowDown);
    }
    let sel = app.world().resource::<Browse>().map;
    assert_eq!(sel, 40);
    let off = scroll_offset(&mut app);
    let top = sel as f32 * row_h;
    assert!(top >= off && top + row_h <= off + 400.0 + 0.01, "row at {top} not in {off}..{}", off + 400.0);
    tap(&mut app, KeyCode::PageDown);
    assert!(app.world().resource::<Browse>().map > 40 + 10);
    tap(&mut app, KeyCode::PageUp);
    tap(&mut app, KeyCode::ArrowUp);
    assert!(app.world().resource::<Browse>().map < 60);
}

#[test]
fn the_wheel_scrolls_a_long_list_within_its_bounds() {
    let mut app = many_maps_app(250);
    let row_h = app.world().resource::<crate::ui::theme::Theme>().list_row_h;
    wheel(&mut app, MouseScrollUnit::Line, -2.0);
    assert!((scroll_offset(&mut app) - 6.0 * row_h).abs() < 0.01);
    wheel(&mut app, MouseScrollUnit::Pixel, 1.0e6);
    assert_eq!(scroll_offset(&mut app), 0.0, "never above the first row");
    wheel(&mut app, MouseScrollUnit::Pixel, -1.0e6);
    assert!((scroll_offset(&mut app) - (250.0 * row_h - 400.0)).abs() < 0.01, "never past the last row");
}

#[test]
fn enter_and_a_click_take_the_map_and_close_the_panel() {
    let mut app = many_maps_app(250);
    tap(&mut app, KeyCode::PageDown);
    let picked = app.world().resource::<Browse>().map;
    assert!(picked > 0);
    tap(&mut app, KeyCode::Enter);
    assert_eq!(screen(&app), Screen::Setup);
    assert_eq!(draft(&app).map, picked);

    press(&mut app, Act::PickMap);
    let row = app.world_mut().query::<(Entity, &ListItem)>().iter(app.world()).find(|(_, i)| i.0 == 7).unwrap().0;
    click(app.world_mut(), row);
    settle(&mut app);
    assert_eq!(screen(&app), Screen::MapSelect, "a click only picks");
    tap(&mut app, KeyCode::NumpadEnter);
    assert_eq!(screen(&app), Screen::Setup);
    assert_eq!(draft(&app).map, 7);
}

#[test]
fn escape_and_cancel_close_the_panel_without_changing_the_map() {
    let mut app = many_maps_app(250);
    tap(&mut app, KeyCode::End);
    assert_eq!(app.world().resource::<Browse>().map, 249);
    tap(&mut app, KeyCode::Escape);
    assert_eq!(screen(&app), Screen::Setup);
    assert_eq!(draft(&app).map, 0);

    press(&mut app, Act::PickMap);
    tap(&mut app, KeyCode::End);
    press(&mut app, Act::Goto(Screen::Setup));
    assert_eq!(screen(&app), Screen::Setup);
    assert_eq!(draft(&app).map, 0);
}

#[test]
fn the_mod_list_closes_on_escape_and_takes_on_enter() {
    let mut app = app();
    press(&mut app, Act::PickMod);
    tap(&mut app, KeyCode::ArrowDown);
    assert_eq!(app.world().resource::<Browse>().mod_row, 1);
    tap(&mut app, KeyCode::Escape);
    assert_eq!(screen(&app), Screen::Main);
    assert_eq!(app.world().resource::<SelectedMod>().0, None);
    press(&mut app, Act::PickMod);
    tap(&mut app, KeyCode::ArrowDown);
    tap(&mut app, KeyCode::Enter);
    assert_eq!(screen(&app), Screen::Main);
    assert_eq!(app.world().resource::<SelectedMod>().0.as_deref(), Some("fx_balance"));
}

#[test]
fn reveal_moves_the_offset_only_as_far_as_needed() {
    use super::reveal;
    assert_eq!(reveal(0.0, 3, 100, 10.0, 50.0), 0.0, "already in view");
    assert_eq!(reveal(0.0, 9, 100, 10.0, 50.0), 50.0, "scrolled down to the row's bottom");
    assert_eq!(reveal(200.0, 5, 100, 10.0, 50.0), 50.0, "scrolled up to the row's top");
    assert_eq!(reveal(0.0, 99, 100, 10.0, 50.0), 950.0, "never past the end");
}

fn race_strings(text: &str) -> crate::ui::i18n::Strings {
    let root = a2_install();
    std::fs::write(root.join("Data/Dynamic_Localized_Strings.h"), text).unwrap();
    let vfs = sta_vfs::Vfs::mount_install(&root, None).unwrap();
    let strings = crate::ui::i18n::Strings::load(&vfs, "english");
    std::fs::remove_dir_all(root).unwrap();
    strings
}

#[test]
fn race_captions_translate_display_keys_and_preserve_literal_fallbacks() {
    let mut data = ScreenData::fake();
    let mut translated = race("fx", Some(0));
    translated.stem = "human".into();
    translated.display_key = Some("GUI_FACTION".into());
    // A nonplayable ODF stem must not shadow the canonical race emitted by setup.
    let mut shadow = race("observer", None);
    shadow.stem = "fx".into();
    shadow.display_key = Some("Wrong observer label".into());
    let mut literal = race("other", Some(1));
    literal.display_key = Some("Fancy Faction".into());
    let mut missing = race("missing", Some(2));
    missing.display_key = Some("GUI_NOT_PRESENT".into());
    data.races = Arc::new(Races::from_defs(vec![shadow, translated, literal, missing, race("bare", Some(3))]));
    let strings = race_strings("\"GUI_FACTION\", \"The Federation\"");
    assert_eq!(data.race_label("fx", Some(&strings)), "The Federation");
    assert_eq!(data.race_label("other", Some(&strings)), "Fancy Faction");
    assert_eq!(data.race_label("missing", Some(&strings)), "GUI_NOT_PRESENT");
    assert_eq!(data.race_label("bare", Some(&strings)), "bare");
    assert_eq!(data.race_label("unknown", Some(&strings)), "unknown");
    assert_eq!(data.race_label("fx", None), "GUI_FACTION");
}

#[test]
fn setup_renders_race_captions_and_refreshes_strings_without_replacing_ids() {
    let mut app = app();
    let mut data = ScreenData::fake();
    let mut faction = race("fx", Some(0));
    faction.display_key = Some("GUI_FACTION".into());
    data.races = Arc::new(Races::from_defs(vec![faction]));
    app.insert_resource(data);
    app.insert_resource(race_strings("\"GUI_FACTION\", \"The Federation\""));
    go(&mut app, Screen::Setup);
    assert!(has_text(&mut app, "The Federation"));
    assert!(!has_text(&mut app, "GUI_FACTION"));
    assert!(app.world().resource::<SetupDraft>().seats.iter().all(|seat| seat.race == "fx"));
    app.insert_resource(race_strings("\"GUI_FACTION\", \"Updated Faction\""));
    settle(&mut app);
    assert!(has_text(&mut app, "Updated Faction"));
    assert!(!has_text(&mut app, "The Federation"));
    let setup = app.world().resource::<SetupDraft>().to_setup(app.world().resource::<ScreenData>()).unwrap();
    assert!(setup.players.iter().all(|player| player.race == "fx"));
}

#[test]
fn installed_race_display_keys_resolve_federation_caption() {
    let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
        eprintln!("SKIP: set STA64_TEST_INSTALL and STA64_TEST_MOD for installed race captions");
        return;
    };
    let active = std::env::var("STA64_TEST_MOD").ok();
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), active.as_deref()).unwrap();
    let (index, _) = sta_content::OdfIndex::build(&vfs);
    let (races, warnings) = Races::load(&vfs, &index);
    assert!(warnings.is_empty(), "{warnings:?}");
    let faction = races.playable().into_iter().find(|race| ["federation", "unitedfederation"].contains(&race.name.as_str())).unwrap();
    let id = faction.name.clone();
    let display_key = faction.display_key.clone().expect("installed federation displayKey");
    let mut data = ScreenData::fake();
    data.races = Arc::new(races);
    let strings = crate::ui::i18n::Strings::load(&vfs, "english");
    assert_eq!(data.race_label(&id, Some(&strings)), "Federation");
    eprintln!("race caption: {} id={id} displayKey={display_key} -> Federation", active.as_deref().unwrap_or("base"));
}
