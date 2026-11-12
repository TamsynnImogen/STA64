//! Interaction and look systems (`GameSet::Ui`) and pointer observers.

use bevy::picking::events::{PointerClick, PointerOut, PointerOver, PointerPress, PointerRelease};
use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;

use super::tokens::{Theme, mix};
use super::widgets::*;

/// A button was clicked.
#[derive(Message, Debug, Clone, Copy)]
pub struct ButtonClicked(pub Entity);

/// A toggle changed.
#[derive(Message, Debug, Clone, Copy)]
pub struct ToggleChanged {
    pub entity: Entity,
    pub on: bool,
}

/// A slider's value changed by the pointer.
#[derive(Message, Debug, Clone, Copy)]
pub struct SliderChanged {
    pub entity: Entity,
    pub value: f32,
}

/// A list row was chosen.
#[derive(Message, Debug, Clone, Copy)]
pub struct ListSelected {
    pub list: Entity,
    pub index: usize,
}

fn set_state(states: &mut Query<&mut WidgetState>, e: Entity, to: WidgetState, only_from: Option<WidgetState>) {
    if let Ok(mut s) = states.get_mut(e)
        && *s != WidgetState::Disabled
        && only_from.is_none_or(|f| *s == f)
    {
        *s = to;
    }
}

pub fn on_over(ev: On<PointerOver>, mut q: Query<&mut WidgetState>) {
    set_state(&mut q, ev.entity, WidgetState::Hovered, Some(WidgetState::Normal));
}
pub fn on_out(ev: On<PointerOut>, mut q: Query<&mut WidgetState>) {
    set_state(&mut q, ev.entity, WidgetState::Normal, None);
}
pub fn on_press(ev: On<PointerPress>, mut q: Query<&mut WidgetState>) {
    set_state(&mut q, ev.entity, WidgetState::Pressed, None);
}
pub fn on_release(ev: On<PointerRelease>, mut q: Query<&mut WidgetState>) {
    set_state(&mut q, ev.entity, WidgetState::Hovered, Some(WidgetState::Pressed));
}

/// Pointer click: the widget's kind decides what happens.
pub fn on_click(ev: On<PointerClick>, mut commands: Commands) {
    let e = ev.entity;
    commands.queue(move |w: &mut World| click(w, e));
}

/// What a click on `e` does. Public so tests can click without a pointer.
pub fn click(w: &mut World, e: Entity) {
    if w.get_resource::<crate::platform::LifecycleGate>().is_some_and(|gate| !gate.is_foreground()) {
        return;
    }
    if w.get::<WidgetState>(e) == Some(&WidgetState::Disabled) {
        return;
    }
    if w.get::<UiButton>(e).is_some() {
        w.write_message(ButtonClicked(e));
    } else if let Some(mut t) = w.get_mut::<Toggle>(e) {
        t.on = !t.on;
        let on = t.on;
        w.write_message(ToggleChanged { entity: e, on });
    } else if let (Some(&ListItem(i)), Some(&ChildOf(list))) = (w.get::<ListItem>(e), w.get::<ChildOf>(e)) {
        if let Some(mut l) = w.get_mut::<List>(list) {
            l.selected = Some(i);
        }
        w.write_message(ListSelected { list, index: i });
    }
}

/// Dragging across a pressed slider sets its value from the cursor.
pub fn drive_sliders(
    mut sliders: Query<(Entity, &WidgetState, &RelativeCursorPosition, &mut Slider)>,
    mut out: MessageWriter<SliderChanged>,
) {
    for (entity, state, rel, mut s) in &mut sliders {
        if *state != WidgetState::Pressed {
            continue;
        }
        let Some(p) = rel.normalized else { continue };
        let value = s.min + (p.x + 0.5).clamp(0.0, 1.0) * (s.max - s.min);
        if (value - s.value).abs() > f32::EPSILON {
            s.value = value;
            out.write(SliderChanged { entity, value });
        }
    }
}

/// The fill a button (`on: None`) or toggle has in a state, from the theme's
/// button roles: `button` -> `button_hot` on hover -> `button_down` pressed;
/// a toggle that is on is `button_on` (gold), eased half way to `button_hot`
/// on hover; disabled is `button_off`.
pub fn state_colour(theme: &Theme, state: WidgetState, on: Option<bool>) -> Color {
    match (state, on) {
        (WidgetState::Disabled, _) => theme.button_off,
        (WidgetState::Pressed, _) => theme.button_down,
        (WidgetState::Hovered, Some(true)) => mix(theme.button_on, theme.button_hot, theme.on_hover_mix),
        (WidgetState::Hovered, _) => theme.button_hot,
        (WidgetState::Normal, Some(true)) => theme.button_on,
        (WidgetState::Normal, _) => theme.button,
    }
}

/// A list row's fill: selected `row_sel`, hovered `row_hot`, odd rows faintly
/// lit (`row_alt`), else clear.
pub fn row_colour(theme: &Theme, state: WidgetState, selected: bool, index: usize) -> Color {
    if selected {
        theme.row_sel
    } else if state == WidgetState::Hovered {
        theme.row_hot
    } else if index % 2 == 1 {
        theme.row_alt
    } else {
        Color::NONE
    }
}

#[allow(clippy::type_complexity)]
pub fn paint_widgets(
    theme: Res<Theme>,
    mut q: Query<
        (&WidgetState, Option<&Toggle>, Option<&ListItem>, Option<&ChildOf>, &mut BackgroundColor),
        Without<Slider>,
    >,
    lists: Query<&List>,
    mut texts: Query<(&ChildOf, &mut crate::ui::sprite_font::SpriteText), With<ButtonText>>,
    mut plain: Query<(&ChildOf, &mut TextColor), (With<ButtonText>, Without<crate::ui::sprite_font::SpriteText>)>,
    mut marks: Query<(&ChildOf, &mut BackgroundColor), (With<RowMark>, Without<WidgetState>)>,
) {
    for (state, toggle, item, parent, mut bg) in &mut q {
        let c = if let Some(&ListItem(i)) = item {
            let selected = parent.and_then(|p| lists.get(p.parent()).ok()).is_some_and(|l| l.selected == Some(i));
            row_colour(&theme, *state, selected, i)
        } else {
            state_colour(&theme, *state, toggle.map(|t| t.on))
        };
        if bg.0 != c {
            bg.0 = c;
        }
    }
    for (parent, mut tc) in &mut texts {
        let disabled = q.get(parent.parent()).is_ok_and(|(s, ..)| *s == WidgetState::Disabled);
        let c = if disabled { theme.text_off } else { theme.kit_text_on };
        if tc.color != c {
            tc.color = c;
        }
    }
    for (parent, mut tc) in &mut plain {
        let disabled = q.get(parent.parent()).is_ok_and(|(s, ..)| *s == WidgetState::Disabled);
        let c = if disabled { theme.text_off } else { theme.text_on };
        if tc.0 != c {
            tc.0 = c;
        }
    }
    for (parent, mut bg) in &mut marks {
        let selected = q
            .get(parent.parent())
            .ok()
            .and_then(|(_, _, item, list, _)| Some((item?.0, list?.parent())))
            .and_then(|(i, l)| lists.get(l).ok().map(|l| l.selected == Some(i)))
            .unwrap_or(false);
        let c = if selected { theme.accent } else { Color::NONE };
        if bg.0 != c {
            bg.0 = c;
        }
    }
}

pub fn fill_bars(
    sliders: Query<(&Slider, &Children), Changed<Slider>>,
    progress: Query<(&ProgressBar, &Children), Changed<ProgressBar>>,
    mut fills: Query<&mut Node, With<Fill>>,
) {
    let mut set = |kids: &Children, f: f32| {
        for k in kids.iter() {
            if let Ok(mut n) = fills.get_mut(k) {
                n.width = Val::Percent(f.clamp(0.0, 1.0) * 100.0);
            }
        }
    };
    for (s, kids) in &sliders {
        set(kids, (s.value - s.min) / (s.max - s.min).max(f32::EPSILON));
    }
    for (p, kids) in &progress {
        set(kids, p.0);
    }
}
