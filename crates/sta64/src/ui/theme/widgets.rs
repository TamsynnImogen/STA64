//! Widget builders. Each returns a bundle to `spawn` or nest in `children!`.
//! Behaviour lives in `systems.rs`, keyed on the components here.

use bevy::ecs::spawn::SpawnIter;
use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;

use super::tokens::{Theme, scale};

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

/// The colour a widget has in its normal state.
#[derive(Component, Debug, Clone, Copy)]
pub struct Look(pub Color);

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

fn text(s: &str, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(s),
        TextFont { font_size: bevy::text::FontSize::Px(size), ..default() },
        TextColor(color),
        Pickable::IGNORE,
    )
}

pub fn label(theme: &Theme, s: &str) -> impl Bundle {
    (UiLabel, Node::default(), children![text(s, theme.font_size, theme.text)])
}

fn pill(theme: &Theme, w: f32) -> Node {
    Node {
        width: Val::Px(w),
        height: Val::Px(theme.bar_height),
        padding: UiRect::horizontal(Val::Px(theme.pad)),
        align_items: AlignItems::Center,
        justify_content: JustifyContent::FlexEnd,
        border_radius: BorderRadius::MAX,
        ..default()
    }
}

/// A pill button in `color`, text right-aligned like LCARS.
pub fn button(theme: &Theme, s: &str, color: Color, w: f32) -> impl Bundle {
    (
        UiButton,
        WidgetState::Normal,
        Look(color),
        pill(theme, w),
        BackgroundColor(color),
        children![text(s, theme.font_size, theme.text_dark)],
    )
}

pub fn disabled_button(theme: &Theme, s: &str, w: f32) -> impl Bundle {
    (
        UiButton,
        WidgetState::Disabled,
        Look(theme.disabled),
        pill(theme, w),
        BackgroundColor(theme.disabled),
        children![text(s, theme.font_size, theme.text_dark)],
    )
}

/// A pill that switches between `color` and a dimmed `color` on click.
pub fn toggle(theme: &Theme, s: &str, color: Color, on: bool, w: f32) -> impl Bundle {
    (
        Toggle { on },
        WidgetState::Normal,
        Look(color),
        pill(theme, w),
        BackgroundColor(color),
        children![text(s, theme.font_size, theme.text_dark)],
    )
}

fn track(theme: &Theme, color: Color) -> Color {
    scale(color, theme.off_dim * 0.6)
}

fn bar_node(theme: &Theme, w: f32) -> Node {
    Node {
        width: Val::Px(w),
        height: Val::Px(theme.bar_height * 0.6),
        border_radius: BorderRadius::MAX,
        overflow: Overflow::clip(),
        ..default()
    }
}

fn fill(color: Color, frac: f32) -> impl Bundle {
    (
        Fill,
        Node { width: Val::Percent(frac.clamp(0.0, 1.0) * 100.0), height: Val::Percent(100.0), ..default() },
        BackgroundColor(color),
        Pickable::IGNORE,
    )
}

/// A slider track with a fill; press and drag across it to set the value.
pub fn slider(theme: &Theme, color: Color, w: f32, min: f32, max: f32, value: f32) -> impl Bundle {
    (
        Slider { value, min, max },
        WidgetState::Normal,
        Look(track(theme, color)),
        RelativeCursorPosition::default(),
        bar_node(theme, w),
        BackgroundColor(track(theme, color)),
        children![fill(color, (value - min) / (max - min).max(f32::EPSILON))],
    )
}

pub fn progress_bar(theme: &Theme, color: Color, w: f32, value: f32) -> impl Bundle {
    (ProgressBar(value.clamp(0.0, 1.0)), bar_node(theme, w), BackgroundColor(track(theme, color)), children![fill(color, value)])
}

/// A selectable list of rows.
pub fn list(theme: &Theme, color: Color, items: &[&str], selected: Option<usize>) -> impl Bundle {
    let (size, text_color, bg) = (theme.font_size, theme.text, theme.background);
    let (pad, _) = (theme.pad, color);
    let rows: Vec<_> = items
        .iter()
        .enumerate()
        .map(|(i, s)| {
            (
                ListItem(i),
                WidgetState::Normal,
                Look(bg),
                Node {
                    padding: UiRect::axes(Val::Px(pad), Val::Px(2.0)),
                    border_radius: BorderRadius::all(Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(bg),
                children![text(s, size, text_color)],
            )
        })
        .collect();
    (
        List { selected },
        Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(2.0), ..default() },
        Children::spawn(SpawnIter(rows.into_iter())),
    )
}

/// A frame: a rail with elbows on the left, a title bar on top, `content`
/// below the bar.
pub fn panel(theme: &Theme, title: &str, color: Color, content: impl Bundle) -> impl Bundle {
    let r = theme.elbow_radius;
    let bar = theme.bar_height;
    (
        Panel,
        Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(theme.gap), ..default() },
        children![
            (
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(theme.gap),
                    width: Val::Px(theme.rail_width),
                    ..default()
                },
                children![
                    (
                        Node { height: Val::Px(bar * 2.0), border_radius: BorderRadius::px(r, 0.0, 0.0, 0.0), ..default() },
                        BackgroundColor(color),
                    ),
                    (Node { flex_grow: 1.0, min_height: Val::Px(40.0), ..default() }, BackgroundColor(color)),
                    (
                        Node { height: Val::Px(bar), border_radius: BorderRadius::px(0.0, 0.0, 0.0, r), ..default() },
                        BackgroundColor(color),
                    ),
                ],
            ),
            (
                Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(theme.gap), flex_grow: 1.0, ..default() },
                children![
                    (
                        Node {
                            height: Val::Px(bar),
                            padding: UiRect::horizontal(Val::Px(theme.pad)),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::FlexEnd,
                            border_radius: BorderRadius::px(0.0, bar / 2.0, bar / 2.0, 0.0),
                            ..default()
                        },
                        BackgroundColor(color),
                        children![text(title, theme.title_size, theme.text_dark)],
                    ),
                    (
                        Node {
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(theme.gap),
                            padding: UiRect::all(Val::Px(theme.pad)),
                            ..default()
                        },
                        children![content],
                    ),
                ],
            ),
        ],
    )
}
