//! The front-end screens (UI-07): main, single player, game setup (Instant
//! Action) with map select, settings and mod select, in the LCARS kit
//! (UI-03), laid out as Refit's `ui/MENUS.md` lists them.
//!
//! - **State.** [`Screen`] is the screen shown while `AppState::FrontEnd`;
//!   set `NextState<Screen>` to go to one. Each screen's widgets are spawned
//!   when it is entered and again when what it shows changes.
//! - **In.** [`ScreenData`] (content, maps, mods) is the host's to insert;
//!   [`SetupDraft`], [`Settings`], [`SelectedMod`] and [`Browse`] are the
//!   player's choices, kept here.
//! - **Out.** Launch writes [`StartGame`] with a [`GameSetup`]; Use mod writes
//!   [`ModChosen`]; Exit writes `AppExit`. INT-M4 connects the first two to the
//!   match start and the remount. Nothing here changes `AppState`.
//! - **Camera.** The plugin spawns a 2D camera unless [`ScreensConfig::own_camera`]
//!   is off (the menu scene, UI-06, has the camera then).

// The screens are for INT-M4 to start; the binary does not yet.
#![allow(dead_code, unused_imports)]

use bevy::prelude::*;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::window::PrimaryWindow;
use sta_sim::setup::GameSetup;

mod data;
pub mod frame;
pub mod stock;
#[cfg(test)]
mod tests;
mod view;

pub use data::{Browse, MAX_SEATS, MapEntry, ModEntry, SKIRMISH_AI, ScreenData, Seat, SeatControl, SelectedMod, SettingKey, Settings, SetupDraft};
pub use view::{Act, ScreenRoot};

use crate::app::{AppState, GameSet};
use crate::ui::theme::{ButtonClicked, ListSelected, SliderChanged, ThemePlugin, ToggleChanged};
use view::{MapList, ModList, ScrollBox, SettingSlider, SettingToggle};

/// The screen shown in `AppState::FrontEnd`.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Screen {
    #[default]
    Main,
    SinglePlayer,
    /// Game setup, as Instant Action opens it.
    Setup,
    MapSelect,
    Settings,
    ModSelect,
    /// A campaign's mission list (UI-25, `frontend::campaign`).
    Missions,
    /// A mission's briefing (UI-25).
    Briefing,
    /// What follows a mission (UI-25).
    Debrief,
}

/// Where the open list (map or mod select) is scrolled, in logical pixels.
/// Kept here because the screen is drawn again whenever the pick changes.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct ListScroll {
    pub offset: f32,
    /// The box's height as last laid out; 0 before the first layout.
    pub view_h: f32,
    /// Bring the picked row into view once the box has a size.
    pub pending: bool,
}

/// The scroll offset that shows row `index` (of `count`, `row_h` tall each) in
/// a box `view_h` tall, moving `offset` as little as possible.
pub fn reveal(offset: f32, index: usize, count: usize, row_h: f32, view_h: f32) -> f32 {
    let top = index as f32 * row_h;
    let mut o = offset;
    if top < o {
        o = top;
    } else if top + row_h > o + view_h {
        o = top + row_h - view_h;
    }
    o.clamp(0.0, (count as f32 * row_h - view_h).max(0.0))
}

/// A game setup column that a click steps through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Col {
    Control,
    Race,
    Team,
}

/// Launch was pressed: start a match from this setup. INT-M4 connects it to
/// INT-M3's match start (loading screen, then `AppState::InGame`).
#[derive(Message, Debug, Clone)]
pub struct StartGame(pub GameSetup);

/// Use mod was pressed: `Some(folder)` for a mod, `None` for the base game.
/// The host remounts the install with it (`Vfs::mount_install`'s `active_mod`).
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct ModChosen(pub Option<String>);

/// How the plugin sits in the host.
#[derive(Resource, Debug, Clone)]
pub struct ScreensConfig {
    /// Stock arrangement (UI-22, default) or UI-07's plain layouts.
    pub layout: stock::ScreenLayout,
    /// Spawn a 2D camera while in the front end.
    pub own_camera: bool,
    /// Scale the UI to the primary window (`UiScale` = height / 1080).
    pub fit_scale: bool,
    /// The screen shown when the front end is entered.
    pub start: Screen,
}

impl Default for ScreensConfig {
    fn default() -> Self {
        ScreensConfig { layout: stock::ScreenLayout::Stock, own_camera: true, fit_scale: true, start: Screen::Main }
    }
}

/// The camera the screens are drawn by, when the host's is not the default.
#[derive(Resource, Debug, Clone, Copy)]
pub struct ScreensCamera(pub Entity);

pub struct ScreensPlugin;

impl Plugin for ScreensPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<ThemePlugin>() {
            app.add_plugins(ThemePlugin);
        }
        if !app.world().contains_resource::<ScreenData>() {
            let empty = sta_content::ContentDb::builder().build().expect("empty content");
            app.insert_resource(ScreenData { content: std::sync::Arc::new(empty), races: std::sync::Arc::new(sta_content::Races::default()), maps: Vec::new(), mods: Vec::new(), base_name: None });
        }
        if !app.world().contains_resource::<State<AppState>>() {
            app.init_state::<AppState>();
        }
        app.init_state::<Screen>()
            .init_resource::<ScreensConfig>()
            .init_resource::<Settings>()
            .init_resource::<crate::ui::profile::UiPreference>()
            .init_resource::<SelectedMod>()
            .init_resource::<Browse>()
            .init_resource::<ListScroll>()
            .init_resource::<stock::StockBox>()
            .init_resource::<stock::OptionsTab>()
            .init_resource::<stock::Refocus>()
            .init_resource::<ButtonInput<KeyCode>>()
            .add_message::<MouseWheel>()
            .add_message::<StartGame>()
            .add_message::<ModChosen>()
            .add_message::<crate::frontend::campaign::StartMission>()
            .add_systems(Startup, make_draft)
            .add_systems(OnEnter(AppState::FrontEnd), enter_front_end)
            .add_systems(OnEnter(Screen::MapSelect), open_list)
            .add_systems(OnEnter(Screen::ModSelect), open_list)
            .add_systems(
                Update,
                on_settings.in_set(GameSet::Ui).before(crate::ui::profile::ProfileSync).run_if(in_state(AppState::FrontEnd)),
            )
            .add_systems(
                Update,
                (fit_scale, stock::track_box, follow_data, on_keys, on_wheel, track_scroll, on_buttons, on_lists, crate::frontend::campaign::on_buttons, stock::on_dropdowns, stock::track_tab, rebuild, crate::frontend::campaign::rebuild, stock::restore_focus)
                    .chain()
                    .in_set(GameSet::Ui)
                    .after(crate::ui::profile::ProfileSync)
                    .after(on_settings)
                    .run_if(in_state(AppState::FrontEnd)),
            );
    }
}

fn make_draft(mut commands: Commands, data: Res<ScreenData>, draft: Option<Res<SetupDraft>>) {
    if draft.is_none() {
        commands.insert_resource(SetupDraft::new(&data));
    }
}

pub(crate) fn enter_front_end(mut commands: Commands, config: Res<ScreensConfig>, theme: Res<crate::ui::theme::Theme>, mut next: ResMut<NextState<Screen>>) {
    next.set(config.start);
    if config.own_camera {
        commands.spawn((Camera2d, Camera { clear_color: ClearColorConfig::Custom(theme.panel), ..default() }, DespawnOnExit(AppState::FrontEnd)));
    }
}

fn fit_scale(config: Res<ScreensConfig>, window: Query<&Window, With<PrimaryWindow>>, scale: Option<ResMut<UiScale>>) {
    let (true, Ok(w), Some(mut scale)) = (config.fit_scale, window.single(), scale) else { return };
    let want = w.height() / 1080.0;
    if (scale.0 - want).abs() > 1e-4 {
        scale.0 = want;
    }
}

/// New maps from the host: keep the draft inside them.
fn follow_data(data: Res<ScreenData>, draft: Option<ResMut<SetupDraft>>, mut browse: ResMut<Browse>, mut first: Local<bool>) {
    if !std::mem::replace(&mut *first, true) || !data.is_changed() {
        return;
    }
    if let Some(mut draft) = draft {
        let map = draft.map;
        draft.set_map(&data, map);
    }
    browse.map = browse.map.min(data.maps.len().saturating_sub(1));
    browse.mod_row = browse.mod_row.min(data.mods.len());
}

/// A list opens at the top, then moves to show the pick.
fn open_list(mut scroll: ResMut<ListScroll>) {
    scroll.offset = 0.0;
    scroll.pending = true;
}

/// Keys in the lists: Up, Down, PageUp, PageDown, Home and End move the pick
/// and keep it in view; Enter takes it; Esc leaves without taking it. They
/// press the screen's own buttons, so a key does what the click does.
#[allow(clippy::too_many_arguments)]
fn on_keys(
    keys: Res<ButtonInput<KeyCode>>,
    screen: Res<State<Screen>>,
    theme: Res<crate::ui::theme::Theme>,
    boxes: Query<&ScrollBox>,
    acts: Query<(Entity, &Act)>,
    mut browse: ResMut<Browse>,
    mut scroll: ResMut<ListScroll>,
    mut clicks: MessageWriter<ButtonClicked>,
) {
    let (take, back) = match screen.get() {
        Screen::MapSelect => (Act::TakeMap, Act::Goto(Screen::Setup)),
        Screen::ModSelect => (Act::UseMod, Act::Goto(Screen::Main)),
        _ => return,
    };
    let press = |want: Act| acts.iter().find(|(_, a)| **a == want).map(|(e, _)| e);
    if keys.just_pressed(KeyCode::Escape) {
        if let Some(e) = press(back) {
            clicks.write(ButtonClicked(e));
        }
        return;
    }
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter) {
        if let Some(e) = press(take) {
            clicks.write(ButtonClicked(e));
        }
        return;
    }
    let Ok(b) = boxes.single() else { return };
    if b.count == 0 {
        return;
    }
    let page = if scroll.view_h > 0.0 { (scroll.view_h / theme.list_row_h).floor().max(1.0) as usize } else { 10 };
    let last = b.count - 1;
    let cur = b.selected.unwrap_or(0).min(last);
    let want = if keys.just_pressed(KeyCode::ArrowDown) {
        (cur + 1).min(last)
    } else if keys.just_pressed(KeyCode::ArrowUp) {
        cur.saturating_sub(1)
    } else if keys.just_pressed(KeyCode::PageDown) {
        (cur + page).min(last)
    } else if keys.just_pressed(KeyCode::PageUp) {
        cur.saturating_sub(page)
    } else if keys.just_pressed(KeyCode::Home) {
        0
    } else if keys.just_pressed(KeyCode::End) {
        last
    } else {
        return;
    };
    let row = match screen.get() {
        Screen::MapSelect => &mut browse.map,
        _ => &mut browse.mod_row,
    };
    if *row != want || b.selected.is_none() {
        *row = want;
    }
    if scroll.view_h > 0.0 {
        scroll.offset = reveal(scroll.offset, want, b.count, theme.list_row_h, scroll.view_h);
    } else {
        scroll.pending = true;
    }
}

/// The mouse wheel scrolls the open list.
fn on_wheel(mut wheel: MessageReader<MouseWheel>, theme: Res<crate::ui::theme::Theme>, boxes: Query<&ScrollBox>, mut scroll: ResMut<ListScroll>) {
    let Ok(b) = boxes.single() else {
        wheel.clear();
        return;
    };
    for w in wheel.read() {
        let dy = match w.unit {
            MouseScrollUnit::Line => w.y * theme.list_row_h * 3.0,
            MouseScrollUnit::Pixel => w.y,
        };
        let max = (b.count as f32 * theme.list_row_h - scroll.view_h).max(0.0);
        scroll.offset = (scroll.offset - dy).clamp(0.0, max);
    }
}

/// Keep the box's scroll and [`ListScroll`] in step: the offset goes to the
/// box, and the box's laid-out height comes back.
fn track_scroll(theme: Res<crate::ui::theme::Theme>, mut boxes: Query<(&ScrollBox, &ComputedNode, &mut ScrollPosition)>, mut scroll: ResMut<ListScroll>) {
    let Ok((b, node, mut pos)) = boxes.single_mut() else { return };
    let view_h = node.size.y * node.inverse_scale_factor;
    if view_h > 0.0 && (scroll.view_h - view_h).abs() > 0.01 {
        scroll.view_h = view_h;
    }
    if scroll.pending && view_h > 0.0 {
        scroll.pending = false;
        if let Some(i) = b.selected {
            scroll.offset = reveal(scroll.offset, i, b.count, theme.list_row_h, view_h);
        }
    }
    if (pos.0.y - scroll.offset).abs() > 0.01 {
        // The wheel and keys write `offset`.
        pos.0.y = scroll.offset;
    }
}

#[allow(clippy::too_many_arguments)]
fn on_buttons(
    mut clicks: MessageReader<ButtonClicked>,
    acts: Query<&Act>,
    mut next: ResMut<NextState<Screen>>,
    data: Res<ScreenData>,
    draft: Option<ResMut<SetupDraft>>,
    mut browse: ResMut<Browse>,
    mut selected: ResMut<SelectedMod>,
    mut start: MessageWriter<StartGame>,
    mut chosen: MessageWriter<ModChosen>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(mut draft) = draft else { return };
    for ButtonClicked(e) in clicks.read() {
        let Ok(&act) = acts.get(*e) else { continue };
        match act {
            Act::Goto(s) => {
                next.set(s);
            }
            Act::PickMap => {
                browse.map = draft.map;
                next.set(Screen::MapSelect);
            }
            Act::PickMod => {
                browse.mod_row = selected.0.as_ref().and_then(|f| data.mods.iter().position(|m| m.folder.eq_ignore_ascii_case(f))).map_or(0, |i| i + 1);
                next.set(Screen::ModSelect);
            }
            Act::TakeMap => {
                draft.set_map(&data, browse.map);
                next.set(Screen::Setup);
            }
            Act::UseMod => {
                let folder = browse.mod_row.checked_sub(1).and_then(|i| data.mods.get(i)).map(|m| m.folder.clone());
                selected.0.clone_from(&folder);
                chosen.write(ModChosen(folder));
                next.set(Screen::Main);
            }
            Act::Cycle(row, Col::Control) => draft.cycle_control(row),
            Act::Cycle(row, Col::Race) => draft.cycle_race(&data, row),
            Act::Cycle(row, Col::Team) => draft.cycle_team(row),
            Act::CycleResources => draft.cycle_resources(),
            Act::CycleOfficerLimit => draft.cycle_officer_limit(&data),
            Act::Launch => {
                if let Some(setup) = draft.to_setup(&data) {
                    start.write(StartGame(setup));
                }
            }
            Act::Exit => {
                exit.write(AppExit::Success);
            }
        }
    }
}

fn on_lists(mut picks: MessageReader<ListSelected>, maps: Query<(), With<MapList>>, mods: Query<(), With<ModList>>, mut browse: ResMut<Browse>) {
    for p in picks.read() {
        if maps.contains(p.list) {
            browse.map = p.index;
        } else if mods.contains(p.list) {
            browse.mod_row = p.index;
        }
    }
}

fn on_settings(
    mut sliders: MessageReader<SliderChanged>,
    mut toggles: MessageReader<ToggleChanged>,
    slider_keys: Query<&SettingSlider>,
    toggle_keys: Query<&SettingToggle>,
    mut settings: ResMut<Settings>,
    mut ui_preference: ResMut<crate::ui::profile::UiPreference>,
) {
    for c in sliders.read() {
        if let Ok(k) = slider_keys.get(c.entity) {
            settings.set_number(k.0, c.value);
        }
    }
    for c in toggles.read() {
        if let Ok(k) = toggle_keys.get(c.entity) {
            settings.set_flag(k.0, c.on);
            if k.0 == SettingKey::LcarsUi {
                ui_preference.lcars = c.on;
            }
        }
    }
}

/// Draw the current screen again when the screen or what it shows changed.
/// Settings are not a trigger: the sliders and toggles move themselves.
#[allow(clippy::too_many_arguments)]
fn rebuild(
    mut commands: Commands,
    roots: Query<Entity, With<ScreenRoot>>,
    screen: Res<State<Screen>>,
    theme: Res<crate::ui::theme::Theme>,
    data: Res<ScreenData>,
    draft: Option<Res<SetupDraft>>,
    browse: Res<Browse>,
    settings: Res<Settings>,
    selected: Res<SelectedMod>,
    camera: Option<Res<ScreensCamera>>,
    scroll: Res<ListScroll>,
    config: Res<ScreensConfig>,
    stock_box: Res<stock::StockBox>,
    options_tab: Res<stock::OptionsTab>,
    strings: Option<Res<crate::ui::i18n::Strings>>,
    campaigns: Option<Res<crate::frontend::campaign::Campaigns>>,
) {
    let Some(draft) = draft else { return };
    if !(screen.is_changed() || campaigns.as_ref().is_some_and(|c| c.is_changed()) || draft.is_changed() || browse.is_changed() || selected.is_changed() || data.is_changed() || theme.is_changed() || stock_box.is_changed() || config.is_changed() || strings.as_ref().is_some_and(|s| s.is_changed())) {
        return;
    }
    for e in &roots {
        commands.entity(e).despawn();
    }
    let ctx = view::Ctx { theme: &theme, data: &data, strings: strings.as_deref(), draft: &draft, browse: &browse, settings: &settings, selected: &selected, scroll: &scroll, layout: config.layout, stock_box: stock_box.0, options_tab: options_tab.0, campaigns: campaigns.as_deref() };
    view::spawn(&mut commands, *screen.get(), &ctx, camera.map(|c| c.0));
}
