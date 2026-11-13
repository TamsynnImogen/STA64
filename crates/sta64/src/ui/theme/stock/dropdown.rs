//! Dropdown select (faction, colour, team, map size): a closed field showing the
//! choice that opens a list over everything below it. The list is a root UI node that
//! only points back at its field ([`DropdownPopup::owner`]), so an ancestor of the
//! field that clips (`Overflow`, a scrolling page) cannot clip it. It is placed from
//! the field's laid-out rectangle each frame, drawn on a global z layer, scrolls past
//! [`Dropdown::max_rows`] rows, opens
//! below the field (above it when it would leave the window), and closes on a choice,
//! on Escape and on a press outside the field and its list.
//!
//! Mouse: press the field to open, press a row to choose. Keyboard (field focused):
//! Up/Down change the choice while closed; Enter or Space opens; open, Up/Down/Home/End
//! move the highlight, Enter or Space chooses, Escape closes.

use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::focus::{KitKey, is_within};
use super::scroll::{ScrollArea, scroll_to_row};
use super::{Focusable, Theme, WidgetState, kit_text};
use crate::ui::sprite_font::SpriteText;
use crate::ui::theme::{row_colour, state_colour};

/// Rows an open list shows before it scrolls (Refit's dropdown shows eight).
pub const MAX_ROWS: usize = 8;

#[derive(Component, Debug, Clone)]
pub struct Dropdown {
    pub options: Vec<String>,
    pub selected: usize,
    pub open: bool,
    /// The row Enter would choose while open.
    pub highlight: usize,
    pub max_rows: usize,
}

impl Dropdown {
    pub fn value(&self) -> Option<&str> {
        self.options.get(self.selected).map(String::as_str)
    }
}

/// The choice changed.
#[derive(Message, Debug, Clone, Copy)]
pub struct DropdownChanged {
    pub entity: Entity,
    pub index: usize,
}

#[derive(Component, Debug, Clone, Copy)]
pub(super) struct DropdownLabel;
#[derive(Component, Debug, Clone, Copy)]
pub(super) struct DropdownArrow;
/// The open list. `owner` is its field (the list is not a child of it); `count` is
/// how many rows it was built with.
#[derive(Component, Debug, Clone, Copy)]
pub struct DropdownPopup {
    pub owner: Entity,
    pub count: usize,
    /// Hash of the option texts it shows, to rebuild the list when they change.
    pub signature: u64,
}

fn signature(options: &[String]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    options.hash(&mut h);
    h.finish()
}
#[derive(Component, Debug, Clone, Copy)]
pub struct DropdownOption(pub usize);

fn row_h(theme: &Theme) -> f32 {
    theme.list_row_h.max(theme.small_size + 8.0)
}

/// A dropdown `w` wide over `options`, showing `selected`.
pub fn dropdown(theme: &Theme, options: &[&str], selected: usize, w: f32) -> impl Bundle {
    let selected = selected.min(options.len().saturating_sub(1));
    let shown = options.get(selected).copied().unwrap_or("").to_owned();
    let r = theme.corners.min(theme.button_h / 2.0);
    (
        Dropdown { options: options.iter().map(|s| (*s).to_owned()).collect(), selected, open: false, highlight: selected, max_rows: MAX_ROWS },
        Focusable,
        WidgetState::Normal,
        Node {
            width: Val::Px(w),
            height: Val::Px(theme.button_h),
            padding: UiRect::horizontal(Val::Px(theme.pad)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::SpaceBetween,
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::px(r, r, r, r),
            ..default()
        },
        BackgroundColor(theme.button),
        BorderColor::all(theme.accent2),
        children![
            (DropdownLabel, kit_text(theme, shown, theme.text_size, theme.kit_text_on)),
            (
                DropdownArrow,
                Pickable::IGNORE,
                Node {
                    width: Val::Px(8.0),
                    height: Val::Px(8.0),
                    border: UiRect { right: Val::Px(2.0), bottom: Val::Px(2.0), ..default() },
                    margin: UiRect::bottom(Val::Px(4.0)),
                    ..default()
                },
                BorderColor::all(theme.kit_text_on),
                UiTransform::from_rotation(Rot2::degrees(45.0)),
            ),
        ],
    )
}

/// Open or close a field by hand (the click uses this).
pub fn set_open(w: &mut World, e: Entity, open: bool) {
    if let Some(mut d) = w.get_mut::<Dropdown>(e)
        && d.open != open
    {
        d.open = open;
        if open {
            d.highlight = d.selected;
        }
    }
}

pub fn close(w: &mut World, e: Entity) {
    set_open(w, e, false);
}

/// Choose row `index`: set it, close, report a change.
pub fn choose(w: &mut World, e: Entity, index: usize) {
    let Some(mut d) = w.get_mut::<Dropdown>(e) else { return };
    if index >= d.options.len() {
        return;
    }
    let changed = d.selected != index;
    d.selected = index;
    d.highlight = index;
    d.open = false;
    if changed {
        w.write_message(DropdownChanged { entity: e, index });
    }
}

/// Click on a field or on one of its rows.
pub fn click(w: &mut World, e: Entity) {
    if w.get::<WidgetState>(e) == Some(&WidgetState::Disabled) {
        return;
    }
    if let Some(&DropdownOption(i)) = w.get::<DropdownOption>(e) {
        let owner = w.get::<ChildOf>(e).and_then(|p| w.get::<DropdownPopup>(p.parent())).map(|p| p.owner);
        if let Some(owner) = owner {
            choose(w, owner, i);
        }
    } else if let Some(d) = w.get::<Dropdown>(e) {
        let open = !d.open;
        set_open(w, e, open);
    }
}

/// The field whose list `e` is in (or is), if it is in one.
pub fn popup_of(w: &World, mut e: Entity) -> Option<Entity> {
    loop {
        if let Some(p) = w.get::<DropdownPopup>(e) {
            return Some(p.owner);
        }
        e = w.get::<ChildOf>(e)?.parent();
    }
}

/// A press at `hits` (what is under the pointer): every open dropdown the press is
/// not inside closes.
pub fn dismiss_outside(w: &mut World, hits: &[Entity]) {
    let open: Vec<Entity> = w.query::<(Entity, &Dropdown)>().iter(w).filter(|(_, d)| d.open).map(|(e, _)| e).collect();
    for d in open {
        if !hits.iter().any(|h| is_within(w, *h, d) || popup_of(w, *h) == Some(d)) {
            close(w, d);
        }
    }
}

pub fn key(w: &mut World, e: Entity, k: &KitKey) {
    let Some(d) = w.get::<Dropdown>(e).cloned() else { return };
    let n = d.options.len();
    if n == 0 {
        return;
    }
    let activate = matches!(k.code, KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space);
    if d.open {
        let to = match k.code {
            KeyCode::ArrowDown => Some((d.highlight + 1).min(n - 1)),
            KeyCode::ArrowUp => Some(d.highlight.saturating_sub(1)),
            KeyCode::Home => Some(0),
            KeyCode::End => Some(n - 1),
            KeyCode::PageDown => Some((d.highlight + d.max_rows).min(n - 1)),
            KeyCode::PageUp => Some(d.highlight.saturating_sub(d.max_rows)),
            _ => None,
        };
        if let Some(to) = to {
            w.get_mut::<Dropdown>(e).unwrap().highlight = to;
        } else if activate {
            choose(w, e, d.highlight);
        } else if k.code == KeyCode::Escape {
            close(w, e);
        }
    } else if activate {
        set_open(w, e, true);
    } else {
        let to = match k.code {
            KeyCode::ArrowDown => (d.selected + 1).min(n - 1),
            KeyCode::ArrowUp => d.selected.saturating_sub(1),
            _ => return,
        };
        choose(w, e, to);
    }
}

/// Keep the field's label, its list and the list's scroll in step with the component.
#[allow(clippy::type_complexity)]
pub(super) fn sync(
    mut commands: Commands,
    theme: Res<Theme>,
    changed: Query<(Entity, &Dropdown, &Children), Changed<Dropdown>>,
    popups: Query<(Entity, &DropdownPopup, &ScrollArea)>,
    mut labels: Query<&mut SpriteText, With<DropdownLabel>>,
    mut scrolls: Query<(&ScrollArea, &mut ScrollPosition)>,
) {
    for (e, d, kids) in &changed {
        for kid in kids.iter() {
            if let Ok(mut t) = labels.get_mut(kid) {
                let shown = d.value().unwrap_or("");
                if t.text != shown {
                    t.text = shown.to_owned();
                }
            }
        }
        let popup = popups.iter().find(|(_, pop, _)| pop.owner == e);
        match (d.open, popup) {
            (true, None) => spawn_popup(&mut commands, &theme, e, d),
            (true, Some((p, pop, _))) if pop.count != d.options.len() || pop.signature != signature(&d.options) => {
                commands.entity(p).despawn();
                spawn_popup(&mut commands, &theme, e, d);
            }
            (true, Some((p, ..))) => {
                if let Ok((area, mut pos)) = scrolls.get_mut(p) {
                    scroll_to_row(area, &mut pos, d.highlight);
                }
            }
            (false, Some((p, ..))) => commands.entity(p).despawn(),
            (false, None) => {}
        }
    }
}

fn spawn_popup(commands: &mut Commands, theme: &Theme, owner: Entity, d: &Dropdown) {
    let rows = d.options.len().min(d.max_rows).max(1);
    let h = row_h(theme);
    let area = ScrollArea { row_h: h, rows, count: d.options.len() };
    let mut pos = ScrollPosition::default();
    scroll_to_row(&area, &mut pos, d.highlight);
    let popup = commands
        .spawn((
            DropdownPopup { owner, count: d.options.len(), signature: signature(&d.options) },
            // Hidden until `place_popups` has the field's rectangle and the list's size.
            Visibility::Hidden,
            area,
            pos,
            GlobalZIndex(1000),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Px(0.0),
                min_width: Val::Px(0.0),
                height: Val::Px(h * rows as f32 + 2.0),
                flex_direction: FlexDirection::Column,
                overflow: Overflow::scroll_y(),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(theme.panel),
            BorderColor::all(theme.accent2),
        ))
        .id();
    for (i, s) in d.options.iter().enumerate() {
        commands.spawn((
            DropdownOption(i),
            WidgetState::Normal,
            Node {
                height: Val::Px(h),
                flex_shrink: 0.0,
                padding: UiRect::horizontal(Val::Px(theme.pad)),
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::NONE),
            ChildOf(popup),
            children![kit_text(theme, s.clone(), theme.text_size, theme.text)],
        ));
    }
}

/// The pointer resting on a row highlights it.
pub(super) fn hover_highlight(
    rows: Query<(&DropdownOption, &WidgetState, &ChildOf), Changed<WidgetState>>,
    popups: Query<&DropdownPopup>,
    mut dropdowns: Query<&mut Dropdown>,
) {
    for (DropdownOption(i), state, parent) in &rows {
        if *state != WidgetState::Hovered {
            continue;
        }
        if let Ok(popup) = popups.get(parent.parent())
            && let Ok(mut d) = dropdowns.get_mut(popup.owner)
            && d.highlight != *i
        {
            d.highlight = *i;
        }
    }
}

/// Fields and rows take their colours from the state, the highlight and the theme.
#[allow(clippy::type_complexity)]
pub(super) fn paint(
    theme: Res<Theme>,
    mut fields: Query<(&Dropdown, &WidgetState, &Children, &mut BackgroundColor, &mut BorderColor), Without<DropdownOption>>,
    mut labels: Query<&mut SpriteText, With<DropdownLabel>>,
    mut arrows: Query<&mut BorderColor, (With<DropdownArrow>, Without<Dropdown>)>,
    mut rows: Query<(&DropdownOption, &WidgetState, &ChildOf, &mut BackgroundColor), Without<Dropdown>>,
    popups: Query<&DropdownPopup>,
    owners: Query<&Dropdown>,
) {
    for (d, state, kids, mut bg, mut border) in &mut fields {
        let base = if d.open { WidgetState::Hovered } else { *state };
        let want = state_colour(&theme, base, None);
        if bg.0 != want {
            bg.0 = want;
        }
        let edge = if d.open { theme.accent } else { theme.accent2 };
        if border.top != edge {
            *border = BorderColor::all(edge);
        }
        let ink = if *state == WidgetState::Disabled { theme.text_off } else { theme.kit_text_on };
        for kid in kids.iter() {
            if let Ok(mut t) = labels.get_mut(kid)
                && t.color != ink
            {
                t.color = ink;
            }
            if let Ok(mut a) = arrows.get_mut(kid)
                && a.top != ink
            {
                *a = BorderColor::all(ink);
            }
        }
    }
    for (DropdownOption(i), state, parent, mut bg) in &mut rows {
        let Some(d) = popups.get(parent.parent()).ok().and_then(|p| owners.get(p.owner).ok()) else { continue };
        let want = row_colour(&theme, *state, d.highlight == *i, *i);
        if bg.0 != want {
            bg.0 = want;
        }
    }
}

/// Where a list goes, in logical pixels, from its field's physical rectangle
/// (`field_top`, `field_bottom`, `left`, `width`), the list's physical height, the
/// window's physical height and the physical-to-logical factor: below the field, above
/// it when it would leave the window and fits there. Returns `(left, top, width)`.
pub fn popup_place(left: f32, field_top: f32, field_bottom: f32, width: f32, popup_h: f32, win_h: f32, inverse_scale: f32) -> (f32, f32, f32) {
    let gap = 2.0 / inverse_scale;
    let above = field_bottom + gap + popup_h > win_h && field_top - gap - popup_h >= 0.0;
    let top = if above { field_top - gap - popup_h } else { field_bottom + gap };
    // The list's border sits on the field's edge, as the child version's -1 px did.
    (left * inverse_scale - 1.0, top * inverse_scale, width * inverse_scale + 2.0)
}

/// Keep every open list at its field: remove lists whose field is gone or closed,
/// and put the others below (or above, near the window's bottom) the field's
/// laid-out rectangle, as wide as the field.
#[allow(clippy::type_complexity)]
pub(super) fn place_popups(
    mut commands: Commands,
    window: Query<&Window, With<PrimaryWindow>>,
    mut popups: Query<(Entity, &DropdownPopup, &ComputedNode, &mut Node, &mut Visibility)>,
    fields: Query<(&Dropdown, &ComputedNode, &UiGlobalTransform), Without<DropdownPopup>>,
) {
    let win_h = window.single().map_or(f32::MAX, |w| w.physical_height() as f32);
    for (e, popup, own, mut node, mut vis) in &mut popups {
        let Ok((d, field, at)) = fields.get(popup.owner) else {
            commands.entity(e).despawn();
            continue;
        };
        if !d.open {
            commands.entity(e).despawn();
            continue;
        }
        // Not laid out yet (first frame, or the field is hidden): stay invisible.
        if field.size().x <= 0.0 || own.size().y <= 0.0 {
            if *vis != Visibility::Hidden {
                *vis = Visibility::Hidden;
            }
            continue;
        }
        let size = field.size();
        let (left, top, width) = popup_place(
            at.translation.x - size.x / 2.0,
            at.translation.y - size.y / 2.0,
            at.translation.y + size.y / 2.0,
            size.x,
            own.size().y,
            win_h,
            field.inverse_scale_factor(),
        );
        let (l, t, w) = (Val::Px(left), Val::Px(top), Val::Px(width));
        if node.left != l || node.top != t || node.width != w {
            node.left = l;
            node.top = t;
            node.width = w;
        }
        if *vis != Visibility::Inherited {
            *vis = Visibility::Inherited;
        }
    }
}
