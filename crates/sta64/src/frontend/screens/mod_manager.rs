//! STA64's mod manager: a compact catalogue and actions for the selected row.

use bevy::ecs::hierarchy::ChildSpawner;
use bevy::ecs::spawn::SpawnWith;
use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::ui::UiTargetCamera;

use super::Screen;
use super::view::{Act, Ctx, ModList, ScreenRoot, ScrollBox};
use crate::app::AppState;
use crate::ui::i18n::{tr, trf};
use crate::ui::theme::{
    List, ListItem, RowMark, Theme, WidgetState, button, disabled_button, kit_panel, kit_text,
};

const ROW_H: f32 = 62.0;

struct Row {
    name: String,
    detail: String,
    authors: String,
    parent: String,
    status: String,
    active: bool,
}

fn text(theme: &Theme, value: impl Into<String>, color: Color, size: f32) -> impl Bundle {
    kit_text(theme, value, size, color)
}

fn column(theme: &Theme, width: f32, first: &str, second: &str, color: Color) -> impl Bundle {
    (
        Pickable::IGNORE,
        Node {
            width: Val::Percent(width),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            overflow: Overflow::clip(),
            ..default()
        },
        children![
            text(theme, first.to_owned(), color, theme.small_size),
            text(theme, second.to_owned(), theme.text_dim, theme.micro_size)
        ],
    )
}

fn row(p: &mut ChildSpawner, theme: &Theme, index: usize, value: &Row, selected: bool) {
    let fill = if selected {
        theme.row_sel
    } else if index % 2 == 1 {
        theme.row_alt
    } else {
        Color::NONE
    };
    p.spawn((
        ListItem(index),
        WidgetState::Normal,
        BackgroundColor(fill),
        Node {
            width: Val::Percent(100.0),
            height: Val::Px(ROW_H),
            flex_shrink: 0.0,
            column_gap: Val::Px(theme.gap.max(10.0)),
            padding: UiRect::axes(Val::Px(theme.pad), Val::Px(4.0)),
            align_items: AlignItems::Center,
            ..default()
        },
    ))
    .with_children(|r| {
        r.spawn((
            RowMark,
            Pickable::IGNORE,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                width: Val::Px(theme.pad * 0.5),
                height: Val::Percent(70.0),
                border_radius: BorderRadius::px(0.0, theme.pad, theme.pad, 0.0),
                ..default()
            },
            BackgroundColor(if selected { theme.accent } else { Color::NONE }),
        ));
        r.spawn(column(theme, 40.0, &value.name, &value.detail, theme.text));
        r.spawn(column(
            theme,
            32.0,
            &value.authors,
            &value.parent,
            theme.text,
        ));
        r.spawn(column(
            theme,
            13.0,
            &tr(&value.status),
            "",
            match value.status.as_str() {
                "Available" => theme.good,
                "Unavailable" => theme.bad,
                _ => theme.warn,
            },
        ));
        r.spawn(column(
            theme,
            9.0,
            &if value.active {
                tr("ACTIVE")
            } else {
                String::new()
            },
            "",
            theme.accent,
        ));
    });
}

fn heading(p: &mut ChildSpawner, theme: &Theme) {
    p.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Px(theme.row_h),
            column_gap: Val::Px(theme.gap.max(10.0)),
            padding: UiRect::horizontal(Val::Px(theme.pad)),
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(theme.accent2),
    ))
    .with_children(|h| {
        for (width, name) in [
            (40.0, "MOD / VERSION"),
            (32.0, "AUTHORS / PARENT"),
            (13.0, "STATUS"),
            (9.0, "Active"),
        ] {
            h.spawn((
                Node {
                    width: Val::Percent(width),
                    overflow: Overflow::clip(),
                    ..default()
                },
                children![text(theme, tr(name), theme.kit_text_on, theme.small_size)],
            ));
        }
    });
}

pub(super) fn spawn(commands: &mut Commands, ctx: &Ctx, camera: Option<Entity>) {
    let theme = ctx.theme.clone();
    let panel_theme = theme.clone();
    let base_name = ctx
        .data
        .base_name
        .clone()
        .unwrap_or_else(|| tr("Base game"));
    let mut rows = vec![Row {
        name: base_name.clone(),
        detail: String::new(),
        authors: String::new(),
        parent: String::new(),
        status: "Available".to_owned(),
        active: ctx.active_mod.0.is_none(),
    }];
    rows.extend(ctx.data.mods.iter().map(|m| {
        let parent = if m.standalone {
            tr("Total conversion")
        } else if let Some(folder) = &m.parent {
            let name = ctx
                .data
                .mods
                .iter()
                .find(|candidate| candidate.folder.eq_ignore_ascii_case(folder))
                .map(|candidate| candidate.name.as_str())
                .unwrap_or(folder);
            trf("Requires: {0}", &[&name])
        } else {
            trf("Requires: {0}", &[&base_name])
        };
        Row {
            name: m.name.clone(),
            detail: match (m.version.as_deref(), m.asset_version) {
                (Some(version), Some(asset)) => format!("{version} ({asset})"),
                (Some(version), None) => version.to_owned(),
                (None, Some(asset)) => asset.to_string(),
                (None, None) => String::new(),
            },
            authors: m.authors.join(", "),
            parent,
            status: if m.blocked.is_some() {
                "Unavailable"
            } else if m.asset_version.is_some_and(|v| v < 40_000) {
                "Legacy assets"
            } else {
                "Available"
            }
            .to_owned(),
            active: ctx
                .active_mod
                .0
                .as_deref()
                .is_some_and(|folder| folder.eq_ignore_ascii_case(&m.folder)),
        }
    }));
    let picked = ctx.browse.mod_row.min(rows.len() - 1);
    let selected = picked.checked_sub(1).and_then(|i| ctx.data.mods.get(i));
    let blocked = selected.is_some_and(|m| m.blocked.is_some());
    let website = selected
        .and_then(|m| m.website.as_deref())
        .is_some_and(|site| super::mod_actions::website_url(site).is_some());
    let selected_note = selected
        .and_then(|m| m.blocked.as_ref())
        .map(|why| trf("Cannot launch: {0}", &[why]))
        .or_else(|| selected.map(|m| m.description.clone()))
        .unwrap_or_else(|| tr("Launch the original installation without a mod."));
    let no_intro = ctx.browse.mod_no_intro;
    let no_ai = ctx.browse.mod_no_ai;
    let scroll = ctx.scroll.offset;
    let count = rows.len();

    let mut root = commands.spawn((
        ScreenRoot(Screen::ModSelect),
        DespawnOnExit(AppState::FrontEnd),
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
    ));
    if let Some(camera) = camera {
        root.insert(UiTargetCamera(camera));
    }
    root.with_children(|root| {
        root.spawn((
            Node {
                width: Val::Percent(94.0),
                max_width: Val::Px(1320.0),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            children![kit_panel(
                &panel_theme,
                &tr("Mods"),
                (
                    Node {
                        width: Val::Percent(100.0),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(theme.gap),
                        ..default()
                    },
                    Children::spawn(SpawnWith(move |p: &mut ChildSpawner| {
                        heading(p, &theme);
                        p.spawn((
                            ScrollBox {
                                selected: Some(picked),
                                count,
                                row_h: ROW_H,
                            },
                            Node {
                                width: Val::Percent(100.0),
                                max_height: Val::Vh(49.0),
                                overflow: Overflow::scroll_y(),
                                ..default()
                            },
                            ScrollPosition(Vec2::new(0.0, scroll)),
                        ))
                        .with_children(|box_| {
                            box_.spawn((
                                List {
                                    selected: Some(picked),
                                },
                                ModList,
                                Node {
                                    width: Val::Percent(100.0),
                                    flex_direction: FlexDirection::Column,
                                    flex_shrink: 0.0,
                                    ..default()
                                },
                            ))
                            .with_children(|list| {
                                for (index, value) in rows.iter().enumerate() {
                                    row(list, &theme, index, value, index == picked);
                                }
                            });
                        });
                        p.spawn((
                            Node {
                                max_height: Val::Px(36.0),
                                overflow: Overflow::clip(),
                                ..default()
                            },
                            children![text(
                                &theme,
                                selected_note,
                                if blocked { theme.warn } else { theme.text_dim },
                                theme.small_size
                            )],
                        ));
                        p.spawn(Node {
                            width: Val::Percent(100.0),
                            flex_wrap: FlexWrap::Wrap,
                            column_gap: Val::Px(theme.gap),
                            row_gap: Val::Px(theme.gap),
                            ..default()
                        })
                        .with_children(|bar| {
                            bar.spawn((
                                button(
                                    &theme,
                                    &if no_intro {
                                        tr("Skip intro: On")
                                    } else {
                                        tr("Skip intro: Off")
                                    },
                                    180.0,
                                ),
                                Act::ToggleModNoIntro,
                            ));
                            bar.spawn((
                                button(
                                    &theme,
                                    &if no_ai {
                                        tr("Disable AI: On")
                                    } else {
                                        tr("Disable AI: Off")
                                    },
                                    180.0,
                                ),
                                Act::ToggleModNoAi,
                            ));
                            if blocked {
                                bar.spawn(disabled_button(&theme, &tr("Launch mod"), 180.0));
                            } else {
                                bar.spawn((button(&theme, &tr("Launch mod"), 180.0), Act::UseMod));
                            }
                            #[cfg(target_os = "linux")]
                            if blocked {
                                bar.spawn(disabled_button(&theme, &tr("Create shortcut"), 205.0));
                            } else {
                                bar.spawn((
                                    button(&theme, &tr("Create shortcut"), 205.0),
                                    Act::CreateModShortcut,
                                ));
                            }
                            if super::mod_actions::WEBSITE_SUPPORTED && website {
                                bar.spawn((
                                    button(&theme, &tr("Visit website"), 180.0),
                                    Act::VisitModWebsite,
                                ));
                            } else {
                                bar.spawn(disabled_button(&theme, &tr("Visit website"), 180.0));
                            }
                        });
                        p.spawn((
                            button(&theme, &tr("Previous menu"), 205.0),
                            Act::Goto(Screen::Main),
                        ));
                    })),
                )
            )],
        ));
    });
}
