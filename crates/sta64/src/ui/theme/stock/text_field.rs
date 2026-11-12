//! Single-line text field. Click to focus (the caret shows as a bar in the text);
//! type, Backspace, Delete, Left, Right, Home and End edit; Enter reports
//! `TextSubmitted` and leaves the field; Escape leaves it. The field never holds more
//! than `max_len` characters nor any the [`TextFilter`] refuses.

use bevy::prelude::*;

use super::focus::{KitFocus, KitKey};
use super::{Focusable, Theme, WidgetState, kit_text};
use crate::ui::sprite_font::SpriteText;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextFilter {
    #[default]
    Any,
    Digits,
    /// Letters, digits, space and `_ - .` (player and file names).
    Name,
}

impl TextFilter {
    pub fn allows(self, c: char) -> bool {
        match self {
            Self::Any => !c.is_control(),
            Self::Digits => c.is_ascii_digit(),
            Self::Name => c.is_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.'),
        }
    }
}

#[derive(Component, Debug, Clone)]
pub struct TextField {
    pub text: String,
    /// The caret, in characters from the start.
    pub cursor: usize,
    pub max_len: usize,
    pub filter: TextFilter,
    pub placeholder: String,
}

impl TextField {
    fn byte_at(&self, chars: usize) -> usize {
        self.text.char_indices().nth(chars).map_or(self.text.len(), |(i, _)| i)
    }
    fn len(&self) -> usize {
        self.text.chars().count()
    }
}

/// The text changed.
#[derive(Message, Debug, Clone, Copy)]
pub struct TextChanged {
    pub entity: Entity,
}

/// Enter was pressed in the field.
#[derive(Message, Debug, Clone, Copy)]
pub struct TextSubmitted {
    pub entity: Entity,
}

#[derive(Component, Debug, Clone, Copy)]
pub(super) struct FieldText;

/// A text field `w` wide holding `initial`.
pub fn text_field(theme: &Theme, initial: &str, placeholder: &str, w: f32, max_len: usize, filter: TextFilter) -> impl Bundle {
    let text: String = initial.chars().filter(|c| filter.allows(*c)).take(max_len).collect();
    let cursor = text.chars().count();
    let r = theme.corners.min(theme.button_h / 2.0);
    (
        TextField { text: text.clone(), cursor, max_len, filter, placeholder: placeholder.to_owned() },
        Focusable,
        WidgetState::Normal,
        Node {
            width: Val::Px(w),
            height: Val::Px(theme.button_h),
            padding: UiRect::horizontal(Val::Px(theme.pad)),
            align_items: AlignItems::Center,
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::px(r, r, r, r),
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(theme.track),
        BorderColor::all(theme.accent2),
        children![(FieldText, kit_text(theme, text, theme.text_size, theme.text))],
    )
}

pub fn key(w: &mut World, e: Entity, k: &KitKey) {
    let Some(mut f) = w.get_mut::<TextField>(e) else { return };
    let before = f.text.clone();
    match k.code {
        KeyCode::Enter | KeyCode::NumpadEnter => {
            w.resource_mut::<KitFocus>().0 = None;
            w.write_message(TextSubmitted { entity: e });
            return;
        }
        KeyCode::Escape => {
            w.resource_mut::<KitFocus>().0 = None;
            return;
        }
        KeyCode::ArrowLeft => f.cursor = f.cursor.saturating_sub(1),
        KeyCode::ArrowRight => f.cursor = (f.cursor + 1).min(f.len()),
        KeyCode::Home => f.cursor = 0,
        KeyCode::End => f.cursor = f.len(),
        KeyCode::Backspace if f.cursor > 0 => {
            let at = f.byte_at(f.cursor - 1);
            f.text.remove(at);
            f.cursor -= 1;
        }
        KeyCode::Delete if f.cursor < f.len() => {
            let at = f.byte_at(f.cursor);
            f.text.remove(at);
        }
        _ => {
            if let Some(typed) = &k.text {
                let filter = f.filter;
                for c in typed.chars().filter(|c| filter.allows(*c)) {
                    if f.len() >= f.max_len {
                        break;
                    }
                    let at = f.byte_at(f.cursor);
                    f.text.insert(at, c);
                    f.cursor += 1;
                }
            }
        }
    }
    if f.text != before {
        w.write_message(TextChanged { entity: e });
    }
}

/// The text shows the caret while the field is focused, the placeholder (dim) while
/// it is empty and not.
#[allow(clippy::type_complexity)]
pub(super) fn sync(
    theme: Res<Theme>,
    focus: Res<KitFocus>,
    fields: Query<(Entity, Ref<TextField>, Ref<WidgetState>, &Children), Without<FieldText>>,
    mut texts: Query<&mut SpriteText, With<FieldText>>,
    mut paint: Query<(&mut BorderColor, &mut BackgroundColor), With<TextField>>,
) {
    for (e, f, state, kids) in &fields {
        let focused = focus.0 == Some(e);
        if let Ok((mut border, mut bg)) = paint.get_mut(e) {
            let edge = if focused { theme.focus } else { theme.accent2 };
            if border.top != edge {
                *border = BorderColor::all(edge);
            }
            if bg.0 != theme.track {
                bg.0 = theme.track;
            }
        }
        if !(f.is_changed() || focus.is_changed() || theme.is_changed() || state.is_changed()) {
            continue;
        }
        let (shown, colour) = if focused {
            let at = f.byte_at(f.cursor);
            (format!("{}|{}", &f.text[..at], &f.text[at..]), theme.text)
        } else if f.text.is_empty() {
            (f.placeholder.clone(), theme.text_dim)
        } else {
            (f.text.clone(), theme.text)
        };
        let colour = if *state == WidgetState::Disabled { theme.text_off } else { colour };
        for kid in kids.iter() {
            if let Ok(mut t) = texts.get_mut(kid) {
                if t.text != shown {
                    t.text = shown.clone();
                }
                if t.color != colour {
                    t.color = colour;
                }
            }
        }
    }
}
