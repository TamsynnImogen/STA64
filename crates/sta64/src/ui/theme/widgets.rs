//! Widget builders. Each returns a bundle to `spawn` or nest in `children!`.
//! Behaviour lives in `systems.rs`, keyed on the components here.

use bevy::ecs::spawn::SpawnIter;
use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;

use super::tokens::Theme;

/// Pointer state of an interactive widget. Set by the pointer observers;
/// tests set it directly. `Disabled` is never overwritten by the pointer.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WidgetState {
    #[default]
    Normal,
    Hovered,
    Pressed,
    Disabled,
}

/// The label of a button or toggle: black on the pill, `text_off` when disabled.
#[derive(Component, Debug, Clone, Copy)]
pub struct ButtonText;

/// The gold half-pill at the left of a selected list row.
#[derive(Component, Debug, Clone, Copy)]
pub struct RowMark;

#[derive(Component, Debug, Clone, Copy)]
pub struct UiButton;

#[derive(Component, Debug, Clone, Copy)]
pub struct Toggle {
    pub on: bool,
}

/// A horizontal slider; `value` in `min..=max`.
#[derive(Component, Debug, Clone, Copy)]
pub struct Slider {
    pub value: f32,
    pub min: f32,
    pub max: f32,
}

/// Fill of a [`Slider`] or [`ProgressBar`] track (child entity).
#[derive(Component, Debug, Clone, Copy)]
pub struct Fill;

/// Progress `0..=1`.
#[derive(Component, Debug, Clone, Copy)]
pub struct ProgressBar(pub f32);

#[derive(Component, Debug, Clone, Copy)]
pub struct UiLabel;

/// Frame with an LCARS elbow, a title bar and a content area.
#[derive(Component, Debug, Clone, Copy)]
pub struct Panel;

/// A selectable list; `selected` is an index into its [`ListItem`] children.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct List {
    pub selected: Option<usize>,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct ListItem(pub usize);

/// Kit text (UI-16): the mod's sprite font of the role this size stands for, the baked
/// TTF where the mod has none. The colour lives in [`SpriteText`].
pub(super) fn text(theme: &Theme, s: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    super::stock::kit_text(theme, s, size, color)
}

pub fn label(theme: &Theme, s: &str) -> impl Bundle {
    (UiLabel, Node::default(), children![text(theme, s, theme.text_size, theme.text)])
}

/// A pill, `button_h` tall; its label sits at the right as on a console.
fn pill(theme: &Theme, w: f32) -> Node {
    Node {
        width: Val::Px(w),
        height: Val::Px(theme.button_h),
        padding: UiRect::horizontal(Val::Px(theme.button_h * 0.5)),
        align_items: AlignItems::Center,
        justify_content: JustifyContent::FlexEnd,
        border_radius: BorderRadius::MAX,
        ..default()
    }
}

fn pill_label(theme: &Theme, s: &str) -> impl Bundle {
    (ButtonText, text(theme, theme.caps(s, theme.caps_buttons), theme.text_size, theme.text_on))
}

/// A flat lavender pill, black capitals at the right. Hover, pressed and
/// disabled change its colour (`state_colour`).
pub fn button(theme: &Theme, s: &str, w: f32) -> impl Bundle {
    (UiButton, WidgetState::Normal, pill(theme, w), BackgroundColor(theme.button), children![pill_label(theme, s)])
}

pub fn disabled_button(theme: &Theme, s: &str, w: f32) -> impl Bundle {
    (UiButton, WidgetState::Disabled, pill(theme, w), BackgroundColor(theme.button_off), children![pill_label(theme, s)])
}

/// A pill that is lavender when off and gold when on.
pub fn toggle(theme: &Theme, s: &str, on: bool, w: f32) -> impl Bundle {
    let c = if on { theme.button_on } else { theme.button };
    (Toggle { on }, WidgetState::Normal, pill(theme, w), BackgroundColor(c), children![pill_label(theme, s)])
}

fn bar_node(theme: &Theme, w: f32) -> Node {
    Node {
        width: Val::Px(w),
        height: Val::Px(theme.track_h),
        border_radius: BorderRadius::MAX,
        overflow: Overflow::clip(),
        ..default()
    }
}

fn fill(theme: &Theme, frac: f32) -> impl Bundle {
    (
        Fill,
        Node { width: Val::Percent(frac.clamp(0.0, 1.0) * 100.0), height: Val::Percent(100.0), ..default() },
        BackgroundColor(theme.fill),
        Pickable::IGNORE,
    )
}

/// A slider: a 6-unit `track` with a gold fill; press and drag across it to set the value.
pub fn slider(theme: &Theme, w: f32, min: f32, max: f32, value: f32) -> impl Bundle {
    (
        Slider { value, min, max },
        WidgetState::Normal,
        RelativeCursorPosition::default(),
        bar_node(theme, w),
        BackgroundColor(theme.track),
        children![fill(theme, (value - min) / (max - min).max(f32::EPSILON))],
    )
}

pub fn progress_bar(theme: &Theme, w: f32, value: f32) -> impl Bundle {
    (ProgressBar(value.clamp(0.0, 1.0)), bar_node(theme, w), BackgroundColor(theme.track), children![fill(theme, value)])
}

/// A selectable list of rows: hover `row_hot`, selected `row_sel` with a gold
/// half-pill at the left, odd rows faintly lit.
pub fn list(theme: &Theme, items: &[&str], selected: Option<usize>) -> impl Bundle {
    let rows: Vec<_> = items
        .iter()
        .enumerate()
        .map(|(i, s)| {
            (
                ListItem(i),
                WidgetState::Normal,
                Node {
                    height: Val::Px(theme.list_row_h),
                    padding: UiRect::axes(Val::Px(theme.pad), Val::Px(2.0)),
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(if selected == Some(i) { theme.row_sel } else { Color::NONE }),
                children![
                    (
                        RowMark,
                        Pickable::IGNORE,
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            width: Val::Px(theme.pad * 0.5),
                            height: Val::Percent(70.0),
                            border_radius: BorderRadius::px(0.0, theme.pad, theme.pad, 0.0),
                            ..default()
                        },
                        BackgroundColor(if selected == Some(i) { theme.accent } else { Color::NONE }),
                    ),
                    text(theme, *s, theme.text_size, theme.text),
                ],
            )
        })
        .collect();
    (List { selected }, Node { flex_direction: FlexDirection::Column, flex_shrink: 0.0, ..default() }, Children::spawn(SpawnIter(rows.into_iter())))
}

/// A black rectangle with one rounded corner, laid over an elbow to cut its inside.
fn cut(left: f32, top: f32, bottom_up: bool, r: f32) -> impl Bundle {
    let radius = if bottom_up { BorderRadius::px(0.0, 0.0, 0.0, r) } else { BorderRadius::px(r, 0.0, 0.0, 0.0) };
    (
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(left),
            right: Val::Px(0.0),
            top: if bottom_up { Val::Px(0.0) } else { Val::Px(top) },
            bottom: if bottom_up { Val::Px(top) } else { Val::Px(0.0) },
            border_radius: radius,
            ..default()
        },
        BackgroundColor(Color::BLACK),
        Pickable::IGNORE,
    )
}

/// An LCARS elbow: `side` wide vertical arm, `bar` tall horizontal arm, outer
/// radius `outer` at the corner, an `inner`-radius cut inside it.
fn elbow(theme: &Theme, color: Color, bottom: bool, bar: f32) -> impl Bundle {
    let (side, outer, inner) = (theme.side_bar, theme.corners, theme.elbow_inner);
    let radius = if bottom { BorderRadius::px(0.0, 0.0, 0.0, outer) } else { BorderRadius::px(outer, 0.0, 0.0, 0.0) };
    (
        Node { width: Val::Px(side + theme.elbow_arm), height: Val::Px(outer + bar), flex_shrink: 0.0, border_radius: radius, ..default() },
        BackgroundColor(color),
        children![cut(side, bar, bottom, inner)],
    )
}

fn bar(color: Color, h: f32, right_cap: bool) -> impl Bundle {
    (
        Node {
            flex_grow: 1.0,
            height: Val::Px(h),
            border_radius: if right_cap { BorderRadius::px(0.0, h / 2.0, h / 2.0, 0.0) } else { BorderRadius::ZERO },
            ..default()
        },
        BackgroundColor(color),
    )
}

/// A frame in Refit's LCARS form: black body, a lavender elbow top left and a
/// pale one bottom left joined by the side bar, the title in gold capitals in
/// the top bar's gap with a sky cap after it, and a gold end bar along the foot.
pub fn panel(theme: &Theme, title: &str, content: impl Bundle) -> impl Bundle {
    let g = theme.bar_gap;
    let top = theme.title_size + 6.0;
    let side = theme.side_bar;
    (
        Panel,
        Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(g), ..default() },
        BackgroundColor(theme.panel),
        children![
            (
                Node { column_gap: Val::Px(g), align_items: AlignItems::FlexStart, ..default() },
                children![
                    elbow(theme, theme.accent2, false, top),
                    (
                        Node { flex_grow: 1.0, column_gap: Val::Px(g), height: Val::Px(top), align_items: AlignItems::Center, ..default() },
                        children![
                            bar(theme.accent2, top, false),
                            text(theme, theme.caps(title, theme.caps_titles), theme.title_size, theme.title),
                            (
                                Node {
                                    width: Val::Px(theme.cap_w),
                                    height: Val::Px(top),
                                    border_radius: BorderRadius::px(0.0, top / 2.0, top / 2.0, 0.0),
                                    ..default()
                                },
                                BackgroundColor(theme.accent3),
                            ),
                        ],
                    ),
                ],
            ),
            (
                Node { column_gap: Val::Px(g), ..default() },
                children![
                    (
                        Node { width: Val::Px(side), flex_direction: FlexDirection::Column, row_gap: Val::Px(g), ..default() },
                        children![
                            (Node { flex_grow: 58.0, ..default() }, BackgroundColor(theme.accent2)),
                            (Node { flex_grow: 42.0, ..default() }, BackgroundColor(theme.panel_hi)),
                        ],
                    ),
                    (
                        Node {
                            flex_grow: 1.0,
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(theme.gap),
                            padding: UiRect::all(Val::Px(theme.pad)),
                            ..default()
                        },
                        children![content],
                    ),
                ],
            ),
            (
                Node { column_gap: Val::Px(g), align_items: AlignItems::FlexEnd, ..default() },
                children![
                    elbow(theme, theme.panel_hi, true, theme.bottom_bar),
                    (
                        Node {
                            width: Val::Px(theme.elbow_arm + side),
                            height: Val::Px(theme.bottom_bar),
                            border_radius: BorderRadius::px(0.0, theme.bottom_bar / 2.0, theme.bottom_bar / 2.0, 0.0),
                            ..default()
                        },
                        BackgroundColor(theme.accent),
                    ),
                ],
            ),
        ],
    )
}
