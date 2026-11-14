//! The HUD's panels as Bevy UI nodes on the LCARS kit: the resources strip,
//! the info panel and the command grid. Value changes patch existing children;
//! structural changes rebuild a panel, and layout changes reposition it.

use bevy::picking::Pickable;
use bevy::prelude::*;
use sta_sim::EntityId;

use super::layout::{CELL, CELL_GAP, GRID_COLS, GRID_PAD, GRID_ROWS, HudLayout, InfoHeight};
use super::model::{Act, Button, Info, Look, ResEntry, Single, Tile};
use crate::ui::gui_sprites::{GuiButton, GuiButtonLabel};
use crate::ui::i18n::{tr, trf};
use crate::ui::notices::Tooltip;
use crate::ui::theme::{Theme, UiButton, WidgetState, progress_bar};

/// The one root every HUD node of ours hangs on.
#[derive(Component)]
pub struct HudRoot;
#[derive(Component)]
pub(super) struct HudBody;
#[derive(Component)]
pub struct ResourceStrip;
#[derive(Component)]
pub struct InfoPanel;
#[derive(Component)]
pub struct GridPanel;

#[derive(Component, Clone, Copy)]
pub(super) enum ValueText { Resource(usize), Hull, Shield, Built }

#[derive(Component, Clone, Copy)]
pub(super) enum ValueBar { Hull, Shield, Built, Group(EntityId) }

/// Numbers do not change the row's structure.
pub(super) fn same_resources(a: &[ResEntry], b: &[ResEntry]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a,b)| a.id == b.id && a.name == b.name)
}

pub(super) fn same_info(a: &Info, b: &Info) -> bool {
    match (a,b) {
        (Info::None, Info::None) => true,
        (Info::Single(a), Info::Single(b)) => a.id == b.id && a.name == b.name
            && a.class == b.class && a.own == b.own && a.builder == b.builder
            && a.shield.is_some() == b.shield.is_some() && a.progress.is_some() == b.progress.is_some(),
        (Info::Group(a), Info::Group(b)) => a.len() == b.len()
            && a.iter().zip(b).all(|(a,b)| a.id == b.id && a.name == b.name),
        _ => false,
    }
}

/// Patch text and progress without destroying headers, rows, glyph owners or
/// click targets. New structural children already contain their initial values.
#[allow(clippy::type_complexity)]
pub(super) fn update_values(
    feed: Res<super::model::HudFeed>,
    layout: Res<super::HudLayoutRes>,
    theme: Res<Theme>,
    mut texts: Query<(&ValueText, &mut crate::ui::sprite_font::SpriteText)>,
    mut bars: Query<(&ValueBar, Option<&mut crate::ui::theme::ProgressBar>, Option<&mut Node>, Option<&mut BackgroundColor>)>,
) {
    let resources = super::model::resources(&feed, layout.0.no_resource);
    let info = super::model::info(&feed);
    for (kind, mut text) in &mut texts {
        let value = match (kind, &info) {
            (ValueText::Resource(id), _) => resources.iter().find(|e| e.id == *id).map(|e| e.text.clone()),
            (ValueText::Hull, Info::Single(s)) => Some(format!("{}/{}", s.hull_now, s.hull_max)),
            (ValueText::Shield, Info::Single(s)) => s.shield.map(|v| format!("{}%", (v*100.0).round() as i32)),
            (ValueText::Built, Info::Single(s)) => s.progress.map(|v| format!("{}%", (v*100.0).round() as i32)),
            _ => None,
        };
        if let Some(value) = value && text.text != value { text.text = value; }
    }
    for (kind, progress, node, color) in &mut bars {
        let value = match (kind, &info) {
            (ValueBar::Hull, Info::Single(s)) => Some(s.hull),
            (ValueBar::Shield, Info::Single(s)) => s.shield,
            (ValueBar::Built, Info::Single(s)) => s.progress,
            (ValueBar::Group(id), Info::Group(tiles)) => tiles.iter().find(|t| t.id == *id).map(|t| t.hull),
            _ => None,
        };
        let Some(value) = value else { continue };
        if let Some(mut progress) = progress && progress.0 != value { progress.0 = value; }
        if matches!(kind, ValueBar::Group(_)) {
            let width = Val::Percent(value.clamp(0.0,1.0)*100.0);
            if let Some(mut node) = node && node.width != width { node.width = width; }
            let next = hull_colour(&theme, value);
            if let Some(mut color) = color && color.0 != next { color.0 = next; }
        }
    }
}

/// A grid cell with something to press.
#[derive(Component, Debug, Clone)]
pub struct GridCell(pub Act);

/// A group tile: a click selects (Shift removes) the unit.
#[derive(Component, Debug, Clone, Copy)]
pub struct GroupTile(pub EntityId);

/// Tile size in a group, as HUD.md has them (46 x 40 units).
pub const TILE_W: f32 = 46.0;
pub const TILE_H: f32 = 40.0;
const TILE_GAP: f32 = 4.0;

fn text(s: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    // The mod's own sprite font where it has one, the TTF otherwise (UI-12).
    crate::ui::sprite_font::sprite_text(s, size, color)
}

/// A cell's label: small, centred, broken anywhere to fit two lines in the cell.
fn cell_label(s: String, theme: &Theme) -> impl Bundle {
    (
        crate::ui::sprite_font::sprite_text_styled(
            s,
            theme.micro_size - 2.0,
            theme.text_on,
            crate::ui::sprite_font::SpriteTextStyle::default().centered().wrapped(CELL - 6.0),
        ),
    )
}

fn at(x: f32, y: f32, w: f32, h: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(x),
        top: Val::Px(y),
        width: Val::Px(w),
        height: Val::Px(h),
        ..default()
    }
}

/// Rebuild persistent chrome on a theme switch while retaining gameplay/UI state.
pub(super) fn reset_on_theme(
    mut commands: Commands,
    theme: Res<Theme>,
    bodies: Query<Entity, With<HudBody>>,
    mut state: ResMut<super::HudState>,
) {
    if !theme.is_changed() { return; }
    for entity in &bodies { commands.entity(entity).despawn(); }
    state.drawn = None;
    state.height = None;
}

pub(super) fn spawn_root(
    mut commands: Commands,
    theme: Res<Theme>,
    layout: Res<super::HudLayoutRes>,
    existing: Query<Entity, With<HudRoot>>,
    bodies: Query<Entity, With<HudBody>>,
    config: Res<super::HudConfig>,
    profile: Option<Res<crate::ui::profile::UiProfile>>,
) {
    // Retain the root and its camera/hosting metadata while replacing chrome.
    let root = existing.single().unwrap_or_else(|_| commands.spawn((
        HudRoot, Pickable::IGNORE,
        Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
    )).id());
    if super::stock::enabled(profile.as_deref(), &config) {
        for entity in &bodies { commands.entity(entity).despawn(); }
        return;
    }
    if !bodies.is_empty() { return; }
    let l = &layout.0;
    let r = l.resources;
    let row = l.bottom_row(None);
    // The grid and the info panel are moved to their places on the first refresh.
    commands.entity(root).with_child((HudBody, Pickable::IGNORE, Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }, children![
        (
            ResourceStrip,
            BackgroundColor(theme.panel.with_alpha(0.85)),
            BorderColor::all(theme.accent2),
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceEvenly,
                border: UiRect::left(Val::Px(theme.side_bar * 0.5)),
                ..at(r.x, r.y, r.w, r.h)
            },
        ),
        (
            InfoPanel,
            Visibility::Hidden,
            Node {
                flex_direction: FlexDirection::Column,
                padding: UiRect::axes(Val::Px(theme.pad + theme.side_bar), Val::Px(theme.gap)),
                row_gap: Val::Px(theme.gap * 0.5),
                border: UiRect { left: Val::Px(theme.side_bar), bottom: Val::Px(theme.bottom_bar * 0.6), ..default() },
                ..at(row.info.x, row.info.y, row.info.w, row.info.h)
            },
            BackgroundColor(theme.panel.with_alpha(0.9)),
            BorderColor { left: theme.accent2, bottom: theme.accent, ..BorderColor::all(Color::NONE) },
        ),
        (
            GridPanel,
            Node {
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..at(row.grid.x, row.grid.y, row.grid.w, row.grid.h)
            },
            BackgroundColor(theme.panel.with_alpha(0.9)),
            BorderColor::all(theme.accent2),
        ),
    ]));
}

/// Move a panel node to `r`.
pub fn place(node: &mut Node, r: super::layout::R) {
    let v = |a: Val, b: f32| if a == Val::Px(b) { a } else { Val::Px(b) };
    node.left = v(node.left, r.x);
    node.top = v(node.top, r.y);
    node.width = v(node.width, r.w);
    node.height = v(node.height, r.h);
}

/// The strip's content: a mark in the resource's colour, its name, its number.
pub fn fill_resources(commands: &mut Commands, theme: &Theme, strip: Entity, entries: &[ResEntry]) {
    let mut row = commands.entity(strip);
    for e in entries {
        let colour = resource_colour(theme, e.id);
        row.with_child((
            Node { align_items: AlignItems::Center, column_gap: Val::Px(theme.gap * 0.75), ..default() },
            Pickable::IGNORE,
            children![
                (Node { width: Val::Px(12.0), height: Val::Px(12.0), border_radius: BorderRadius::MAX, ..default() }, BackgroundColor(colour), Pickable::IGNORE),
                text(e.name.to_uppercase(), theme.micro_size, theme.text_dim),
                (ValueText::Resource(e.id), text(e.text.clone(), theme.text_size, theme.text)),
            ],
        ));
    }
}

/// The colour of a resource's mark.
pub fn resource_colour(theme: &Theme, id: usize) -> Color {
    match id {
        0 => theme.accent3,
        1 => theme.warn,
        2 => theme.accent2,
        3 => theme.good,
        _ => theme.accent,
    }
}

fn bar_row(theme: &Theme, label: &str, value: f32, shown: String, w: f32, kind: ValueBar, number: ValueText) -> impl Bundle {
    (
        Node { align_items: AlignItems::Center, column_gap: Val::Px(theme.gap), ..default() },
        Pickable::IGNORE,
        children![
            (Node { width: Val::Px(54.0), ..default() }, children![text(label.to_uppercase(), theme.micro_size, theme.text_dim)]),
            (kind, progress_bar(theme, w, value)),
            (number, text(shown, theme.small_size, theme.text)),
        ],
    )
}

/// The info panel's frame and content, for a panel of `size`.
pub fn fill_info(commands: &mut Commands, theme: &Theme, panel: Entity, info: &Info, size: Vec2) {
    commands.entity(panel).insert(if matches!(info, Info::None) { Visibility::Hidden } else { Visibility::Inherited });
    let mut p = commands.entity(panel);
    match info {
        Info::None => {}
        Info::Single(s) => {
            fill_single(&mut p, theme, s, size);
        }
        Info::Group(tiles) => fill_group(&mut p, theme, tiles, size),
    }
}

fn fill_single(p: &mut EntityCommands, theme: &Theme, s: &Single, size: Vec2) {
    let bar = ((size.x - 2.0 * theme.pad - theme.side_bar) * 0.5).clamp(120.0, 360.0);
    let name_colour = if s.own { theme.title } else { theme.bad };
    p.with_child((
        Node { align_items: AlignItems::Baseline, column_gap: Val::Px(theme.gap), ..default() },
        Pickable::IGNORE,
        children![text(s.name.to_uppercase(), theme.heading_size, name_colour), text(s.class.to_uppercase(), theme.small_size, theme.text_dim)],
    ));
    p.with_child(bar_row(theme, &tr("Hull"), s.hull, format!("{}/{}", s.hull_now, s.hull_max), bar, ValueBar::Hull, ValueText::Hull));
    if let Some(sh) = s.shield {
        p.with_child(bar_row(theme, &tr("Shield"), sh, format!("{}%", (sh * 100.0).round() as i32), bar, ValueBar::Shield, ValueText::Shield));
    }
    if let Some(pr) = s.progress {
        p.with_child(bar_row(theme, &tr("Built"), pr, format!("{}%", (pr * 100.0).round() as i32), bar, ValueBar::Built, ValueText::Built));
    }
    if s.builder {
        p.with_child(text(tr("Select Build in the grid to queue ships"), theme.small_size, theme.text_dim));
    }
}

fn fill_group(p: &mut EntityCommands, theme: &Theme, tiles: &[Tile], size: Vec2) {
    let inner = size.x - 2.0 * theme.pad - theme.side_bar;
    let per_row = ((inner + TILE_GAP) / (TILE_W + TILE_GAP)).floor().max(1.0) as usize;
    p.with_child(text(trf("{0} SELECTED", &[&tiles.len()]), theme.small_size, theme.text_dim));
    for row in tiles.chunks(per_row) {
        p.with_child((
            Node { column_gap: Val::Px(TILE_GAP), ..default() },
            Pickable::IGNORE,
            Children::spawn(bevy::ecs::spawn::SpawnIter(row.iter().map(|t| tile(theme, t)).collect::<Vec<_>>().into_iter())),
        ));
    }
}

fn hull_colour(theme: &Theme, hull: f32) -> Color {
    if hull > 0.5 { theme.good } else if hull > 0.25 { theme.warn } else { theme.bad }
}

fn tile(theme: &Theme, t: &Tile) -> impl Bundle {
    let hull_colour = hull_colour(theme, t.hull);
    (
        GroupTile(t.id),
        UiButton,
        WidgetState::Normal,
        Tooltip::new(t.name.clone()),
        Node {
            width: Val::Px(TILE_W),
            height: Val::Px(TILE_H),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::SpaceBetween,
            padding: UiRect::all(Val::Px(3.0)),
            border_radius: BorderRadius::all(Val::Px(4.0)),
            ..default()
        },
        BackgroundColor(theme.button),
        children![
            (crate::ui::theme::ButtonText, text(t.name.chars().take(5).collect::<String>().to_uppercase(), theme.micro_size, theme.text_on)),
            (
                Node { width: Val::Percent(100.0), height: Val::Px(6.0), ..default() },
                BackgroundColor(theme.track),
                Pickable::IGNORE,
                children![(
                    ValueBar::Group(t.id),
                    Node { width: Val::Percent(t.hull.clamp(0.0, 1.0) * 100.0), height: Val::Percent(100.0), ..default() },
                    BackgroundColor(hull_colour),
                    Pickable::IGNORE,
                )],
            ),
        ],
    )
}

/// The grid: [`GRID_COLS`] x [`GRID_ROWS`] cells, the empty ones as dim slots.
pub fn fill_grid(commands: &mut Commands, theme: &Theme, panel: Entity, buttons: &[Button]) {
    let mut p = commands.entity(panel);
    for cell in 0..GRID_COLS * GRID_ROWS {
        let (c, r) = (cell % GRID_COLS, cell / GRID_COLS);
        let x = GRID_PAD + c as f32 * (CELL + CELL_GAP) - 2.0;
        let y = GRID_PAD + r as f32 * (CELL + CELL_GAP) - 2.0;
        match buttons.iter().find(|b| b.cell == cell) {
            None => {
                p.with_child((Node { border_radius: BorderRadius::all(Val::Px(6.0)), ..at(x, y, CELL, CELL) }, BackgroundColor(theme.button_off.with_alpha(0.35)), Pickable::IGNORE));
            }
            Some(b) => {
                let state = if b.look == Look::Locked { WidgetState::Disabled } else { WidgetState::Normal };
                let mut tip = Tooltip::new(b.tip.clone());
                if let Some(v) = &b.verbose {
                    tip = tip.verbose(v.clone());
                }
                p.with_child((
                    GridCell(b.act.clone()),
                    GuiButton { names: b.sprite_names.clone(), keep_label: b.keep_label },
                    UiButton,
                    state,
                    tip,
                    Node {
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        border_radius: BorderRadius::all(Val::Px(6.0)),
                        padding: UiRect::all(Val::Px(2.0)),
                        ..at(x, y, CELL, CELL)
                    },
                    BackgroundColor(if state == WidgetState::Disabled { theme.button_off } else { theme.button }),
                    children![
                        (GuiButtonLabel, crate::ui::theme::ButtonText, cell_label(b.label.clone(), theme)),
                        (
                            // The stock does not cover it: a warning pip in the corner.
                            Node {
                                display: if b.look == Look::Poor { Display::Flex } else { Display::None },
                                border_radius: BorderRadius::MAX,
                                ..at(CELL - 14.0, 4.0, 8.0, 8.0)
                            },
                            BackgroundColor(theme.bad),
                            Pickable::IGNORE,
                        ),
                    ],
                ));
            }
        }
    }
}

/// Where each panel is for a layout.
pub fn layout_nodes(l: &HudLayout, height: Option<InfoHeight>) -> (super::layout::R, super::layout::R, super::layout::R) {
    let row = l.bottom_row(height);
    (l.resources, row.info, row.grid)
}
