//! Scrolling lists and the shared scroll behaviour (wheel, keep-row-visible), plus
//! the keyboard step of sliders.

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::picking::Pickable;
use bevy::picking::hover::HoverMap;
use bevy::prelude::*;

use super::focus::{KitKey, hovered};
use super::{Focusable, Theme, WidgetState, kit_text};
use crate::ui::theme::{List, ListItem, ListSelected, RowMark, Slider, SliderChanged};

/// A node of `count` rows `row_h` tall showing `rows` of them; scrolls by the wheel.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct ScrollArea {
    pub row_h: f32,
    pub rows: usize,
    pub count: usize,
}

impl ScrollArea {
    pub fn max_offset(&self) -> f32 {
        (self.count.saturating_sub(self.rows)) as f32 * self.row_h
    }
}

/// Scroll just enough to show `row`.
pub fn scroll_to_row(area: &ScrollArea, pos: &mut ScrollPosition, row: usize) {
    let top = row as f32 * area.row_h;
    let bottom = top + area.row_h;
    let view = area.rows as f32 * area.row_h;
    let mut y = pos.0.y;
    if top < y {
        y = top;
    } else if bottom > y + view {
        y = bottom - view;
    }
    y = y.clamp(0.0, area.max_offset());
    if pos.0.y != y {
        pos.0.y = y;
    }
}

/// A selectable list `rows` tall (it scrolls past that), rows as the plain kit list
/// draws them. Chosen rows report through `ListSelected`.
pub fn scroll_list(theme: &Theme, items: &[&str], selected: Option<usize>, rows: usize) -> impl Bundle {
    let h = theme.list_row_h;
    let shown = rows.min(items.len().max(1));
    let rows_bundle: Vec<_> = items
        .iter()
        .enumerate()
        .map(|(i, s)| {
            (
                ListItem(i),
                WidgetState::Normal,
                Node {
                    height: Val::Px(h),
                    flex_shrink: 0.0,
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
                    kit_text(theme, (*s).to_owned(), theme.text_size, theme.text),
                ],
            )
        })
        .collect();
    (
        List { selected },
        ScrollArea { row_h: h, rows: shown, count: items.len() },
        ScrollPosition::default(),
        Focusable,
        Node {
            flex_direction: FlexDirection::Column,
            flex_shrink: 0.0,
            height: Val::Px(h * shown as f32),
            overflow: Overflow::scroll_y(),
            ..default()
        },
        BackgroundColor(theme.track),
        Children::spawn(bevy::ecs::spawn::SpawnIter(rows_bundle.into_iter())),
    )
}

/// Wheel over a scroll area (a list, an open dropdown) scrolls it.
pub(super) fn wheel(
    mut wheel: MessageReader<MouseWheel>,
    hover: Res<HoverMap>,
    parents: Query<&ChildOf>,
    mut areas: Query<(&ScrollArea, &mut ScrollPosition)>,
) {
    let mut dy = 0.0;
    for ev in wheel.read() {
        dy += match ev.unit {
            MouseScrollUnit::Line => ev.y * 3.0,
            MouseScrollUnit::Pixel => ev.y / 24.0,
        };
    }
    if dy == 0.0 {
        return;
    }
    for hit in hovered(&hover) {
        let mut e = hit;
        loop {
            if let Ok((area, mut pos)) = areas.get_mut(e) {
                pos.0.y = (pos.0.y - dy * area.row_h).clamp(0.0, area.max_offset());
                return;
            }
            match parents.get(e) {
                Ok(p) => e = p.parent(),
                Err(_) => break,
            }
        }
    }
}

/// Up/Down/Home/End/PageUp/PageDown move a focused list's selection.
pub fn key(w: &mut World, e: Entity, k: &KitKey) {
    let (Some(area), Some(list)) = (w.get::<ScrollArea>(e).copied(), w.get::<List>(e).copied()) else { return };
    if area.count == 0 {
        return;
    }
    let cur = list.selected;
    let last = area.count - 1;
    let to = match k.code {
        KeyCode::ArrowDown => Some(cur.map_or(0, |c| (c + 1).min(last))),
        KeyCode::ArrowUp => Some(cur.map_or(0, |c| c.saturating_sub(1))),
        KeyCode::Home => Some(0),
        KeyCode::End => Some(last),
        KeyCode::PageDown => Some(cur.map_or(0, |c| (c + area.rows).min(last))),
        KeyCode::PageUp => Some(cur.map_or(0, |c| c.saturating_sub(area.rows))),
        _ => None,
    };
    let Some(to) = to else { return };
    if let Some(mut pos) = w.get_mut::<ScrollPosition>(e) {
        scroll_to_row(&area, &mut pos, to);
    }
    if cur != Some(to) {
        w.get_mut::<List>(e).unwrap().selected = Some(to);
        w.write_message(ListSelected { list: e, index: to });
    }
}

/// How far Left and Right move a focused slider.
#[derive(Component, Debug, Clone, Copy)]
pub struct SliderStep(pub f32);

/// A slider the keyboard reaches: Left/Right by `step`, Home/End to the ends.
pub fn slider_focusable(theme: &Theme, w: f32, min: f32, max: f32, value: f32, step: f32) -> impl Bundle {
    (crate::ui::theme::slider(theme, w, min, max, value), Focusable, SliderStep(step))
}

pub fn slider_key(w: &mut World, e: Entity, k: &KitKey) {
    let Some(s) = w.get::<Slider>(e).copied() else { return };
    let step = w.get::<SliderStep>(e).map_or((s.max - s.min) / 20.0, |s| s.0);
    let to = match k.code {
        KeyCode::ArrowRight | KeyCode::ArrowUp => s.value + step,
        KeyCode::ArrowLeft | KeyCode::ArrowDown => s.value - step,
        KeyCode::Home => s.min,
        KeyCode::End => s.max,
        _ => return,
    }
    .clamp(s.min, s.max);
    if (to - s.value).abs() > f32::EPSILON {
        w.get_mut::<Slider>(e).unwrap().value = to;
        w.write_message(SliderChanged { entity: e, value: to });
    }
}
