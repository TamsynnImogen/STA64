use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;

use super::{AuthoredIds, Document, Editing};
use crate::sim_link::{ContentRes, Selection, SimLink};
use crate::ui::sprite_font::SpriteText;
use crate::ui::theme::{
    FocusScope, ScrollArea, TextFilter, Theme, kit_button, kit_text, text_field,
};

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
const INSPECTOR_ROW_H: f32 = 40.0;
const INSPECTOR_VISIBLE_ROWS: usize = 12;

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
    let same_inspector = modal && roots.iter().any(|(_, root)| {
        root.0 == editing.inspected && root.1 == editing.map_settings
    });
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
            width: Val::Px(if modal {
                INSPECTOR_WIDTH
            } else {
                WIDTH
            }),
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
        let width = if modal {
            INSPECTOR_WIDTH
        } else {
            WIDTH
        } - 20.0;
        if editing.map_settings {
            if let Some(doc) = &document {
                ui.spawn(kit_text(&theme, "Map properties", theme.title_size, theme.text));
                super::map_settings::draw(ui, &theme, doc, width);
                ui.spawn((InspectorError, kit_text(&theme, &editing.inspector_error, theme.small_size, theme.text)));
                ui.spawn((kit_button(&button_theme, "Apply", width), Act::ApplyMap));
                ui.spawn((kit_button(&button_theme, "Cancel", width), Act::CloseStats));
            }
            return;
        }
        if let Some(id) = editing.inspected {
            ui.spawn(kit_text(
                &theme,
                "Object inspector",
                theme.title_size,
                theme.text,
            ));
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
            let sections = rows
                .iter()
                .enumerate()
                .filter(|(i, row)| *i == 0 || rows[*i - 1].section != row.section)
                .count();
            let item_count = rows.len() + sections;
            let visible = item_count.max(1).min(INSPECTOR_VISIBLE_ROWS);
            ui.spawn((
                ScrollArea {
                    row_h: INSPECTOR_ROW_H,
                    rows: visible,
                    count: item_count,
                },
                ScrollPosition::default(),
                Node {
                    width: Val::Px(width),
                    height: Val::Px(INSPECTOR_ROW_H * visible as f32),
                    flex_direction: FlexDirection::Column,
                    flex_shrink: 0.0,
                    overflow: Overflow::scroll_y(),
                    ..default()
                },
                BackgroundColor(theme.track),
            ))
            .with_children(|list| {
                if rows.is_empty() {
                    list.spawn(kit_text(
                        &theme,
                        "Object no longer available",
                        theme.small_size,
                        theme.text,
                    ));
                }
                let mut section = None;
                for row in rows {
                    if section != Some(row.section) {
                        section = Some(row.section);
                        list.spawn(Node {
                                height: Val::Px(INSPECTOR_ROW_H),
                                flex_shrink: 0.0,
                                padding: UiRect::horizontal(Val::Px(theme.pad)),
                                align_items: AlignItems::Center,
                                ..default()
                            }).with_child(kit_text(&theme, row.section, theme.text_size, theme.title));
                    }
                    list.spawn((Node {
                        height: Val::Px(INSPECTOR_ROW_H),
                        flex_shrink: 0.0,
                        padding: UiRect::horizontal(Val::Px(theme.pad)),
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(4.0),
                        ..default()
                    },))
                        .with_children(|line| {
                            line.spawn(Node {
                                    width: Val::Px(136.0),
                                    flex_shrink: 0.0,
                                    overflow: Overflow::clip(),
                                    ..default()
                                }).with_child(kit_text(&theme, row.label, theme.small_size, theme.text_dim));
                            if let Some(key) = row.key {
                                line.spawn((
                                    text_field(
                                        &button_theme,
                                        &row.value,
                                        "",
                                        width - 160.0,
                                        if matches!(key, "captain" | "registry") { 511 } else { 128 },
                                        TextFilter::Any,
                                    ),
                                    InspectorField(key),
                                ));
                            } else {
                                line.spawn(Node {
                                        flex_grow: 1.0,
                                        overflow: Overflow::clip(),
                                        ..default()
                                    }).with_child(kit_text(&theme, row.value, theme.small_size, theme.text));
                            }
                        });
                }
            });
            ui.spawn((
                InspectorError,
                kit_text(
                    &theme,
                    &editing.inspector_error,
                    theme.small_size,
                    theme.text,
                ),
            ));
            ui.spawn((kit_button(&button_theme, "Apply", width), Act::ApplyStats));
            ui.spawn((
                kit_button(&button_theme, "Restore", width),
                Act::RestoreStats,
            ));
            ui.spawn((kit_button(&button_theme, "Cancel", width), Act::CloseStats));
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
        if document.as_ref().is_some_and(|doc| doc.can_change_objects()) {
            ui.spawn((kit_button(&button_theme, "Map properties", width), Act::MapSettings));
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
