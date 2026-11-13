//! The in-game menu (UI-34, the minimum of UI-20): a modal over the running match with
//! **Resume**, **Options**, **Exit to main menu** and **Quit to desktop**.
//!
//! # What the player does
//!
//! Esc (the rebindable action [`Action::GameMenu`]; the stock games bind `mainMenu` to Esc in
//! their `Input.map`, read from the Armada II, Fleet Operations and Roots installs, and Armada I
//! has no such line) opens it. Esc again closes it (on the Options page and on a question it goes
//! back one step). The top-right icon row of UI-20 opens it by writing [`OpenInGameMenu`].
//!
//! - **Resume** closes it. **Options** shows the front end's options panel
//!   ([`options_body`](crate::frontend::screens::stock::options_body)): the same sliders and
//!   toggles, written to the same [`Settings`] resource that UI-29 saves, so a change persists.
//!   **Exit to main menu** asks, then writes [`LeaveMatch`]; **Quit to desktop** asks, then
//!   writes `AppExit`.
//! - Single player pauses the sim while the menu is open (the stock game does; the evidence is
//!   the original's behaviour, not measured here). A networked match must not pause (lockstep):
//!   whoever starts one inserts [`NetworkedMatch`] and the menu then leaves the clock alone.
//! - The map's selection, orders, camera and the HUD placement modes do not see the mouse or keys
//!   while it is open (the overlay takes the pointer; the input chains wait on [`menu_closed`]).
//!
//! # Leaving
//!
//! The one way out of a match is [`LeaveMatch`] (what F10 writes too): `game::flow` takes the
//! state back to `FrontEnd`, and everything of the match goes with the state change
//! (`OnEnter(FrontEnd)`: `flow::clear_match`, `map::clear_match`, `mission_host::finish_mission`,
//! this module's reset on `OnExit(InGame)`). A campaign mission left this way has no outcome, so
//! `finish_mission` records an abandon (a loss, progress unchanged), exactly as F10 does.
//! [`leave_current_match`] is the function to call. **Multiplayer hooks in there:** the net match
//! (INT-M5) listens to `LeaveMatch` to leave its session (`net_match::net_duties` already writes
//! it for an ended game) and drops its `NetGame` with `clear_net` on `OnEnter(FrontEnd)`; it also
//! inserts [`NetworkedMatch`] when it links and removes it there.
//!
//! # Layout
//!
//! The regions are fractions of the stock box (UI-22's `fit_box`, 4:3 to 16:9 centred in the
//! window) over the 800x600 design screen. **Arranged**, all of them: the stock in-mission menu is
//! a dialog of the executable whose only measured point (Refit's `ui/MENUS.md`) is Return to Game at
//! design (91, 579); the rest of its column is not measured, and a column in a corner is not what
//! a modal over a match wants, so this menu is centred. Captions: the mounted `label.map` where
//! it has the key (`return`, `options`), translated English otherwise.
//!
//! # Runs only in a match
//!
//! Every system here is gated on `AppState::InGame` (the state, not resources that exist only
//! during a match) except the settings-to-volume wiring, which is harmless anywhere, and the
//! reset on leaving.

use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::math::{Rect, Vec2};
use bevy::prelude::*;

use crate::app::{AppState, GameSet};
use crate::clock::AnimClock;
use crate::frontend::screens::Settings;
use crate::frontend::screens::stock::{self, Grade, OptionsTab, Region, StockBox};
use crate::game::flow::LeaveMatch;
use crate::input::keys::{Action, KeyBindings};
use crate::sim_link::SimLink;
use crate::ui::i18n::{Strings, tr};
use crate::ui::theme::{ButtonClicked, Theme, ThemePlugin, kit_button, kit_text};

/// Open the in-game menu. The HUD's menu button (UI-20) writes this; so does Esc.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenInGameMenu;

/// Present while the match is a networked one: the menu then never pauses the sim.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct NetworkedMatch;

/// The page of the menu that is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MenuPage {
    #[default]
    Main,
    Options,
    /// "Leave this match?"
    ConfirmExit,
    /// "Quit to the desktop?"
    ConfirmQuit,
}

/// Whether the menu is open and where it is. Always present once the plugin is added; closed
/// outside a match.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InGameMenu {
    pub open: bool,
    pub page: MenuPage,
}

/// Run condition for the map's input systems: the menu is not over the match.
pub fn menu_closed(menu: Option<Res<InGameMenu>>) -> bool {
    !menu.is_some_and(|m| m.open)
}

/// Leave the current match for the main menu: single player, a campaign mission (recorded as an
/// abandon) and, once INT-M5 listens, a networked match. The only place a player-initiated leave
/// is written; the flow does the rest.
pub fn leave_current_match(leave: &mut MessageWriter<LeaveMatch>) {
    leave.write(LeaveMatch);
}

/// What a button of the menu does.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAct {
    Resume,
    Options,
    /// Back from Options, or "No" on a question.
    Back,
    AskExit,
    AskQuit,
    /// "Yes" on the question the page asks.
    Confirm,
}

/// Each part of the menu, by what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuSlot {
    Panel,
    Title,
    Resume,
    Options,
    Exit,
    Quit,
    OptionsPanel,
    OptionsTitle,
    OptionsBody,
    OptionsBack,
    ConfirmPanel,
    ConfirmTitle,
    ConfirmLine1,
    ConfirmLine2,
    Yes,
    No,
}

const fn a(x: f32, y: f32, w: f32, h: f32) -> Region {
    Region { x, y, w, h, grade: Grade::Arranged }
}

/// The region of `slot` on the 800x600 design screen. All arranged (see the module docs).
pub const fn region(slot: MenuSlot) -> Region {
    match slot {
        MenuSlot::Panel => a(260.0, 160.0, 280.0, 290.0),
        MenuSlot::Title => a(280.0, 174.0, 240.0, 34.0),
        MenuSlot::Resume => a(280.0, 226.0, 240.0, 36.0),
        MenuSlot::Options => a(280.0, 274.0, 240.0, 36.0),
        MenuSlot::Exit => a(280.0, 322.0, 240.0, 36.0),
        MenuSlot::Quit => a(280.0, 370.0, 240.0, 36.0),
        MenuSlot::OptionsPanel => a(80.0, 40.0, 640.0, 520.0),
        MenuSlot::OptionsTitle => a(100.0, 54.0, 600.0, 34.0),
        MenuSlot::OptionsBody => a(100.0, 100.0, 600.0, 360.0),
        MenuSlot::OptionsBack => a(300.0, 492.0, 200.0, 36.0),
        MenuSlot::ConfirmPanel => a(220.0, 210.0, 360.0, 190.0),
        MenuSlot::ConfirmTitle => a(240.0, 222.0, 320.0, 34.0),
        MenuSlot::ConfirmLine1 => a(240.0, 262.0, 320.0, 26.0),
        MenuSlot::ConfirmLine2 => a(240.0, 290.0, 320.0, 26.0),
        MenuSlot::Yes => a(240.0, 336.0, 150.0, 36.0),
        MenuSlot::No => a(410.0, 336.0, 150.0, 36.0),
    }
}

/// The slots each page shows.
#[cfg(test)]
pub fn slots_of(page: MenuPage) -> &'static [MenuSlot] {
    match page {
        MenuPage::Main => &[MenuSlot::Panel, MenuSlot::Title, MenuSlot::Resume, MenuSlot::Options, MenuSlot::Exit, MenuSlot::Quit],
        MenuPage::Options => &[MenuSlot::OptionsPanel, MenuSlot::OptionsTitle, MenuSlot::OptionsBody, MenuSlot::OptionsBack],
        MenuPage::ConfirmExit | MenuPage::ConfirmQuit => &[MenuSlot::ConfirmPanel, MenuSlot::ConfirmTitle, MenuSlot::ConfirmLine1, MenuSlot::ConfirmLine2, MenuSlot::Yes, MenuSlot::No],
    }
}

/// The root of the menu's nodes.
#[derive(Component)]
pub struct MenuRoot;

/// A node placed in its slot, for tests and layout checks.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuPlaced(pub MenuSlot);

type Box_<'a> = RelatedSpawnerCommands<'a, ChildOf>;

/// The mounted `label.map`'s text for `key`, else the translated `fallback`.
fn caption(strings: Option<&Strings>, key: &str, fallback: &str) -> String {
    strings.and_then(|s| s.native_label(key)).filter(|t| !t.trim().is_empty()).map_or_else(|| tr(fallback), str::to_owned)
}

fn rect(slot: MenuSlot, b: Vec2) -> Rect {
    region(slot).px(b)
}

/// `content` in `slot`, centred vertically in it.
fn put(p: &mut Box_, slot: MenuSlot, b: Vec2, content: impl Bundle) {
    let r = rect(slot, b);
    p.spawn((
        MenuPlaced(slot),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(r.min.x),
            top: Val::Px(r.min.y),
            width: Val::Px(r.width()),
            height: Val::Px(r.height()),
            align_items: AlignItems::Center,
            ..default()
        },
        children![content],
    ));
}

fn button(p: &mut Box_, theme: &Theme, slot: MenuSlot, b: Vec2, text: &str, act: MenuAct) {
    put(p, slot, b, (kit_button(theme, text, rect(slot, b).width()), act));
}

fn title(p: &mut Box_, theme: &Theme, slot: MenuSlot, b: Vec2, text: &str) {
    put(p, slot, b, kit_text(theme, theme.caps(text, theme.caps_titles), theme.title_size, theme.title));
}

fn line(p: &mut Box_, theme: &Theme, slot: MenuSlot, b: Vec2, text: &str) {
    put(p, slot, b, kit_text(theme, text, theme.text_size, theme.text));
}

/// The panel behind a page, drawn under its parts.
fn panel(p: &mut Box_, theme: &Theme, slot: MenuSlot, b: Vec2) {
    let r = rect(slot, b);
    p.spawn((
        MenuPlaced(slot),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(r.min.x),
            top: Val::Px(r.min.y),
            width: Val::Px(r.width()),
            height: Val::Px(r.height()),
            border: UiRect::all(Val::Px(2.0)),
            ..default()
        },
        BackgroundColor(theme.panel),
        BorderColor::all(theme.accent2),
    ));
}

/// What the menu is made from, gathered so the drawing is a function of values.
struct Parts<'a> {
    theme: &'a Theme,
    settings: &'a Settings,
    strings: Option<&'a Strings>,
    tab: usize,
    mission: bool,
}

fn spawn_menu(commands: &mut Commands, page: MenuPage, parts: &Parts, b: Vec2) {
    let t = parts.theme;
    commands
        .spawn((
            MenuRoot,
            DespawnOnExit(AppState::InGame),
            GlobalZIndex(800),
            // The whole window: dims the match and takes the pointer, so nothing under it is hit.
            Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
        ))
        .with_children(|p| {
            p.spawn(Node { width: Val::Px(b.x), height: Val::Px(b.y), ..default() }).with_children(|p| match page {
                MenuPage::Main => {
                    panel(p, t, MenuSlot::Panel, b);
                    title(p, t, MenuSlot::Title, b, &tr("Game menu"));
                    button(p, t, MenuSlot::Resume, b, &caption(parts.strings, "return", "Return to game"), MenuAct::Resume);
                    button(p, t, MenuSlot::Options, b, &caption(parts.strings, "options", "Options"), MenuAct::Options);
                    button(p, t, MenuSlot::Exit, b, &tr("Exit to main menu"), MenuAct::AskExit);
                    button(p, t, MenuSlot::Quit, b, &tr("Quit to desktop"), MenuAct::AskQuit);
                }
                MenuPage::Options => {
                    panel(p, t, MenuSlot::OptionsPanel, b);
                    title(p, t, MenuSlot::OptionsTitle, b, &caption(parts.strings, "options", "Options"));
                    stock::options_body(p, t, parts.settings, parts.tab, rect(MenuSlot::OptionsBody, b), MenuPlaced(MenuSlot::OptionsBody));
                    button(p, t, MenuSlot::OptionsBack, b, &tr("Back"), MenuAct::Back);
                }
                MenuPage::ConfirmExit | MenuPage::ConfirmQuit => {
                    let quit = page == MenuPage::ConfirmQuit;
                    let (head, one, two) = match (quit, parts.mission) {
                        (true, _) => ("Quit to desktop", "Quit the game?", "The match in progress is lost."),
                        (false, true) => ("Abort mission", "Abort the mission?", "It counts as lost; progress stays."),
                        (false, false) => ("Exit to main menu", "Leave this match?", "The match in progress is lost."),
                    };
                    panel(p, t, MenuSlot::ConfirmPanel, b);
                    title(p, t, MenuSlot::ConfirmTitle, b, &tr(head));
                    line(p, t, MenuSlot::ConfirmLine1, b, &tr(one));
                    line(p, t, MenuSlot::ConfirmLine2, b, &tr(two));
                    button(p, t, MenuSlot::Yes, b, &tr("Yes"), MenuAct::Confirm);
                    button(p, t, MenuSlot::No, b, &tr("No"), MenuAct::Back);
                }
            });
        });
}

/// Draw the menu again when it, the theme or the box changed; take it down when closed.
#[allow(clippy::too_many_arguments)]
fn rebuild(
    mut commands: Commands,
    roots: Query<Entity, With<MenuRoot>>,
    menu: Res<InGameMenu>,
    theme: Res<Theme>,
    stock_box: Res<StockBox>,
    settings: Res<Settings>,
    tab: Res<OptionsTab>,
    strings: Option<Res<Strings>>,
    mission: Option<Res<crate::game::mission_host::MissionRun>>,
) {
    if !(menu.is_changed() || theme.is_changed() || stock_box.is_changed() || strings.as_ref().is_some_and(|s| s.is_changed())) {
        return;
    }
    for e in &roots {
        commands.entity(e).despawn();
    }
    if !menu.open {
        return;
    }
    let parts = Parts { theme: &theme, settings: &settings, strings: strings.as_deref(), tab: tab.0, mission: mission.is_some() };
    spawn_menu(&mut commands, menu.page, &parts, stock_box.0);
}

/// Esc (the bound key) and [`OpenInGameMenu`]: open, close, or go back one page.
#[allow(clippy::too_many_arguments)]
fn keys_and_requests(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    bindings: Option<Res<KeyBindings>>,
    mut requests: MessageReader<OpenInGameMenu>,
    mut menu: ResMut<InGameMenu>,
    link: Option<Res<SimLink>>,
    mission: Option<Res<crate::game::mission_host::MissionRun>>,
    hud: Option<ResMut<crate::ui::hud::HudState>>,
) {
    let asked = requests.read().count() > 0;
    let pressed = keys.as_deref().is_some_and(|k| match bindings.as_deref() {
        Some(b) => b.just_pressed(Action::GameMenu, k),
        None => k.just_pressed(KeyCode::Escape),
    });
    if menu.open {
        if pressed {
            // One step back: a question or Options to the menu, the menu to the match.
            if menu.page == MenuPage::Main {
                menu.open = false;
            } else {
                menu.page = MenuPage::Main;
            }
        }
        return;
    }
    // A decided match ends on Esc or Enter (`flow::leave_keys`), not in a menu; a mission ends by its script.
    let decided = mission.is_none() && link.is_some_and(|l| crate::game::verdict(l.curr(), l.player()).is_some());
    if asked || (pressed && !decided) {
        menu.open = true;
        menu.page = MenuPage::Main;
        // A placement or a target waiting for a click must not take the click on a menu button.
        if let Some(mut h) = hud {
            h.placing = None;
            h.targeting = None;
        }
    }
}

/// The menu's buttons.
fn buttons(
    mut clicks: MessageReader<ButtonClicked>,
    acts: Query<&MenuAct>,
    mut menu: ResMut<InGameMenu>,
    mut leave: MessageWriter<LeaveMatch>,
    mut exit: MessageWriter<AppExit>,
) {
    for ButtonClicked(e) in clicks.read() {
        let Ok(act) = acts.get(*e) else { continue };
        if !menu.open {
            continue;
        }
        match act {
            MenuAct::Resume => menu.open = false,
            MenuAct::Options => menu.page = MenuPage::Options,
            MenuAct::Back => menu.page = MenuPage::Main,
            MenuAct::AskExit => menu.page = MenuPage::ConfirmExit,
            MenuAct::AskQuit => menu.page = MenuPage::ConfirmQuit,
            MenuAct::Confirm => match menu.page {
                MenuPage::ConfirmQuit => {
                    exit.write(AppExit::Success);
                }
                MenuPage::ConfirmExit => leave_current_match(&mut leave),
                _ => {}
            },
        }
    }
}

/// Single player stops the sim while the menu is open; a networked match never does.
fn pause(menu: Res<InGameMenu>, net: Option<Res<NetworkedMatch>>, clock: Option<ResMut<AnimClock>>) {
    let Some(mut clock) = clock else { return };
    let want = menu.open && net.is_none();
    if clock.paused() != want {
        clock.set_paused(want);
    }
}

/// Back out of the match: the menu is closed and the clock runs again, for the next match.
fn reset(mut menu: ResMut<InGameMenu>, clock: Option<ResMut<AnimClock>>, mut tab: ResMut<OptionsTab>) {
    *menu = InGameMenu::default();
    tab.0 = 0;
    if let Some(mut c) = clock {
        c.set_paused(false);
    }
}

/// The options panel's volumes reach the mixer: `Settings` (0..=100) to `Volumes` (0..=1).
/// The front end's sliders wrote to `Settings` and saved it, but nothing read it.
fn apply_volumes(settings: Res<Settings>, volumes: Option<ResMut<crate::audio::Volumes>>) {
    let Some(mut v) = volumes else { return };
    if !settings.is_changed() {
        return;
    }
    let level = |n: f32| (n / 100.0).clamp(0.0, 1.0);
    let want = crate::audio::Volumes { master: level(settings.master), music: level(settings.music), sfx: level(settings.effects), voice: level(settings.voice), ..*v };
    if *v != want {
        *v = want;
    }
}

/// Registers the menu. Add it with the HUD.
pub struct InGameMenuPlugin;

impl Plugin for InGameMenuPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<ThemePlugin>() {
            app.add_plugins(ThemePlugin);
        }
        if !app.world().contains_resource::<State<AppState>>() {
            app.init_state::<AppState>();
        }
        let ingame = in_state(AppState::InGame);
        app.init_resource::<InGameMenu>()
            .init_resource::<Settings>()
            .init_resource::<crate::ui::profile::UiPreference>()
            .init_resource::<OptionsTab>()
            .init_resource::<StockBox>()
            .init_resource::<ButtonInput<KeyCode>>()
            .add_message::<OpenInGameMenu>()
            .add_message::<LeaveMatch>()
            .add_systems(OnExit(AppState::InGame), reset)
            // The menu's own logic runs before the sim link: a menu opened this frame holds the next tick.
            .add_systems(Update, (keys_and_requests, buttons, pause).chain().run_if(ingame.clone()).in_set(GameSet::Input))
            .add_systems(
                Update,
                (
                    // The options panel's widgets, as the front end's screens run them (`on_settings` runs in a match too).
                    (stock::track_box, stock::track_tab, rebuild).chain().run_if(ingame),
                )
                    .in_set(GameSet::Ui),
            )
            .add_systems(Update, apply_volumes.in_set(GameSet::Ui));
    }
}

#[cfg(test)]
mod tests;
