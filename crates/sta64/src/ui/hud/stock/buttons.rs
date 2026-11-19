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
//! - **comm**, **help** (cycles three tooltip states), **palette** (the popup
//!   palette) and **toggle** (swaps cameras) drive systems STA64 does not have yet: drawn, disabled.
//! - **more** shows and hides the second row (menu, help, palette). Grades: the install's tooltip text
//!   for it says it shows more buttons and names help, palette and the options menu (read from data); a
//!   static reading of the executable agrees. An older reading took it as moving the panel between two
//!   rectangles; the two readings are not reconciled. INTERIM: the start state is not measured, so all
//!   buttons show at match start (and again at each new match); what the original draws around the second
//!   row is not established: buttons only, no invented art. Esc opens the in-game menu whatever is shown.
//!
//! The panel is the cfg's `buttonPanelArea` with its `buttonBackgroundPanel` end pieces (read from data),
//! anchored as one group at the window's right edge on a window wider than the sheet
//! ([`StockLayout::panel_at_right_edge`]); the buttons' areas are relative to it.

use bevy::prelude::*;
use sta_sim::view::UnitFlags;

use super::layout::StockLayout;
use crate::ui::hud::layout::R;
use bevy::picking::Pickable;
use super::node;
use crate::input::select::CameraFocus;
use crate::ui::gui_sprites::{GuiButton, GuiButtonColors};
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
}

const fn keys(area: &'static str, sprite: &'static str, default_sprite: &'static str, act: PanelButton, tip: &'static str) -> PanelKeys {
    PanelKeys { area, sprite, default_sprite, act, tip }
}

/// The buttons that show only while "more" is on.
pub(in crate::ui::hud) const SECOND_ROW: [&str; 3] = ["menuButtonArea", "helpButtonArea", "paletteButtonArea"];

/// Whether the second row shows (the player's, kept while the HUD is rebuilt; true at each match start: interim).
#[derive(Resource)]
pub(in crate::ui) struct ShowMore(pub bool);

impl Default for ShowMore {
    fn default() -> Self {
        Self(true)
    }
}

/// A new match starts with every button shown.
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
    keys("menuButtonArea", "menuButton", "menu_button", PanelButton::Menu, "Menu"),
    keys("commButtonArea", "commButton", "comm_button", PanelButton::Unavailable, "Communications"),
    keys("idleButtonArea", "idleButton", "idle_button", PanelButton::Idle, "Next idle ship"),
    keys("toggleButtonArea", "toggleButton", "toggle_button", PanelButton::Unavailable, "Toggle view"),
    keys("objectivesButtonArea", "objectivesButton", "objectives_button", PanelButton::Objectives, "Objectives"),
    keys("helpButtonArea", "helpButton", "help_button", PanelButton::Unavailable, "Help"),
    keys("paletteButtonArea", "paletteButton", "palette_button", PanelButton::Unavailable, "Command palette"),
    keys("moreButtonArea", "moreButton", "more_button", PanelButton::More, "More"),
];

/// The sprite of a button: the cfg's, else the engine's default.
pub(in crate::ui::hud) fn sprite(layout: &StockLayout, k: &PanelKeys) -> String {
    layout.name(k.sprite).unwrap_or(k.default_sprite).to_owned()
}

/// The panel's rectangle: the cfg's `buttonPanelArea` placed at the right edge, or, when the cfg has
/// none, the whole sheet (the buttons' areas then count from its corner).
pub(in crate::ui::hud) fn panel_rect(layout: &StockLayout) -> Option<R> {
    layout.panel_at_right_edge("button")
}

/// Spawn the panel group: its background pieces and every button whose area the cfg authors (a zero
/// rectangle hides it).
pub(in crate::ui::hud) fn spawn(commands: &mut Commands, parent: Entity, layout: &StockLayout, theme: &Theme, more: bool) {
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
        let mut button = commands.spawn((
            k.act,
            UiButton,
            state,
            GuiButtonColors::default(),
            Tooltip::new(k.tip.to_owned()),
            GuiButton { names: vec![sprite(layout, k)], keep_label: false },
            node,
            BackgroundColor(theme.button),
            ChildOf(group),
        ));
        if second {
            button.insert(SecondRow);
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
#[allow(clippy::too_many_arguments)]
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
) {
    let mut objectives = objectives;
    for ButtonClicked(entity) in clicks.read() {
        match buttons.get(*entity) {
            Ok(PanelButton::Menu) => {
                menu.write(OpenInGameMenu);
            }
            Ok(PanelButton::More) => more.0 = !more.0,
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
