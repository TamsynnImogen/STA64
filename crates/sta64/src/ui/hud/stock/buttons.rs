//! The Armada II / Fleet Operations button panel (UI-48): the small buttons
//! the cfg places with `<name>ButtonArea` and draws with the `<name>Button`
//! sprite (the engine's `<name>_button` when the cfg names none). Rules read
//! from the Armada II executable, static; not compared on screen:
//!
//! - **menu** opens the in-game menu;
//! - **objectives** shows the objectives display;
//! - **idle** moves the camera to an idle ship. Which ships the original counts
//!   as idle is its overview's own list (not read); here it is an own craft that
//!   can move and stands still, not building or docked (ours), taken in turn;
//! - **toggle** swaps the strategic and tactical cameras;
//! - **comm**, **help** (cycles three tooltip states) and **palette** (the popup
//!   palette) drive systems STA64 does not have yet: drawn, disabled.
//! - **more** shows and hides the second row (menu, help, palette). Grades: the install's tooltip text
//!   for it says it shows more buttons and names help, palette and the options menu (read from data); a
//!   static reading of the executable agrees. An older reading took it as moving the panel between two
//!   rectangles; the two readings are not reconciled. The start state is collapsed, at each new match too
//!   (stated by a player). The split is STA64's rule, graded read from data: the second row is menu, help
//!   and palette ([`SECOND_ROW`]); idle, toggle, comm, objectives and more always show. INTERIM: what the original draws around the second row is not established: buttons only, no
//!   invented art. Esc opens the in-game menu whatever is shown.
//!
//! The panel is the cfg's `buttonPanelArea` with its `buttonBackgroundPanel` end pieces (read from data).
//! It is placed like every other stock panel: at its sheet position inside the clamped, centred HUD area
//! (ADR 0008 H1/H2, [`StockLayout::panel`]), as one group (group node, end pieces, buttons; the buttons'
//! areas are relative to it), and its pointer-solid rectangles come from the same placed rectangle
//! ([`solid`]). A group at the window's edge is a declared anchor (ADR 0008 H5), not built.

use bevy::prelude::*;
use sta_sim::view::UnitFlags;

use super::layout::StockLayout;
use crate::ui::hud::layout::R;
use crate::ui::profile::UiProfile;
use bevy::picking::Pickable;
use super::node;
use crate::input::select::CameraFocus;
use crate::ui::gui_sprites::{DisplayTint, GuiButton, GuiButtonColors};
use crate::ui::hud::HudFeed;
use crate::ui::ingame_menu::OpenInGameMenu;
use crate::ui::notices::Tooltip;
use crate::ui::presentation::ObjectivesView;
use crate::ui::theme::{ButtonClicked, Theme, UiButton, WidgetState};

/// What a panel button does.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::hud) enum PanelButton {
    Menu,
    Objectives,
    Idle,
    /// Swap the strategic and tactical cameras.
    ToggleView,
    /// The Armada I action camera's picture (UI-53): a click goes to what it shows.
    ActionView,
    /// Shows or hides the second row (menu, help, palette).
    More,
    /// A system STA64 does not have yet.
    Unavailable,
}

/// One panel button: its cfg area key, cfg sprite key, the engine's sprite when
/// the cfg names none, what it does, and its tooltip. In the engine's load order.
pub(in crate::ui::hud) struct PanelKeys {
    pub area: &'static str,
    pub sprite: &'static str,
    pub default_sprite: &'static str,
    pub act: PanelButton,
    pub tip: &'static str,
    pub verbose: Option<&'static str>,
}

const fn keys(area: &'static str, sprite: &'static str, default_sprite: &'static str, act: PanelButton, tip: &'static str, verbose: Option<&'static str>) -> PanelKeys {
    PanelKeys { area, sprite, default_sprite, act, tip, verbose }
}

/// The hover and pressed tints of the group's buttons in the Armada II look: a per-channel multiplier of the
/// button picture in display (sRGB-encoded) values, clamped at white. Grade: measured at one window size,
/// hover on three buttons, pressed on one button.
pub(in crate::ui::hud) const GROUP_TINT: DisplayTint = DisplayTint { hovered: [1.355, 0.979, 0.786], pressed: [1.589, 1.384, 1.261] };

/// The buttons that show only while "more" is on.
pub(in crate::ui::hud) const SECOND_ROW: [&str; 3] = ["menuButtonArea", "helpButtonArea", "paletteButtonArea"];

/// Whether the second row shows (the player's, kept while the HUD is rebuilt; false at each match start: stated by a player).
#[derive(Resource, Default)]
pub(in crate::ui) struct ShowMore(pub bool);

/// A new match starts with the second row hidden.
pub(in crate::ui::hud) fn reset_more(mut more: ResMut<ShowMore>) {
    more.0 = ShowMore::default().0;
}

/// The button panel's group: its rectangle (the buttons' areas are relative to it).
#[derive(Component)]
pub(in crate::ui::hud) struct ButtonPanelGroup;

/// A button of the second row.
#[derive(Component)]
pub(in crate::ui::hud) struct SecondRow;

pub(in crate::ui::hud) const BUTTONS: [PanelKeys; 8] = [
    keys("menuButtonArea", "menuButton", "menu_button", PanelButton::Menu, "Menu", None),
    keys("commButtonArea", "commButton", "comm_button", PanelButton::Unavailable, "Communications", None),
    keys("idleButtonArea", "idleButton", "idle_button", PanelButton::Idle, "Next idle ship", Some("Focus the next ship with no orders.")),
    keys("toggleButtonArea", "toggleButton", "toggle_button", PanelButton::ToggleView, "Toggle view", Some("Switch between strategic and tactical views.")),
    keys("objectivesButtonArea", "objectivesButton", "objectives_button", PanelButton::Objectives, "Objectives", Some("Show current mission objectives.")),
    keys("helpButtonArea", "helpButton", "help_button", PanelButton::Unavailable, "Help", Some("Turn tooltip help on or off.")),
    keys("paletteButtonArea", "paletteButton", "palette_button", PanelButton::Unavailable, "Command palette", Some("Switch the command palette between persistent and popup modes.")),
    keys("moreButtonArea", "moreButton", "more_button", PanelButton::More, "More", Some("Show additional buttons, including Help, Palette, and Menu.")),
];

/// The sprite of a button: the cfg's, else the engine's default.
pub(in crate::ui::hud) fn sprite(layout: &StockLayout, k: &PanelKeys) -> String {
    layout.name(k.sprite).unwrap_or(k.default_sprite).to_owned()
}

/// The panel's rectangle: the cfg's `buttonPanelArea` placed in the HUD area like every stock panel, or,
/// when the cfg has none, nothing (the buttons' areas then count from the window's corner).
pub(in crate::ui::hud) fn panel_rect(layout: &StockLayout) -> Option<R> {
    layout.panel("button")
}

/// The pointer-solid rectangles of the group. Closed: the panel's placed rectangle (the first row). Open, in
/// the Armada II look: ONE rectangle covering both rows, the panel's width from its top down to the lowest
/// edge of the second-row buttons' areas (rule measured at one window size, in Armada II only: a click or
/// hover anywhere in it, gaps and the empty part of the row included, never reaches the world or the
/// minimap behind it). Other looks (Fleet Operations) keep the earlier interim: the shown second-row
/// buttons' own areas, which is not compared with their original.
pub(in crate::ui::hud) fn solid(layout: &StockLayout, more: bool, profile: Option<UiProfile>) -> Vec<R> {
    let Some(panel) = panel_rect(layout) else { return Vec::new() };
    if !more {
        return vec![panel];
    }
    let origin = Vec2::new(panel.x, panel.y);
    let row: Vec<R> = SECOND_ROW.iter().filter_map(|key| layout.at(origin, key)).collect();
    if profile == Some(UiProfile::Armada2) {
        let bottom = row.iter().map(|r| r.bottom()).fold(panel.bottom(), f32::max);
        return vec![R::new(panel.x, panel.y, panel.w, bottom - panel.y)];
    }
    let mut out = vec![panel];
    out.extend(row);
    out
}

/// Spawn the panel group: its background pieces and every button whose area the cfg authors (a zero
/// rectangle hides it).
pub(in crate::ui::hud) fn spawn(commands: &mut Commands, parent: Entity, layout: &StockLayout, theme: &Theme, more: bool, display_tint: bool) {
    let area = panel_rect(layout).unwrap_or(R::new(0.0, 0.0, 0.0, 0.0));
    let group = commands
        .spawn((
            ButtonPanelGroup,
            Pickable::IGNORE,
            Node { position_type: PositionType::Absolute, left: Val::Px(area.x), top: Val::Px(area.y), width: Val::Px(area.w), height: Val::Px(area.h), ..default() },
            ChildOf(parent),
        ))
        .id();
    super::a1::frame_pieces(commands, group, layout, "button", Vec2::ZERO);
    for k in &BUTTONS {
        let Some(rect) = layout.rect(k.area) else { continue };
        let state = if k.act == PanelButton::Unavailable { WidgetState::Disabled } else { WidgetState::Normal };
        let second = SECOND_ROW.contains(&k.area);
        let mut node = node(rect);
        if second && !more {
            node.display = Display::None;
        }
        let mut tip = Tooltip::new(k.tip);
        if let Some(verbose) = k.verbose {
            tip = tip.verbose(verbose);
        }
        let mut button = commands.spawn((
            k.act,
            UiButton,
            state,
            GuiButtonColors::default(),
            tip,
            GuiButton { names: vec![sprite(layout, k)], keep_label: false },
            node,
            BackgroundColor(theme.button),
            ChildOf(group),
        ));
        if second {
            button.insert(SecondRow);
        }
        if display_tint {
            button.insert(GROUP_TINT);
        }
    }
}

/// Show or hide the second row as [`ShowMore`] says.
pub(in crate::ui::hud) fn apply_more(more: Res<ShowMore>, mut rows: Query<&mut Node, With<SecondRow>>) {
    if !more.is_changed() {
        return;
    }
    for mut node in &mut rows {
        node.display = if more.0 { Display::Flex } else { Display::None };
    }
}

/// An own craft that can move and stands still, not building, docked or dying (ours).
fn idle(feed: &HudFeed, u: &sta_sim::view::UnitView) -> bool {
    u.owner == feed.player
        && u.speed == sta_sim::Fixed::ZERO
        && !u.flags.contains(UnitFlags::BUILDING)
        && !u.flags.contains(UnitFlags::DOCKED)
        && !u.flags.contains(UnitFlags::DYING)
        && u.progress.is_none()
        && feed.content.as_ref().and_then(|db| db.unit(u.def)).is_some_and(|d| sta_sim::world::EntityKind::of_def(d) == sta_sim::world::EntityKind::Craft && d.movement.max_speed > sta_sim::Fixed::ZERO)
}

/// The idle ships in id order; the next one after `last`, wrapping.
pub(in crate::ui::hud) fn next_idle(feed: &HudFeed, last: Option<sta_sim::EntityId>) -> Option<&sta_sim::view::UnitView> {
    let idle: Vec<_> = feed.view.units.iter().filter(|u| idle(feed, u)).collect();
    idle.iter().find(|u| last.is_none_or(|l| u.id > l)).or_else(|| idle.first()).copied()
}

/// Presses of the panel buttons.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(in crate::ui::hud) fn press(
    mut clicks: MessageReader<ButtonClicked>,
    buttons: Query<&PanelButton>,
    feed: Res<HudFeed>,
    objectives: Option<ResMut<ObjectivesView>>,
    mut menu: MessageWriter<OpenInGameMenu>,
    mut focus: MessageWriter<CameraFocus>,
    (subject, entities): (Option<Res<crate::render::action_camera::ActionSubject>>, Option<Res<crate::sim_link::SimEntities>>),
    mut last: Local<Option<sta_sim::EntityId>>,
    mut more: ResMut<ShowMore>,
    tactical: Option<ResMut<crate::input::camera_tactical::TacticalView>>,
    (time, mut last_picture_click, a2_window): (Res<Time>, Local<Option<f64>>, Query<(), With<super::window::CinematicSwitch>>),
) {
    let mut objectives = objectives;
    let mut tactical = tactical;
    for ButtonClicked(entity) in clicks.read() {
        match buttons.get(*entity) {
            Ok(PanelButton::Menu) => {
                menu.write(OpenInGameMenu);
            }
            Ok(PanelButton::More) => more.0 = !more.0,
            Ok(PanelButton::ToggleView) => {
                if let Some(view) = tactical.as_mut() {
                    view.request_toggle();
                }
            }
            Ok(PanelButton::Objectives) => {
                if let Some(o) = objectives.as_mut() {
                    o.visible = true;
                }
            }
            Ok(PanelButton::Idle) => {
                if let Some(u) = next_idle(&feed, *last) {
                    *last = Some(u.id);
                    let p = u.pos.to_f32();
                    // Sim to map space: z is mirrored.
                    focus.write(CameraFocus(Vec3::new(p[0], p[1], -p[2])));
                }
            }
            Ok(PanelButton::ActionView) => {
                // A double click swaps the views (strategic and tactical) and then points at the shown unit, as
                // a single click does (read from the executable, static; the original's double-click interval
                // is not established: INTERIM, the selection's 0.4 s). Armada II's window only: Armada I's is
                // its own rule set (not read).
                let now = time.elapsed_secs_f64();
                let double = a2_window.contains(*entity) && last_picture_click.is_some_and(|t| now - t <= crate::input::select::DOUBLE);
                *last_picture_click = if double { None } else { Some(now) };
                if double && let Some(view) = tactical.as_mut() {
                    view.request_toggle();
                }
                // Only a unit the renderer still has (in sight): the frame it leaves sight, the click does nothing.
                let drawn = |id| entities.as_ref().is_some_and(|e| e.0.contains_key(&id));
                if let Some(u) = subject.as_ref().and_then(|s| s.0).filter(|&id| drawn(id)).and_then(|id| feed.view.unit(id)) {
                    let p = u.pos.to_f32();
                    focus.write(CameraFocus(Vec3::new(p[0], p[1], -p[2])));
                }
            }
            _ => {}
        }
    }
}
