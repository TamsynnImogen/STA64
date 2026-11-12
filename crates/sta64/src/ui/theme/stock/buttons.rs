//! Kit pills: the LCARS kit's `button` and `toggle` with labels in the kit's own
//! `kit_text_on` colour (light on the stock skins' dark pills). The LCARS builders
//! keep `text_on`, which the HUD also reads.

use bevy::prelude::*;

use super::{Theme, WidgetState, kit_text};
use crate::ui::sprite_font::SpriteText;
use crate::ui::theme::{ButtonText, Toggle, UiButton};

/// Marks a pill label the kit colours with `kit_text_on`.
#[derive(Component, Debug, Clone, Copy)]
pub struct KitLabel;

fn pill(theme: &Theme, w: f32) -> Node {
    Node {
        width: Val::Px(w),
        height: Val::Px(theme.button_h),
        padding: UiRect::horizontal(Val::Px(theme.button_h * 0.5)),
        align_items: AlignItems::Center,
        justify_content: if theme.caps_buttons { JustifyContent::FlexEnd } else { JustifyContent::Center },
        border_radius: BorderRadius::MAX,
        ..default()
    }
}

fn label(theme: &Theme, s: &str) -> impl Bundle {
    (ButtonText, KitLabel, kit_text(theme, theme.caps(s, theme.caps_buttons), theme.text_size, theme.kit_text_on))
}

/// A button whose label reads on the active skin's pill.
pub fn kit_button(theme: &Theme, s: &str, w: f32) -> impl Bundle {
    (UiButton, WidgetState::Normal, pill(theme, w), BackgroundColor(theme.button), children![label(theme, s)])
}

/// A toggle whose label reads on the active skin's pill.
pub fn kit_toggle(theme: &Theme, s: &str, on: bool, w: f32) -> impl Bundle {
    let c = if on { theme.button_on } else { theme.button };
    (Toggle { on }, WidgetState::Normal, pill(theme, w), BackgroundColor(c), children![label(theme, s)])
}

pub(super) fn paint(
    theme: Res<Theme>,
    states: Query<&WidgetState>,
    mut labels: Query<(&ChildOf, &mut SpriteText), With<KitLabel>>,
) {
    for (parent, mut t) in &mut labels {
        let disabled = states.get(parent.parent()).is_ok_and(|s| *s == WidgetState::Disabled);
        let want = if disabled { theme.text_off } else { theme.kit_text_on };
        if t.color != want {
            t.color = want;
        }
    }
}
