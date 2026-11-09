//! Interaction and look systems (`GameSet::Ui`) and pointer observers.

use bevy::picking::events::{PointerClick, PointerOut, PointerOver, PointerPress, PointerRelease};
use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;

use super::tokens::{Theme, scale};
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

/// The colour a state gives a widget's base colour.
pub fn state_colour(theme: &Theme, base: Color, state: WidgetState, on: Option<bool>) -> Color {
    let c = match state {
        WidgetState::Disabled => return theme.disabled,
        WidgetState::Normal => base,
        WidgetState::Hovered => scale(base, theme.hover_boost),
        WidgetState::Pressed => scale(base, theme.press_boost),
    };
    if on == Some(false) { scale(c, theme.off_dim) } else { c }
}

#[allow(clippy::type_complexity)]
pub fn paint_widgets(
    theme: Res<Theme>,
    mut q: Query<
        (&WidgetState, &Look, Option<&Toggle>, Option<&ListItem>, Option<&ChildOf>, &mut BackgroundColor),
        Without<Slider>,
    >,
    lists: Query<&List>,
) {
    for (state, look, toggle, item, parent, mut bg) in &mut q {
        let c = if let Some(&ListItem(i)) = item {
            let selected = parent.and_then(|p| lists.get(p.parent()).ok()).is_some_and(|l| l.selected == Some(i));
            match (state, selected) {
                (WidgetState::Hovered, false) => scale(theme.disabled, 0.6),
                (_, true) => state_colour(&theme, theme.orange, *state, None),
                _ => state_colour(&theme, theme.background, *state, None),
            }
        } else {
            state_colour(&theme, look.0, *state, toggle.map(|t| t.on))
        };
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
