use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;

use super::{Document, Editing};
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
    Page(usize),
}

const WIDTH: f32 = 262.0;
const PAGE_SIZE: usize = 12;

#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    mut commands: Commands,
    editing: Option<Res<Editing>>,
    state: Res<State<crate::app::AppState>>,
    document: Option<Res<Document>>,
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
    if !editing.is_changed() && !selection.is_changed() && !theme.is_changed() && !roots.is_empty() {
        return;
    }
    for root in &roots {
        commands.entity(root).despawn();
    }
    let selected = selection.0.first().and_then(|id| link.as_ref()?.curr().unit(*id))
        .and_then(|unit| content.as_ref()?.0.name(unit.def));
    let can_change_objects = document.as_ref().is_none_or(|d| d.can_change_objects());
    let entries = editing.entries();
    let offset = editing.page * PAGE_SIZE;

    commands.spawn((
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
            row_gap: Val::Px(4.0),
            padding: UiRect::all(Val::Px(10.0)),
            ..default()
        },
    )).with_children(|ui| {
        let width = WIDTH - 20.0;
        ui.spawn(kit_text(&theme, "Map editor", theme.title_size, theme.text));
        if let Some(doc) = &document {
            let name = super::map_stem(&doc.path).unwrap_or(&doc.path);
            ui.spawn(kit_text(&theme, name, theme.small_size, theme.text));
        }
        ui.spawn((kit_button(&theme, "Play  ·  Ctrl+E", width), Act::Play));
        if document.is_some() {
            ui.spawn((kit_button(&theme, "Save copy  ·  Ctrl+S", width), Act::Save));
        }
        ui.spawn((kit_button(&theme, "Return to main menu", width), Act::Exit));
        if let Some(selected) = selected {
            ui.spawn(kit_text(&theme, selected, theme.text_size, theme.text));
            ui.spawn((kit_button(&theme, "Rotate  ·  [ / ]", width), Act::Rotate));
            if can_change_objects {
                ui.spawn((kit_button(&theme, "Delete", width), Act::Delete));
            }
        }
        let hint = if let Some((odf, _)) = &editing.placing {
            format!("Place {odf} with left click. Right click cancels.")
        } else if !can_change_objects {
            "Existing placements can be moved and rotated.".into()
        } else {
            "Select a ship or station. Drag or right click to move it.".into()
        };
        ui.spawn(kit_text(&theme, hint, theme.small_size, theme.text));
        if !editing.status.is_empty() {
            ui.spawn(kit_text(&theme, &editing.status, theme.small_size, theme.text));
        }
        if !can_change_objects {
            return;
        }
        for (i, entry) in entries.iter().enumerate().skip(offset).take(PAGE_SIZE) {
            let label = match entry.kind {
                super::EntryKind::Branch(_) => format!("{}  ›", entry.label),
                super::EntryKind::Leaf { .. } => entry.label.clone(),
            };
            ui.spawn((kit_button(&theme, &label, width), Act::Entry(i)));
        }
        if editing.page > 0 {
            ui.spawn((kit_button(&theme, "Previous page", width), Act::Page(editing.page - 1)));
        }
        if offset + PAGE_SIZE < entries.len() {
            ui.spawn((kit_button(&theme, "Next page", width), Act::Page(editing.page + 1)));
        }
        if !editing.directory.is_empty() {
            ui.spawn((kit_button(&theme, "Back", width), Act::Back));
        }
    });
}
