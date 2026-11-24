use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;

use super::{AuthoredIds, Document, Editing};
use crate::sim_link::{ContentRes, Selection, SimLink};
use crate::ui::sprite_font::SpriteText;
use crate::ui::theme::{FocusScope, Theme, kit_button, kit_text};

mod inspector_grid;
pub(super) use inspector_grid::inspector_grid_update;

#[derive(Component)]
pub(super) struct Root(Option<sta_sim::EntityId>, bool);

#[derive(Component)]
pub(super) struct InspectorField(pub &'static str);

#[derive(Component)]
pub(super) struct InspectorError;

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
    ApplyStats,
    RestoreStats,
    MapSettings,
    ApplyMap,
}

const WIDTH: f32 = 262.0;
const INSPECTOR_WIDTH: f32 = 446.0;

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
    roots: Query<(Entity, &Root)>,
) {
    let Some(editing) = editing.filter(|_| *state.get() == crate::app::AppState::InGame) else {
        for (root, _) in &roots {
            commands.entity(root).despawn();
        }
        return;
    };
    let modal = editing.inspected.is_some() || editing.map_settings;
    let same_inspector = modal
        && roots
            .iter()
            .any(|(_, root)| root.0 == editing.inspected && root.1 == editing.map_settings);
    if (same_inspector
        || (!editing.is_changed()
            && !selection.is_changed()
            && !document.as_ref().is_some_and(|d| d.is_changed())
            && !authored.as_ref().is_some_and(|ids| ids.is_changed())
            && !(editing.inspected.is_some() && link.as_ref().is_some_and(|l| l.is_changed()))))
        && !theme.is_changed()
        && !roots.is_empty()
    {
        return;
    }
    for (root, _) in &roots {
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

    let mut root = commands.spawn((
        Root(editing.inspected, editing.map_settings),
        RelativeCursorPosition::default(),
        GlobalZIndex(21),
        BackgroundColor(theme.panel.with_alpha(0.94)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(14.0),
            left: Val::Px(14.0),
            width: Val::Px(if modal { INSPECTOR_WIDTH } else { WIDTH }),
            max_height: Val::Percent(96.0),
            overflow: Overflow::clip(),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(3.0),
            padding: UiRect::all(Val::Px(10.0)),
            ..default()
        },
    ));
    if modal {
        root.insert(FocusScope);
    }
    root.with_children(|ui| {
        let width = if modal { INSPECTOR_WIDTH } else { WIDTH } - 20.0;
        if editing.map_settings {
            if let Some(doc) = &document {
                ui.spawn(kit_text(
                    &theme,
                    "Map properties",
                    theme.title_size,
                    theme.text,
                ));
                super::map_settings::draw(ui, &theme, doc, width);
                ui.spawn((
                    InspectorError,
                    kit_text(
                        &theme,
                        &editing.inspector_error,
                        theme.small_size,
                        theme.text,
                    ),
                ));
                ui.spawn((kit_button(&button_theme, "Apply", width), Act::ApplyMap));
                ui.spawn((kit_button(&button_theme, "Cancel", width), Act::CloseStats));
            }
            return;
        }
        if let Some(id) = editing.inspected {
            let object = authored
                .as_ref()
                .and_then(|ids| ids.0.get(&id))
                .and_then(|index| document.as_ref()?.object(*index));
            let extras = object.and_then(|object| document.as_ref()?.properties(object.entity_id));
            let rows = link
                .as_ref()
                .map(|link| {
                    super::inspector::rows(
                        id,
                        link,
                        content.as_deref(),
                        object,
                        extras,
                        can_change_objects,
                    )
                })
                .unwrap_or_default();
            let breadcrumb = link.as_ref().map_or_else(String::new, |link| {
                super::inspector::class_chain(id, link, content.as_deref())
            });
            inspector_grid::draw(ui, &theme, &button_theme, width, &breadcrumb, &rows);
            ui.spawn((
                InspectorError,
                kit_text(
                    &theme,
                    &editing.inspector_error,
                    theme.small_size,
                    theme.text,
                ),
            ));
            ui.spawn(Node {
                width: Val::Px(width),
                column_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|footer| {
                footer.spawn((
                    kit_button(&button_theme, "OK", (width - 8.0) / 2.0),
                    Act::ApplyStats,
                ));
                footer.spawn((
                    kit_button(&button_theme, "Cancel", (width - 8.0) / 2.0),
                    Act::CloseStats,
                ));
            });
            return;
        }
        ui.spawn(kit_text(&theme, "Map editor", theme.title_size, theme.text));
        if let Some(doc) = &document {
            let name = super::map_stem(&doc.path).unwrap_or(&doc.path);
            ui.spawn(kit_text(&theme, name, theme.small_size, theme.text));
        }
        ui.spawn((
            kit_button(&button_theme, "Play  ·  Ctrl+E", width),
            Act::Play,
        ));
        if document.is_some() {
            ui.spawn((
                kit_button(&button_theme, "Save copy  ·  Ctrl+S", width),
                Act::Save,
            ));
        }
        if document
            .as_ref()
            .is_some_and(|doc| doc.can_change_objects())
        {
            ui.spawn((
                kit_button(&button_theme, "Map properties", width),
                Act::MapSettings,
            ));
        }
        ui.spawn((
            kit_button(&button_theme, "Return to main menu", width),
            Act::Exit,
        ));
        if let Some(unit) = selected
            && let Some(content) = &content
        {
            if let Some(name) = content.0.name(unit.def) {
                ui.spawn(kit_text(&theme, name, theme.text_size, theme.text));
            }
            if super::editable(&content.0, unit.def) {
                ui.spawn((
                    kit_button(&button_theme, "Rotate  ·  [ / ]", width),
                    Act::Rotate,
                ));
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
        ui.spawn(kit_text(
            &theme,
            "Double-click an object to inspect",
            theme.small_size,
            theme.text,
        ));
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

/// Update validation feedback without replacing text fields and their draft values.
pub(super) fn inspector_error(
    editing: Option<Res<Editing>>,
    mut errors: Query<&mut SpriteText, With<InspectorError>>,
) {
    let Some(editing) = editing else { return };
    for mut text in &mut errors {
        if text.text != editing.inspector_error {
            text.text = editing.inspector_error.clone();
        }
    }
}
