//! Check box: a square frame with a mark, and its caption.

use bevy::picking::Pickable;
use bevy::prelude::*;

use super::{Focusable, Theme, WidgetState, kit_text};
use crate::ui::sprite_font::SpriteText;


#[derive(Component, Debug, Clone, Copy)]
pub struct Checkbox {
    pub on: bool,
}

/// A check box changed (click, or Enter/Space when focused).
#[derive(Message, Debug, Clone, Copy)]
pub struct CheckboxChanged {
    pub entity: Entity,
    pub on: bool,
}

#[derive(Component, Debug, Clone, Copy)]
pub(super) struct CheckFrame;
#[derive(Component, Debug, Clone, Copy)]
pub(super) struct CheckMark;
#[derive(Component, Debug, Clone, Copy)]
pub(super) struct CheckCaption;

/// A check box with its caption to the right.
pub fn checkbox(theme: &Theme, caption: &str, on: bool) -> impl Bundle {
    let side = (theme.row_h * 0.7).round();
    (
        Checkbox { on },
        Focusable,
        WidgetState::Normal,
        Node { height: Val::Px(theme.row_h), align_items: AlignItems::Center, column_gap: Val::Px(theme.gap), ..default() },
        children![
            (
                CheckFrame,
                Pickable::IGNORE,
                Node {
                    width: Val::Px(side),
                    height: Val::Px(side),
                    padding: UiRect::all(Val::Px(3.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::px(theme.corners.min(4.0), theme.corners.min(4.0), theme.corners.min(4.0), theme.corners.min(4.0)),
                    flex_shrink: 0.0,
                    ..default()
                },
                BorderColor::all(theme.accent2),
                BackgroundColor(theme.track),
                children![(
                    CheckMark,
                    Pickable::IGNORE,
                    Node { width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
                    BackgroundColor(if on { theme.accent } else { Color::NONE }),
                )],
            ),
            (CheckCaption, kit_text(theme, caption, theme.text_size, theme.text)),
        ],
    )
}

/// Toggle a check box (the click and the Enter/Space activation).
pub fn click(w: &mut World, e: Entity) {
    if w.get::<WidgetState>(e) == Some(&WidgetState::Disabled) {
        return;
    }
    let Some(mut c) = w.get_mut::<Checkbox>(e) else { return };
    c.on = !c.on;
    let on = c.on;
    w.write_message(CheckboxChanged { entity: e, on });
}

pub(super) fn paint(
    theme: Res<Theme>,
    boxes: Query<(&Checkbox, &WidgetState, &Children)>,
    mut frames: Query<(&Children, &mut BorderColor), With<CheckFrame>>,
    mut marks: Query<&mut BackgroundColor, With<CheckMark>>,
    mut captions: Query<&mut SpriteText, With<CheckCaption>>,
) {
    for (c, state, kids) in &boxes {
        for kid in kids.iter() {
            if let Ok((inner, mut border)) = frames.get_mut(kid) {
                let want = match state {
                    WidgetState::Hovered | WidgetState::Pressed => theme.button_hot,
                    _ => theme.accent2,
                };
                if border.top != want {
                    *border = BorderColor::all(want);
                }
                for m in inner.iter() {
                    if let Ok(mut bg) = marks.get_mut(m) {
                        let want = match (c.on, *state == WidgetState::Disabled) {
                            (true, true) => theme.button_off,
                            (true, false) => theme.accent,
                            _ => Color::NONE,
                        };
                        if bg.0 != want {
                            bg.0 = want;
                        }
                    }
                }
            }
            if let Ok(mut t) = captions.get_mut(kid) {
                let want = if *state == WidgetState::Disabled { theme.text_off } else { theme.text };
                if t.color != want {
                    t.color = want;
                }
            }
        }
    }

}
