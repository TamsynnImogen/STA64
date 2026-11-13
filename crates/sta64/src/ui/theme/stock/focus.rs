//! Keyboard focus for the widget kit. One focused widget at a time ([`KitFocus`]);
//! a press focuses the nearest [`Focusable`] under the pointer (or clears focus),
//! Tab and Shift+Tab walk the focusable widgets in tree order, and every other key
//! goes to the focused widget's own handler (`kit_key`).

use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::picking::hover::HoverMap;
use bevy::prelude::*;

use super::WidgetState;

/// The focused widget, if any.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct KitFocus(pub Option<Entity>);

/// Marks a widget the keyboard can reach. The kit's dropdowns, text fields, tab bars,
/// check boxes and scrolling lists carry it; add it to a button, toggle or slider with
/// [`focusable`].
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct Focusable;

/// Make any widget bundle keyboard reachable (Enter and Space activate buttons and
/// toggles; a slider answers Left and Right).
pub fn focusable(widget: impl Bundle) -> impl Bundle {
    (widget, Focusable)
}

/// A key press, reduced to what the kit reads.
#[derive(Debug, Clone)]
pub struct KitKey {
    pub code: KeyCode,
    /// The text the key types, if it types any.
    pub text: Option<String>,
    pub shift: bool,
}

impl KitKey {
    pub fn code(code: KeyCode) -> Self {
        KitKey { code, text: None, shift: false }
    }
    pub fn typed(s: &str) -> Self {
        KitKey { code: KeyCode::Unidentified(bevy::input::keyboard::NativeKeyCode::Unidentified), text: Some(s.to_owned()), shift: false }
    }
}

/// Entities the pointer is over, nearest first.
pub fn hovered(hover: &HoverMap) -> Vec<Entity> {
    let mut hits: Vec<(Entity, f32)> = hover.values().flat_map(|m| m.iter().map(|(e, hit)| (*e, hit.depth))).collect();
    hits.sort_by(|a, b| a.1.total_cmp(&b.1));
    hits.into_iter().map(|(e, _)| e).collect()
}

/// Whether `e` is `root` or below it.
pub fn is_within(w: &World, mut e: Entity, root: Entity) -> bool {
    loop {
        if e == root {
            return true;
        }
        match w.get::<ChildOf>(e) {
            Some(p) => e = p.parent(),
            None => return false,
        }
    }
}

/// The nearest [`Focusable`] at or above `e`.
pub fn focusable_ancestor(w: &World, mut e: Entity) -> Option<Entity> {
    loop {
        if w.get::<Focusable>(e).is_some() {
            return Some(e);
        }
        // An open dropdown list is a root node: it belongs to its field.
        if let Some(p) = w.get::<super::DropdownPopup>(e) {
            e = p.owner;
            continue;
        }
        e = w.get::<ChildOf>(e)?.parent();
    }
}

/// Marks the root of a modal screen (an options menu over the match, an editor
/// dialog): while any exists, Tab walks only the focusables inside scopes.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct FocusScope;

/// Whether the kit wants the keyboard or the wheel this frame, for game systems to
/// check before acting on the same input (match hotkeys while typing, camera zoom
/// while the pointer scrolls a list, the screen's Back on an Escape that closed a
/// dropdown).
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KitCapture {
    /// A text field has focus, or the focused dropdown is open.
    pub keyboard: bool,
    /// The pointer is over an open dropdown list or a scrolling list.
    pub pointer: bool,
}

/// Whether `e` and everything above it is displayed.
pub fn is_displayed(w: &World, mut e: Entity) -> bool {
    loop {
        if w.get::<Node>(e).is_some_and(|n| n.display == Display::None) {
            return false;
        }
        match w.get::<ChildOf>(e) {
            Some(p) => e = p.parent(),
            None => return true,
        }
    }
}

/// Every focusable, enabled, displayed widget in tree order.
pub fn focus_order(w: &mut World) -> Vec<Entity> {
    let mut roots: Vec<Entity> = w.query_filtered::<Entity, With<FocusScope>>().iter(w).collect();
    if roots.is_empty() {
        roots = w.query_filtered::<Entity, (With<Node>, Without<ChildOf>)>().iter(w).collect();
    }
    roots.sort();
    let mut out = Vec::new();
    let mut stack: Vec<Entity> = roots.into_iter().rev().collect();
    while let Some(e) = stack.pop() {
        if w.get::<Node>(e).is_some_and(|n| n.display == Display::None) {
            continue;
        }
        if w.get::<Focusable>(e).is_some() && w.get::<WidgetState>(e) != Some(&WidgetState::Disabled) {
            out.push(e);
        }
        if let Some(kids) = w.get::<Children>(e) {
            stack.extend(kids.iter().rev());
        }
    }
    out
}

/// Move focus to the next (or previous) focusable; wraps. With none focused, the
/// first (last).
pub fn move_focus(w: &mut World, back: bool) {
    let order = focus_order(w);
    if order.is_empty() {
        return;
    }
    let at = w.resource::<KitFocus>().0.and_then(|f| order.iter().position(|e| *e == f));
    let next = match (at, back) {
        (None, false) => 0,
        (None, true) => order.len() - 1,
        (Some(i), false) => (i + 1) % order.len(),
        (Some(i), true) => (i + order.len() - 1) % order.len(),
    };
    w.resource_mut::<KitFocus>().0 = Some(order[next]);
}

/// A press at `target` (the nearest thing under the pointer, `None` over nothing):
/// closes dropdowns the press is outside of and focuses the nearest focusable.
pub fn press(w: &mut World, hits: &[Entity]) {
    super::dropdown::dismiss_outside(w, hits);
    // A disabled widget does not take focus (and leaves none behind).
    let focus = hits
        .first()
        .and_then(|e| focusable_ancestor(w, *e))
        .filter(|f| w.get::<WidgetState>(*f) != Some(&WidgetState::Disabled));
    w.resource_mut::<KitFocus>().0 = focus;
}

pub(super) fn pointer_press(mouse: Res<ButtonInput<MouseButton>>, hover: Res<HoverMap>, mut commands: Commands) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let hits = hovered(&hover);
    commands.queue(move |w: &mut World| press(w, &hits));
}

pub(super) fn key_events(mut keys: MessageReader<KeyboardInput>, held: Res<ButtonInput<KeyCode>>, mut commands: Commands) {
    for ev in keys.read() {
        if ev.state != ButtonState::Pressed {
            continue;
        }
        let shift = held.pressed(KeyCode::ShiftLeft) || held.pressed(KeyCode::ShiftRight);
        let text = ev.text.as_ref().map(|t| t.to_string()).or_else(|| match &ev.logical_key {
            Key::Space => Some(" ".to_owned()),
            _ => None,
        });
        let key = KitKey { code: ev.key_code, text, shift };
        commands.queue(move |w: &mut World| kit_key(w, key));
    }
}

/// Route a key press: Tab moves focus, everything else goes to the focused widget.
/// Public so tests (and scripted input) press keys without a window.
pub fn kit_key(w: &mut World, key: KitKey) {
    if w.get_resource::<crate::platform::LifecycleGate>().is_some_and(|gate| !gate.is_foreground()) {
        return;
    }
    let mut focused = w.get_resource::<KitFocus>().and_then(|f| f.0);
    // Focus on something despawned or hidden (a tab panel switched away) is dropped,
    // and the key goes on as if nothing had focus.
    if let Some(e) = focused
        && (w.get_entity(e).is_err() || !is_displayed(w, e))
    {
        super::dropdown::close(w, e);
        w.resource_mut::<KitFocus>().0 = None;
        focused = None;
    }
    let Some(e) = focused else {
        if key.code == KeyCode::Tab {
            move_focus(w, key.shift);
        }
        return;
    };
    if key.code == KeyCode::Tab {
        super::dropdown::close(w, e);
        move_focus(w, key.shift);
        return;
    }
    if w.get::<WidgetState>(e) == Some(&WidgetState::Disabled) {
        return;
    }
    if w.get::<super::TextField>(e).is_some() {
        super::text_field::key(w, e, &key);
    } else if w.get::<super::Dropdown>(e).is_some() {
        super::dropdown::key(w, e, &key);
    } else if w.get::<super::TabBar>(e).is_some() {
        super::tabs::key(w, e, &key);
    } else if w.get::<super::ScrollArea>(e).is_some() {
        super::scroll::key(w, e, &key);
    } else if w.get::<crate::ui::theme::Slider>(e).is_some() {
        super::scroll::slider_key(w, e, &key);
    } else if matches!(key.code, KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space) {
        // Buttons, toggles and check boxes.
        crate::ui::theme::click(w, e);
        super::checkbox::click(w, e);
    }
}

/// Publish [`KitCapture`] for this frame.
pub(super) fn capture(
    focus: Res<KitFocus>,
    fields: Query<(), With<super::TextField>>,
    dropdowns: Query<&super::Dropdown>,
    hover: Res<HoverMap>,
    areas: Query<(), With<super::ScrollArea>>,
    parents: Query<&ChildOf>,
    mut out: ResMut<KitCapture>,
) {
    let keyboard = focus.0.is_some_and(|e| fields.contains(e) || dropdowns.get(e).is_ok_and(|d| d.open));
    let pointer = hovered(&hover).into_iter().any(|h| {
        let mut e = h;
        loop {
            if areas.contains(e) {
                return true;
            }
            match parents.get(e) {
                Ok(p) => e = p.parent(),
                Err(_) => return false,
            }
        }
    });
    let now = KitCapture { keyboard, pointer };
    if *out != now {
        *out = now;
    }
}

/// The focus ring: an outline in the theme's `focus` colour on the focused widget.
pub(super) fn paint_focus(
    focus: Res<KitFocus>,
    theme: Res<crate::ui::theme::Theme>,
    q: Query<(Entity, Option<&Outline>), With<Focusable>>,
    mut commands: Commands,
) {
    if !focus.is_changed() && !theme.is_changed() {
        return;
    }
    for (e, outline) in &q {
        let want = focus.0 == Some(e);
        match (want, outline.is_some()) {
            (true, _) => {
                commands.entity(e).insert(Outline::new(Val::Px(2.0), Val::Px(1.0), theme.focus));
            }
            (false, true) => {
                commands.entity(e).remove::<Outline>();
            }
            _ => {}
        }
    }
}
