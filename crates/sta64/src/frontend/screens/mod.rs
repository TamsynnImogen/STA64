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
use bevy::window::PrimaryWindow;
use sta_sim::setup::GameSetup;

mod data;
pub mod frame;
#[cfg(test)]
mod tests;
mod view;

pub use data::{Browse, MAX_SEATS, MapEntry, ModEntry, SKIRMISH_AI, ScreenData, Seat, SeatControl, SelectedMod, SettingKey, Settings, SetupDraft};
pub use view::{Act, ScreenRoot};

use crate::app::{AppState, GameSet};
use crate::ui::theme::{ButtonClicked, ListSelected, SliderChanged, ThemePlugin, ToggleChanged};
use view::{MapList, ModList, SettingSlider, SettingToggle};

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
    /// Spawn a 2D camera while in the front end.
    pub own_camera: bool,
    /// Scale the UI to the primary window (`UiScale` = height / 1080).
    pub fit_scale: bool,
    /// The screen shown when the front end is entered.
    pub start: Screen,
}

impl Default for ScreensConfig {
    fn default() -> Self {
        ScreensConfig { own_camera: true, fit_scale: true, start: Screen::Main }
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
            app.insert_resource(ScreenData { content: std::sync::Arc::new(empty), maps: Vec::new(), mods: Vec::new() });
        }
        if !app.world().contains_resource::<State<AppState>>() {
            app.init_state::<AppState>();
        }
        app.init_state::<Screen>()
            .init_resource::<ScreensConfig>()
            .init_resource::<Settings>()
            .init_resource::<SelectedMod>()
            .init_resource::<Browse>()
            .add_message::<StartGame>()
            .add_message::<ModChosen>()
            .add_systems(Startup, make_draft)
            .add_systems(OnEnter(AppState::FrontEnd), enter_front_end)
            .add_systems(
                Update,
                (fit_scale, follow_data, on_buttons, on_lists, on_settings, rebuild)
                    .chain()
                    .in_set(GameSet::Ui)
                    .run_if(in_state(AppState::FrontEnd)),
            );
    }
}

fn make_draft(mut commands: Commands, data: Res<ScreenData>, draft: Option<Res<SetupDraft>>) {
    if draft.is_none() {
        commands.insert_resource(SetupDraft::new(&data));
    }
}

fn enter_front_end(mut commands: Commands, config: Res<ScreensConfig>, theme: Res<crate::ui::theme::Theme>, mut next: ResMut<NextState<Screen>>) {
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
) {
    for c in sliders.read() {
        if let Ok(k) = slider_keys.get(c.entity) {
            settings.set_number(k.0, c.value);
        }
    }
    for c in toggles.read() {
        if let Ok(k) = toggle_keys.get(c.entity) {
            settings.set_flag(k.0, c.on);
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
) {
    let Some(draft) = draft else { return };
    if !(screen.is_changed() || draft.is_changed() || browse.is_changed() || selected.is_changed() || data.is_changed() || theme.is_changed()) {
        return;
    }
    for e in &roots {
        commands.entity(e).despawn();
    }
    let ctx = view::Ctx { theme: &theme, data: &data, draft: &draft, browse: &browse, settings: &settings, selected: &selected };
    view::spawn(&mut commands, *screen.get(), &ctx, camera.map(|c| c.0));
}
