//! Tabs: a bar of labelled tabs and the panels it shows. Spawn the bar, then a
//! [`tab_panel`] per tab naming the bar's entity; only the selected panel is displayed.

use bevy::prelude::*;

use super::focus::KitKey;
use super::{Focusable, Theme, WidgetState, kit_text};
use crate::ui::sprite_font::SpriteText;
use crate::ui::theme::{ButtonText, state_colour};

#[derive(Component, Debug, Clone, Copy)]
pub struct TabBar {
    pub selected: usize,
    pub count: usize,
}

/// One tab of a bar (child of the [`TabBar`]).
#[derive(Component, Debug, Clone, Copy)]
pub struct Tab(pub usize);

/// Content shown while its bar has tab `index` selected.
#[derive(Component, Debug, Clone, Copy)]
pub struct TabPanel {
    pub bar: Entity,
    pub index: usize,
}

#[derive(Message, Debug, Clone, Copy)]
pub struct TabChanged {
    pub bar: Entity,
    pub index: usize,
}

/// A row of tabs.
pub fn tab_bar(theme: &Theme, labels: &[&str], selected: usize) -> impl Bundle {
    let tabs: Vec<_> = labels
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let r = theme.corners.min(theme.button_h / 2.0);
            (
                Tab(i),
                WidgetState::Normal,
                Node {
                    height: Val::Px(theme.button_h),
                    padding: UiRect::horizontal(Val::Px(theme.pad * 1.5)),
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius::px(r, r, 0.0, 0.0),
                    ..default()
                },
                BackgroundColor(theme.button),
                children![(ButtonText, kit_text(theme, theme.caps(s, theme.caps_buttons), theme.text_size, theme.text_on))],
            )
        })
        .collect();
    (
        TabBar { selected: selected.min(labels.len().saturating_sub(1)), count: labels.len() },
        Focusable,
        Node { column_gap: Val::Px(theme.bar_gap.max(2.0)), align_items: AlignItems::FlexEnd, ..default() },
        Children::spawn(bevy::ecs::spawn::SpawnIter(tabs.into_iter())),
    )
}

/// The content of tab `index` of `bar`.
pub fn tab_panel(bar: Entity, index: usize) -> impl Bundle {
    (TabPanel { bar, index }, Node { flex_direction: FlexDirection::Column, ..default() })
}

pub fn select(w: &mut World, bar: Entity, index: usize) {
    let Some(mut b) = w.get_mut::<TabBar>(bar) else { return };
    if index >= b.count || b.selected == index {
        return;
    }
    b.selected = index;
    w.write_message(TabChanged { bar, index });
}

/// Click on a tab.
pub fn click(w: &mut World, e: Entity) {
    if w.get::<WidgetState>(e) == Some(&WidgetState::Disabled) {
        return;
    }
    if let (Some(&Tab(i)), Some(parent)) = (w.get::<Tab>(e), w.get::<ChildOf>(e).map(|p| p.parent())) {
        select(w, parent, i);
    }
}

pub fn key(w: &mut World, e: Entity, k: &KitKey) {
    let Some(b) = w.get::<TabBar>(e).copied() else { return };
    match k.code {
        KeyCode::ArrowRight => select(w, e, (b.selected + 1).min(b.count.saturating_sub(1))),
        KeyCode::ArrowLeft => select(w, e, b.selected.saturating_sub(1)),
        KeyCode::Home => select(w, e, 0),
        KeyCode::End => select(w, e, b.count.saturating_sub(1)),
        _ => {}
    }
}

/// Show the selected panel only; paint the selected tab in the "on" colour.
#[allow(clippy::type_complexity)]
pub(super) fn sync(
    theme: Res<Theme>,
    bars: Query<&TabBar>,
    mut panels: Query<(&TabPanel, &mut Node)>,
    mut tabs: Query<(&Tab, &WidgetState, &ChildOf, &Children, &mut BackgroundColor)>,
    mut labels: Query<&mut SpriteText, With<ButtonText>>,
) {
    for (p, mut node) in &mut panels {
        let want = if bars.get(p.bar).is_ok_and(|b| b.selected == p.index) { Display::Flex } else { Display::None };
        if node.display != want {
            node.display = want;
        }
    }
    for (Tab(i), state, parent, kids, mut bg) in &mut tabs {
        let Ok(bar) = bars.get(parent.parent()) else { continue };
        let on = bar.selected == *i;
        let want = state_colour(&theme, *state, Some(on));
        if bg.0 != want {
            bg.0 = want;
        }
        let ink = if *state == WidgetState::Disabled { theme.text_off } else { theme.text_on };
        for kid in kids.iter() {
            if let Ok(mut t) = labels.get_mut(kid)
                && t.color != ink
            {
                t.color = ink;
            }
        }
    }
}
