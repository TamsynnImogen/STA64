use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;

use super::{AuthoredIds, Document, Editing};
use crate::sim_link::{ContentRes, Selection, SimLink};
use crate::ui::theme::{Theme, kit_button, kit_text};

#[derive(Component)]
pub(super) struct Root;

#[derive(Component)]
pub(crate) enum Act {
    Play,
    Save,
    Exit,
    Rotate,
    Delete,
    Entry(usize),
    Back,
    CloseStats,
}

const WIDTH: f32 = 262.0;

#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    mut commands: Commands,
    editing: Option<Res<Editing>>,
    state: Res<State<crate::app::AppState>>,
    document: Option<Res<Document>>,
    authored: Option<Res<AuthoredIds>>,
    selection: Res<Selection>,
    link: Option<Res<SimLink>>,
    content: Option<Res<ContentRes>>,
    theme: Res<Theme>,
    roots: Query<Entity, With<Root>>,
) {
    let Some(editing) = editing.filter(|_| *state.get() == crate::app::AppState::InGame) else {
        for root in &roots {
            commands.entity(root).despawn();
        }
        return;
    };
    if !editing.is_changed()
        && !selection.is_changed()
        && !theme.is_changed()
        && !document.as_ref().is_some_and(|d| d.is_changed())
        && !authored.as_ref().is_some_and(|ids| ids.is_changed())
        && !(editing.inspected.is_some() && link.as_ref().is_some_and(|l| l.is_changed()))
        && !roots.is_empty()
    {
        return;
    }
    for root in &roots {
        commands.entity(root).despawn();
    }
    let selected = selection
        .0
        .first()
        .and_then(|id| link.as_ref()?.curr().unit(*id));
    let can_change_objects = document.as_ref().is_none_or(|d| d.can_change_objects());
    let entries = editing.entries();
    // Keep the full twelve-slot list visible alongside the editor tools.
    let mut button_theme = theme.as_ref().clone();
    button_theme.button_h = button_theme.button_h.min(26.0);

    commands
        .spawn((
            Root,
            RelativeCursorPosition::default(),
            GlobalZIndex(21),
            BackgroundColor(theme.panel.with_alpha(0.94)),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(14.0),
                left: Val::Px(14.0),
                width: Val::Px(WIDTH),
                max_height: Val::Percent(96.0),
                overflow: Overflow::clip(),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(3.0),
                padding: UiRect::all(Val::Px(10.0)),
                ..default()
            },
        ))
        .with_children(|ui| {
            let width = WIDTH - 20.0;
            if let Some(id) = editing.inspected {
                ui.spawn(kit_text(&theme, "Object stats", theme.title_size, theme.text));
                ui.spawn((kit_button(&button_theme, "Close  ·  Tab", width), Act::CloseStats));
                let object = authored.as_ref().and_then(|ids| ids.0.get(&id))
                    .and_then(|index| document.as_ref()?.object(*index));
                let stats = link.as_ref().and_then(|link| {
                    stats_rows(id, link, content.as_deref(), object)
                });
                for row in stats.unwrap_or_else(|| vec!["Object no longer available".into()]) {
                    ui.spawn(kit_text(&theme, row, theme.small_size, theme.text));
                }
                return;
            }
            ui.spawn(kit_text(&theme, "Map editor", theme.title_size, theme.text));
            if let Some(doc) = &document {
                let name = super::map_stem(&doc.path).unwrap_or(&doc.path);
                ui.spawn(kit_text(&theme, name, theme.small_size, theme.text));
            }
            ui.spawn((kit_button(&button_theme, "Play  ·  Ctrl+E", width), Act::Play));
            if document.is_some() {
                ui.spawn((kit_button(&button_theme, "Save copy  ·  Ctrl+S", width), Act::Save));
            }
            ui.spawn((kit_button(&button_theme, "Return to main menu", width), Act::Exit));
            if let Some(unit) = selected && let Some(content) = &content {
                if let Some(name) = content.0.name(unit.def) {
                    ui.spawn(kit_text(&theme, name, theme.text_size, theme.text));
                }
                if super::editable(&content.0, unit.def) {
                    ui.spawn((kit_button(&button_theme, "Rotate  ·  [ / ]", width), Act::Rotate));
                    if can_change_objects {
                        ui.spawn((kit_button(&button_theme, "Delete", width), Act::Delete));
                    }
                }
            }
            let hint = if can_change_objects {
                "F keys open menus or place"
            } else {
                "Move or rotate existing ships"
            };
            ui.spawn(kit_text(&theme, hint, theme.small_size, theme.text));
            ui.spawn(kit_text(&theme, "Double-click an object for stats", theme.small_size, theme.text));
            if !editing.status.is_empty() {
                ui.spawn(kit_text(
                    &theme,
                    &editing.status,
                    theme.small_size,
                    theme.text,
                ));
            }
            if !can_change_objects {
                return;
            }
            if !editing.directory.is_empty() {
                ui.spawn((kit_button(&button_theme, "Back  ·  Tab", width), Act::Back));
            }
            for (i, entry) in entries.iter().enumerate() {
                let label = match entry.kind {
                    super::EntryKind::Branch(_) => {
                        format!("F{}  {}  ›", entry.slot + 1, entry.label)
                    }
                    super::EntryKind::Leaf { .. } => {
                        format!("F{}  {}", entry.slot + 1, entry.label)
                    }
                };
                ui.spawn((kit_button(&button_theme, &label, width), Act::Entry(i)));
            }
        });
}

/// Show the map's identity fields alongside the paused world's current values.
fn stats_rows(
    id: sta_sim::EntityId,
    link: &SimLink,
    content: Option<&ContentRes>,
    object: Option<&sta_bzn::MapObject>,
) -> Option<Vec<String>> {
    let unit = link.real_curr().unit(id)?;
    let def = content.and_then(|content| content.0.unit(unit.def));
    let odf = object.map(|object| object.odf.as_str())
        .or_else(|| def.map(|def| def.odf.as_str())).unwrap_or("Unknown");
    let mut rows = vec![format!("ODF  ·  {odf}"), format!("Entity  ·  {}", id.0)];
    if let Some(def) = def {
        rows.push(format!("Class  ·  {}", def.class.0));
    }
    if let Some(object) = object {
        for (field, value) in [("Label", &object.label), ("Name", &object.instance_name)] {
            if !value.is_empty() {
                rows.push(format!("{field}  ·  {value}"));
            }
        }
    }
    if unit.owner == sta_sim::world::NEUTRAL {
        rows.push("Team  ·  Neutral".into());
    } else {
        rows.push(format!("Team  ·  {}", u16::from(unit.owner.0) + 1));
    }
    let [x, y, z] = unit.pos.to_f32();
    let heading = unit.heading.0.to_f32().atan2(unit.heading.1.to_f32()).to_degrees().rem_euclid(360.0);
    rows.push(format!("Position  ·  {x:.1}, {y:.1}, {z:.1}"));
    rows.push(format!("Heading  ·  {heading:.0}°"));
    let pair = |value: [sta_sim::Fixed; 2]| {
        format!("{:.0} / {:.0}", value[0].to_f32(), value[1].to_f32())
    };
    for (name, value) in [("Hull", Some(unit.hull)), ("Crew", unit.crew), ("Energy", unit.energy)] {
        if let Some(value) = value.filter(|value| value[1] > sta_sim::Fixed::ZERO) {
            rows.push(format!("{name}  ·  {}", pair(value)));
        }
    }
    let [fore, aft, port, starboard] = unit.shields;
    if fore == aft && fore == port && fore == starboard {
        if fore[1] > sta_sim::Fixed::ZERO {
            rows.push(format!("Shields  ·  {}", pair(fore)));
        }
    } else {
        for (name, value) in [("Fore", fore), ("Aft", aft), ("Port", port), ("Starboard", starboard)] {
            if value[1] > sta_sim::Fixed::ZERO {
                rows.push(format!("{name} shield  ·  {}", pair(value)));
            }
        }
    }
    if let Some(node) = link.nodes().iter().find(|node| node.entity == id) {
        let amount = if node.infinite { "Unlimited".into() } else { pair([node.amount, node.start]) };
        rows.push(format!("Resources  ·  {amount}"));
    }
    Some(rows)
}
