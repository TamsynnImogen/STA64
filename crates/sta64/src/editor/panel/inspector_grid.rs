//! Mounted object-inspector grid. Visibility changes leave field entities and drafts intact.

use std::collections::BTreeSet;

use bevy::prelude::*;

use super::{Act, InspectorField};
use crate::ui::sprite_font::SpriteText;
use crate::ui::theme::{
    ButtonClicked, DropdownChanged, ScrollArea, TextFilter, Theme, checkbox, dropdown, kit_button,
    kit_text, text_field,
};

use super::super::inspector::{Control, Row};

const ROW_H: f32 = 32.0;
const MAX_ROWS: usize = 14;
const LABEL_W: f32 = 146.0;
const TEAM_OPTIONS: [&str; 9] = [
    "Neutral", "Team 1", "Team 2", "Team 3", "Team 4", "Team 5", "Team 6", "Team 7", "Team 8",
];

#[derive(Component)]
pub(in crate::editor) struct ViewChoice;

#[derive(Component)]
pub(in crate::editor) struct SectionToggle(&'static str);

#[derive(Component)]
pub(in crate::editor) struct GroupToggle(&'static str, &'static str);

#[derive(Clone, Copy)]
enum LineKind {
    Section,
    Group,
    Field,
}

#[derive(Component)]
pub(in crate::editor) struct Line {
    section: &'static str,
    group: Option<&'static str>,
    advanced: bool,
    kind: LineKind,
}

#[derive(Component)]
pub(in crate::editor) struct Grid {
    advanced: bool,
    closed_sections: BTreeSet<&'static str>,
    closed_groups: BTreeSet<(&'static str, &'static str)>,
}

fn any_basic(rows: &[Row], section: &'static str, group: Option<&'static str>) -> bool {
    rows.iter().any(|row| {
        row.section == section && (group.is_none() || row.group == group) && !row.advanced
    })
}

fn initially_closed(section: &str, group: &str) -> bool {
    (section == "GameObject" && group == "Carrier") || section == "CraftSystem"
}

pub(super) fn draw(
    ui: &mut ChildSpawnerCommands,
    theme: &Theme,
    controls: &Theme,
    width: f32,
    breadcrumb: &str,
    rows: &[Row],
) {
    ui.spawn(Node {
        width: Val::Px(width),
        align_items: AlignItems::Center,
        column_gap: Val::Px(8.0),
        ..default()
    })
    .with_children(|header| {
        header.spawn(kit_text(theme, "View", theme.small_size, theme.text_dim));
        header.spawn((
            dropdown(controls, &["Basic", "Advanced"], 1, 140.0),
            ViewChoice,
        ));
        header.spawn(Node {
            flex_grow: 1.0,
            ..default()
        });
        header.spawn((
            kit_button(controls, "Restore Class", 142.0),
            Act::RestoreStats,
        ));
    });
    ui.spawn((
        Node {
            width: Val::Px(width),
            height: Val::Px(ROW_H),
            align_items: AlignItems::Center,
            padding: UiRect::horizontal(Val::Px(theme.pad)),
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(theme.row_alt),
    ))
    .with_child(kit_text(theme, breadcrumb, theme.small_size, theme.title));

    let section_count = rows
        .iter()
        .map(|row| row.section)
        .collect::<BTreeSet<_>>()
        .len();
    let group_count = rows
        .iter()
        .filter_map(|row| row.group.map(|group| (row.section, group)))
        .collect::<BTreeSet<_>>()
        .len();
    let closed_groups: BTreeSet<_> = rows
        .iter()
        .filter_map(|row| {
            let group = row.group?;
            initially_closed(row.section, group).then_some((row.section, group))
        })
        .collect();
    let visible_fields = rows
        .iter()
        .filter(|row| {
            row.group
                .is_none_or(|group| !closed_groups.contains(&(row.section, group)))
        })
        .count();
    let count = (visible_fields + section_count + group_count).max(1);
    let shown = count.min(MAX_ROWS);
    ui.spawn((
        Grid {
            advanced: true,
            closed_sections: BTreeSet::new(),
            closed_groups: closed_groups.clone(),
        },
        ScrollArea {
            row_h: ROW_H,
            rows: shown,
            count,
        },
        ScrollPosition::default(),
        Node {
            width: Val::Px(width),
            height: Val::Px(ROW_H * shown as f32),
            min_height: Val::Px(ROW_H),
            flex_direction: FlexDirection::Column,
            flex_shrink: 1.0,
            overflow: Overflow::scroll_y(),
            ..default()
        },
        BackgroundColor(theme.track),
    ))
    .with_children(|list| {
        if rows.is_empty() {
            list.spawn(Node {
                height: Val::Px(ROW_H),
                flex_shrink: 0.0,
                ..default()
            })
            .with_child(kit_text(
                theme,
                "Object no longer available",
                theme.small_size,
                theme.text,
            ));
        }
        let mut section = None;
        let mut group = None;
        for row in rows {
            if section != Some(row.section) {
                section = Some(row.section);
                group = None;
                let basic_only = !any_basic(rows, row.section, None);
                list.spawn((
                    Line {
                        section: row.section,
                        group: None,
                        advanced: basic_only,
                        kind: LineKind::Section,
                    },
                    Node {
                        height: Val::Px(ROW_H),
                        flex_shrink: 0.0,
                        ..default()
                    },
                ))
                .with_child((
                    kit_button(controls, &format!("-  {}", row.section), width),
                    SectionToggle(row.section),
                ));
            }
            if group != row.group {
                group = row.group;
                if let Some(name) = group {
                    let basic_only = !any_basic(rows, row.section, Some(name));
                    list.spawn((
                        Line {
                            section: row.section,
                            group: Some(name),
                            advanced: basic_only,
                            kind: LineKind::Group,
                        },
                        Node {
                            height: Val::Px(ROW_H),
                            flex_shrink: 0.0,
                            ..default()
                        },
                    ))
                    .with_child((
                        kit_button(
                            controls,
                            &format!(
                                "    {}  {name}",
                                if closed_groups.contains(&(row.section, name)) {
                                    "+"
                                } else {
                                    "-"
                                }
                            ),
                            width,
                        ),
                        GroupToggle(row.section, name),
                    ));
                }
            }
            list.spawn((
                Line {
                    section: row.section,
                    group: row.group,
                    advanced: row.advanced,
                    kind: LineKind::Field,
                },
                Node {
                    width: Val::Px(width),
                    height: Val::Px(ROW_H),
                    display: if row
                        .group
                        .is_some_and(|group| closed_groups.contains(&(row.section, group)))
                    {
                        Display::None
                    } else {
                        Display::Flex
                    },
                    flex_shrink: 0.0,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(6.0),
                    padding: UiRect::horizontal(Val::Px(theme.pad)),
                    ..default()
                },
                BackgroundColor(theme.row_alt),
            ))
            .with_children(|line| {
                line.spawn(Node {
                    width: Val::Px(LABEL_W),
                    flex_shrink: 0.0,
                    overflow: Overflow::clip(),
                    ..default()
                })
                .with_child(kit_text(
                    theme,
                    row.label,
                    theme.small_size,
                    if row.key.is_some() {
                        theme.text
                    } else {
                        theme.text_dim
                    },
                ));
                let value_w = width - LABEL_W - 2.0 * theme.pad - 6.0;
                if let Some(key) = row.key {
                    match &row.control {
                        Control::Text => {
                            line.spawn((
                                text_field(
                                    controls,
                                    &row.value,
                                    "",
                                    value_w,
                                    if matches!(key, "captain" | "registry") {
                                        511
                                    } else {
                                        128
                                    },
                                    TextFilter::Any,
                                ),
                                InspectorField(key),
                            ));
                        }
                        Control::Check => {
                            line.spawn((
                                checkbox(controls, "", row.value == "1"),
                                InspectorField(key),
                            ));
                        }
                        Control::Team => {
                            let selected = row.value.parse::<usize>().unwrap_or(0).min(8);
                            line.spawn((
                                dropdown(controls, &TEAM_OPTIONS, selected, value_w),
                                InspectorField(key),
                            ));
                        }
                        Control::Race(options) => {
                            let mut choices: Vec<&str> =
                                options.iter().map(String::as_str).collect();
                            let selected = if let Some(index) = choices
                                .iter()
                                .position(|choice| *choice == row.value.as_str())
                            {
                                index
                            } else {
                                choices.push(&row.value);
                                choices.len() - 1
                            };
                            line.spawn((
                                dropdown(controls, &choices, selected, value_w),
                                InspectorField(key),
                            ));
                        }
                    }
                } else {
                    let shown = if matches!(&row.control, Control::Team) {
                        row.value
                            .parse::<usize>()
                            .ok()
                            .and_then(|index| TEAM_OPTIONS.get(index))
                            .copied()
                            .unwrap_or(&row.value)
                    } else {
                        &row.value
                    };
                    line.spawn(Node {
                        flex_grow: 1.0,
                        overflow: Overflow::clip(),
                        ..default()
                    })
                    .with_child(kit_text(
                        theme,
                        shown,
                        theme.small_size,
                        theme.text_dim,
                    ));
                }
            });
        }
    });
}

/// The view and fold controls change only visibility, retaining every mounted draft.
pub(in crate::editor) fn inspector_grid_update(
    (mut buttons, mut dropdowns): (MessageReader<ButtonClicked>, MessageReader<DropdownChanged>),
    sections: Query<(&SectionToggle, &Children)>,
    groups: Query<(&GroupToggle, &Children)>,
    views: Query<(), With<ViewChoice>>,
    mut grid: Query<(&mut Grid, &mut ScrollArea, &mut Node, &mut ScrollPosition)>,
    mut lines: Query<(&Line, &mut Node), Without<Grid>>,
    mut captions: Query<&mut SpriteText>,
) {
    let Ok((mut state, mut area, mut scroll_node, mut scroll)) = grid.single_mut() else {
        return;
    };
    let mut changed = false;
    for ButtonClicked(entity) in buttons.read() {
        if let Ok((toggle, _)) = sections.get(*entity) {
            if !state.closed_sections.insert(toggle.0) {
                state.closed_sections.remove(toggle.0);
            }
            changed = true;
        } else if let Ok((toggle, _)) = groups.get(*entity) {
            let key = (toggle.0, toggle.1);
            if !state.closed_groups.insert(key) {
                state.closed_groups.remove(&key);
            }
            changed = true;
        }
    }
    for event in dropdowns.read() {
        if views.get(event.entity).is_ok() {
            state.advanced = event.index != 0;
            changed = true;
        }
    }
    if !changed {
        return;
    }
    for (toggle, children) in &sections {
        if let Some(label) = children.first()
            && let Ok(mut text) = captions.get_mut(*label)
        {
            let arrow = if state.closed_sections.contains(toggle.0) {
                "+"
            } else {
                "-"
            };
            text.text = format!("{arrow}  {}", toggle.0);
        }
    }
    for (toggle, children) in &groups {
        if let Some(label) = children.first()
            && let Ok(mut text) = captions.get_mut(*label)
        {
            let arrow = if state.closed_groups.contains(&(toggle.0, toggle.1)) {
                "+"
            } else {
                "-"
            };
            text.text = format!("    {arrow}  {}", toggle.1);
        }
    }
    let mut count = 0;
    for (line, mut node) in &mut lines {
        let visible = (state.advanced || !line.advanced)
            && match line.kind {
                LineKind::Section => true,
                LineKind::Group => !state.closed_sections.contains(line.section),
                LineKind::Field => {
                    !state.closed_sections.contains(line.section)
                        && line.group.is_none_or(|group| {
                            !state.closed_groups.contains(&(line.section, group))
                        })
                }
            };
        node.display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        count += usize::from(visible);
    }
    area.count = count.max(1);
    area.rows = area.count.min(MAX_ROWS);
    scroll_node.height = Val::Px(ROW_H * area.rows as f32);
    scroll.0.y = scroll.0.y.clamp(0.0, area.max_offset());
}
