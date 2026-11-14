//! CFG/SPR presentation of the shared HUD actions. No simulation policy lives here.
mod commands;
mod info;
pub(super) mod infobar;
pub mod layout;

use super::layout::R;
use super::{
    HudConfig, HudFeed, HudState,
    model::{self, Act, Button, Info, Look, Page, TargetMode},
    panels::{GridCell, HudBody, HudRoot},
};
use crate::ui::{
    gui_sprites::{GuiActive, GuiButton, GuiButtonColors, GuiButtonLabel},
    notices::Tooltip,
    profile::UiProfile,
    theme::{ButtonClicked, Theme, UiButton, WidgetState},
};
use crate::viewer::Mounted;
use bevy::{picking::Pickable, prelude::*};
use commands::{NativeCommand, NativeCommands};
use layout::{PaletteLayout, StockLayout};
use sta_content::GuiLayout;

#[derive(Component)]
pub(super) struct StockBody;
#[derive(Component)]
pub(super) struct StockInfo;
#[derive(Component)]
pub(super) struct StockInfoBar;
#[derive(Component)]
pub(super) struct StockGrid;
#[derive(Component)]
pub(super) struct StockNav(i32);

#[derive(Resource, Default)]
pub(super) struct StockState {
    pub layout: Option<StockLayout>,
    pub solid: Vec<R>,
    cfg: Option<GuiLayout>,
    profile: Option<UiProfile>,
    native: NativeCommands,
    info_style: infobar::InfoStyle,
    info: Option<Info>,
    grid: Vec<Button>,
    page: Page,
    build_page: usize,
    loaded: bool,
}

pub(super) fn enabled(profile: Option<&UiProfile>, config: &HudConfig) -> bool {
    profile.is_some_and(|p| p.is_stock()) && config.gui.is_some()
}

/// The separate resource panels of a GUI cfg without `resourcePanelArea` (prefix, readout id,
/// hide flag): dilithium 2, crew 0, officers 1.
pub(super) const SEPARATE_RESOURCES: [(&str, usize, &str); 3] =
    [("dilithium", 2, "noDilithium"), ("crew", 0, "noCrew"), ("officer", 1, "noOfficers")];

/// True when the cfg authors no `resourcePanelArea` key at all (a zero rectangle still counts
/// as authored and hides the strip).
pub(super) fn separate_resources(layout: &StockLayout) -> bool {
    layout.raw_rect("resourcePanelArea").is_none()
}

/// The resource strip's visible panels: the single one, or the separate ones.
pub(super) fn resource_panels(layout: &StockLayout) -> Vec<R> {
    if separate_resources(layout) {
        SEPARATE_RESOURCES.iter().filter_map(|(prefix, ..)| layout.panel(prefix)).collect()
    } else {
        layout.panel("resource").into_iter().collect()
    }
}

pub(super) fn node(r: R) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(r.x),
        top: Val::Px(r.y),
        width: Val::Px(r.w),
        height: Val::Px(r.h),
        ..default()
    }
}

fn metadata<'a>(native: &'a NativeCommands, act: &Act) -> Option<&'a NativeCommand> {
    native.get(match act {
        Act::Target(TargetMode::Move) => "move",
        Act::Target(TargetMode::Attack) => "attack",
        Act::Target(TargetMode::Mine) => "harvest",
        Act::Target(TargetMode::Rally) => "rally",
        Act::Stop => "stop",
        Act::Cancel(_) => "cancelbuild",
        _ => return None,
    })
}

fn menu_button(id: u32, name: &str) -> Button {
    Button {
        cell: 0,
        label: name.to_uppercase(),
        act: Act::OpenNativeMenu(id),
        look: Look::Ready,
        tip: name.to_owned(),
        verbose: None,
        sprite_names: vec![format!("b_{name}"), name.to_owned()],
        keep_label: false,
    }
}

/// Only already-supported actions can enter a native menu. The ODF is metadata,
/// never authority to enable a new order backend.
fn buttons(feed: &HudFeed, page: Page, native: &NativeCommands) -> Vec<Button> {
    let mut buttons = model::grid_with_capacity(feed, page, usize::MAX);
    let mut groups = Vec::new();
    buttons.retain(|button| {
        if matches!(page, Page::Build | Page::Formations) {
            return true;
        }
        let group = metadata(native, &button.act)
            .and_then(|m| m.menu.as_deref())
            .and_then(|m| native.menu_id(m));
        match page {
            Page::Native(id) => group == Some(id),
            _ => {
                if let Some(id) = group {
                    if !groups.contains(&id) {
                        groups.push(id);
                    }
                    false
                } else {
                    true
                }
            }
        }
    });
    if let Page::Native(_) = page {
        buttons.push(Button {
            cell: usize::MAX,
            label: "BACK".into(),
            act: Act::Back,
            look: Look::Ready,
            tip: "Back".into(),
            verbose: None,
            sprite_names: vec!["b_back".into()],
            keep_label: false,
        });
    } else {
        for id in groups {
            if let Some(name) = native.menu_name(id) {
                buttons.push(menu_button(id, name));
            }
        }
    }
    for button in &mut buttons {
        if let Some(meta) = metadata(native, &button.act) {
            button.sprite_names = meta.sprite_names();
            if let Some(tip) = &meta.tooltip {
                button.tip = tip.clone();
            }
            if let Some(verbose) = &meta.verbose {
                button.verbose = Some(verbose.clone());
            }
        }
    }
    // Stock A2 compacts visible buttons by command-type priority (read from the executable),
    // not by preferredPosition alone. Equal-priority ordering uses the authored
    // preferred position then shared action order; native qsort ties are unspecified.
    buttons.sort_by_key(|b| {
        let meta = metadata(native, &b.act);
        let rank = meta.map_or(9, |m| command_rank(&m.command));
        let position = meta
            .and_then(|m| m.preferred)
            .map_or((usize::MAX, b.cell), |[x, y]| (y, x));
        (rank, position)
    });
    buttons
}

/// Twice the native static command priority (read from the stock A2 command-name
/// table and priority routine; static evidence).
fn command_rank(command: &str) -> u8 {
    match command.to_ascii_uppercase().as_str() {
        "NONE" => 0,
        "GO" | "GO_SINGLE" | "WORMHOLE" | "GO_WARP" | "GO_WARP_SINGLE" => 2,
        "QUEUE" | "FOLLOW" => 4,
        "TRANSPORT" | "TRANSPORT_SPECIAL" => 6,
        "START_TRADE" | "END_TRADE" => 8,
        "MINE" => 9,
        "COLONIZE" => 10,
        "REPAIR_OTHER" => 12,
        "ATTACK" | "ANCHORED_ATTACK" | "TRANSPORT_ATTACK" => 14,
        "SPECIAL_ATTACK" => 16,
        _ => 5,
    }
}

fn visible_cells(p: &PaletteLayout, hud: R) -> Vec<R> {
    let screen = Vec2::new(hud.right(), hud.bottom());
    // Iterate screen-derived bounds, not arbitrary mod dimensions.
    let cols = p
        .columns
        .min(((screen.x / p.cell.x.max(1.0)).ceil() as usize).saturating_add(1));
    let rows = p
        .rows
        .min(((screen.y / p.cell.y.max(1.0)).ceil() as usize).saturating_add(1));
    (0..rows)
        .flat_map(|row| (0..cols).map(move |col| p.cell_rect(col, row)))
        .filter(|r| r.x >= hud.x && r.y >= hud.y && r.right() <= screen.x + 0.5 && r.bottom() <= screen.y + 0.5)
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn spawn_button(
    commands: &mut Commands,
    parent: Entity,
    theme: &Theme,
    rect: R,
    b: &Button,
    native: &NativeCommands,
    hud: &HudState,
) {
    let state = if b.look == Look::Locked {
        WidgetState::Disabled
    } else {
        WidgetState::Normal
    };
    let mut colors = GuiButtonColors::default();
    if let Some(meta) = metadata(native, &b.act) {
        colors.normal = meta.normal.unwrap_or(colors.normal);
        colors.hovered = meta.hovered.unwrap_or(colors.hovered);
        colors.pressed = meta.pressed.unwrap_or(colors.pressed);
        colors.disabled = meta.disabled.unwrap_or(colors.disabled);
        colors.active = meta.active.unwrap_or(colors.active);
    }
    let active = matches!(b.act, Act::Target(mode) if hud.targeting == Some(mode));
    let mut tip = Tooltip::new(b.tip.clone());
    if let Some(v) = &b.verbose {
        tip = tip.verbose(v.clone());
    }
    commands.spawn((
        GridCell(b.act.clone()),
        UiButton,
        state,
        colors,
        GuiActive(active),
        tip,
        GuiButton {
            names: b.sprite_names.clone(),
            keep_label: b.keep_label,
        },
        Node {
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..node(rect)
        },
        BackgroundColor(theme.button),
        ChildOf(parent),
        children![(
            GuiButtonLabel,
            Text::new(b.label.clone()),
            TextFont {
                font_size: (rect.h * 0.22).clamp(18.0, 22.0).into(),
                ..default()
            },
            TextColor(theme.text),
            TextLayout::new(Justify::Center, LineBreak::WordOrCharacter),
            Pickable::IGNORE
        )],
    ));
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn refresh(
    mut commands: Commands,
    config: Res<HudConfig>,
    profile: Option<Res<UiProfile>>,
    theme: Res<Theme>,
    feed: Res<HudFeed>,
    mut hud: ResMut<HudState>,
    mounted: Option<Res<Mounted>>,
    roots: Query<Entity, With<HudRoot>>,
    bodies: Query<Entity, With<StockBody>>,
    legacy: Query<Entity, With<HudBody>>,
    infos: Query<Entity, With<StockInfo>>,
    info_bars: Query<Entity, With<StockInfoBar>>,
    grids: Query<Entity, With<StockGrid>>,
    mut state: ResMut<StockState>,
) {
    if !enabled(profile.as_deref(), &config) {
        for entity in &bodies {
            commands.entity(entity).despawn();
        }
        if state.cfg.is_some() {
            commands.insert_resource(crate::render::overlays::GaugeStyle::default());
        }
        state.layout = None;
        state.loaded = false;
        state.solid.clear();
        state.cfg = None;
        state.info = None;
        state.grid.clear();
        if matches!(hud.page, Page::Native(_)) {
            hud.page = Page::Top;
        }
        return;
    }
    let Ok(root) = roots.single() else { return };
    for entity in &legacy {
        commands.entity(entity).despawn();
    }
    let mount_changed = mounted.as_ref().is_some_and(|m| m.is_changed());
    if !state.loaded || mount_changed {
        state.info_style = mounted.as_ref().map_or_else(infobar::InfoStyle::default, |m| infobar::InfoStyle::load(&m.0));
        state.native = mounted
            .as_ref()
            .map_or_else(NativeCommands::default, |m| NativeCommands::load(&m.0));
        state.loaded = true;
    }
    let changed = (config.is_changed() && state.cfg.as_ref() != config.gui.as_ref())
        || state.profile != profile.as_deref().copied()
        || state
            .layout
            .as_ref()
            .is_none_or(|l| (l.screen.x - super::layout::SCREEN_H * config.aspect).abs() > 0.1)
        || theme.is_changed()
        || mount_changed;
    let replaced = changed || bodies.is_empty();
    let body = if replaced {
        for entity in &bodies {
            commands.entity(entity).despawn();
        }
        state.cfg = config.gui.clone();
        state.profile = profile.as_deref().copied();
        state.layout = config
            .gui
            .as_ref()
            .map(|cfg| StockLayout::for_aspect(cfg, config.aspect));
        commands.insert_resource(crate::render::overlays::GaugeStyle::from_cfg(
            state.layout.as_ref().and_then(|layout| layout.number("selectionGaugeHeight")),
            state.layout.as_ref().and_then(|layout| layout.number("selectionGaugeGap")),
        ));
        state.info = None;
        state.grid.clear();
        commands
            .spawn((
                StockBody,
                Pickable::IGNORE,
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                ChildOf(root),
            ))
            .id()
    } else {
        bodies.single().unwrap()
    };
    let layout = state.layout.take().unwrap();
    let now = model::info(&feed);
    let structural = state
        .info
        .as_ref()
        .is_none_or(|old| !super::panels::same_info(old, &now));
    if structural {
        if !replaced {
            for entity in &infos {
                commands.entity(entity).despawn();
            }
            for entity in &info_bars {
                commands.entity(entity).despawn();
            }
        }
        let parent = commands
            .spawn((StockInfo, Pickable::IGNORE, Node::default(), ChildOf(body)))
            .id();
        info::spawn(&mut commands, parent, &layout, &feed, &theme);
        let bar = commands.spawn((StockInfoBar, Pickable::IGNORE, Node::default(), ChildOf(body))).id();
        infobar::spawn(&mut commands, bar, &layout, &theme, layout::info_variant(&now));
    }
    state.info = Some(now.clone());
    let mut next = buttons(&feed, hud.page, &state.native);
    if matches!(hud.page, Page::Native(_)) && next.iter().all(|b| b.act == Act::Back) {
        hud.page = Page::Top;
        next = buttons(&feed, hud.page, &state.native);
    }
    if state.page != hud.page {
        state.build_page = 0;
    }
    // Active-state changes patch the existing GuiActive components below.
    let rebuild = changed || structural || state.page != hud.page || state.grid != next || grids.is_empty();
    state.solid = resource_panels(&layout);
    state.solid.extend([layout.panel("minimap"), layout.info_area(layout::info_variant(&now))].into_iter().flatten());
    if layout.info_area(layout::info_variant(&now)).is_some() {
        let variant = layout::info_variant(&now);
        let origin =
            layout.info_origin(variant) + layout.point(&format!("infoBarOffset_{variant}")).unwrap_or(Vec2::ZERO);
        state.solid.extend(
            ["shipName", "hull", "shields", "crew_amt", "energy_amt", "icon_hull", "icon_shields", "icon_crew_amt", "icon_energy_amt"]
                .into_iter()
                .filter_map(|key| layout.at(origin, key)),
        );
    }
    if let Some(palette) = layout.palette() {
        let cells = visible_cells(&palette, layout.hud);
        let capacity = cells.len();
        let pinned: Vec<_> = next
            .iter()
            .filter(|b| matches!(b.act, Act::Back | Act::Cancel(_)))
            .collect();
        let paged = next.len() > capacity && capacity >= pinned.len() + 3;
        let slots = if paged { capacity - 2 - pinned.len() } else { capacity };
        let content: Vec<_> = next
            .iter()
            .filter(|b| !paged || !matches!(b.act, Act::Back | Act::Cancel(_)))
            .collect();
        let pages = content.len().div_ceil(slots.max(1)).max(1);
        state.build_page = state.build_page.min(pages - 1);
        let start = state.build_page * slots;
        let mut shown: Vec<_> = content
            .iter()
            .skip(start)
            .take(slots)
            .enumerate()
            .map(|(i, b)| (i, *b))
            .collect();
        if paged {
            shown.extend(pinned.iter().enumerate().map(|(i, b)| (slots + i, *b)));
        }
        state.solid.extend(shown.iter().map(|(i, _)| cells[*i]));
        if paged {
            state.solid.extend_from_slice(&cells[capacity - 2..]);
        }
        if rebuild {
            if !replaced {
                for entity in &grids {
                    commands.entity(entity).despawn();
                }
            }
            let parent = commands
                .spawn((StockGrid, Pickable::IGNORE, Node::default(), ChildOf(body)))
                .id();
            for (index, button) in &shown {
                spawn_button(
                    &mut commands,
                    parent,
                    &theme,
                    cells[*index],
                    button,
                    &state.native,
                    &hud,
                );
            }
            if paged {
                for (index, delta, label) in [(capacity - 2, -1, "PREV"), (capacity - 1, 1, "NEXT")] {
                    commands.spawn((
                        StockNav(delta),
                        UiButton,
                        if (delta < 0 && state.build_page == 0) || (delta > 0 && state.build_page + 1 == pages) {
                            WidgetState::Disabled
                        } else {
                            WidgetState::Normal
                        },
                        node(cells[index]),
                        BackgroundColor(theme.button),
                        ChildOf(parent),
                        children![(
                            Text::new(label),
                            TextFont {
                                font_size: 18.0.into(),
                                ..default()
                            },
                            TextColor(theme.text),
                            Pickable::IGNORE
                        )],
                    ));
                }
            }
        }
    }
    // A sheet rectangle may start left of the canvas (FO's minimap panel at x = -1): the
    // pointer-solid part never reaches into the margin, which belongs to the world. The art
    // is not clipped.
    let hud_rect = layout.hud;
    for r in &mut state.solid {
        let (left, right) = (r.x.max(hud_rect.x), r.right().min(hud_rect.right()));
        *r = R::new(left, r.y, (right - left).max(0.0), r.h);
    }
    state.solid.retain(|r| r.w > 0.0);
    state.grid = next;
    state.page = hud.page;
    state.layout = Some(layout);
}

pub(super) fn active(hud: Res<HudState>, mut buttons: Query<(&GridCell, &mut GuiActive)>) {
    for (cell, mut active) in &mut buttons {
        let next = matches!(cell.0,Act::Target(mode) if hud.targeting==Some(mode));
        if active.0 != next {
            active.0 = next;
        }
    }
}

pub(super) fn navigate(mut clicks: MessageReader<ButtonClicked>, nav: Query<&StockNav>, mut state: ResMut<StockState>) {
    for ButtonClicked(entity) in clicks.read() {
        if let Ok(nav) = nav.get(*entity) {
            state.build_page = state.build_page.saturating_add_signed(nav.0 as isize);
            // Force only the palette to redraw on the next frame.
            state.grid.clear();
        }
    }
}

pub(super) use info::update as update_values;

#[cfg(test)]
mod gpu;
#[cfg(test)]
mod tests;
