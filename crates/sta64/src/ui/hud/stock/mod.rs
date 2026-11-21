//! CFG/SPR presentation of the shared HUD actions. No simulation policy lives here.
pub(in crate::ui::hud) mod a1;
pub(in crate::ui) mod buttons;
pub(in crate::ui::hud) mod commands;
mod info;
pub(super) mod infobar;
pub mod layout;
mod window;

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
    sprite_font::{SpriteTextStyle, sprite_text_styled},
    stock_font::{StockCanvas, StockTextCanvas},
    theme::{ButtonClicked, Theme, UiButton, WidgetState},
};
use crate::viewer::Mounted;
use bevy::{picking::Pickable, prelude::*};
use commands::{NativeCommand, NativeCommands};
use layout::{PaletteLayout, StockLayout};
use sta_content::{ButtonSlots, GuiLayout};

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
    /// Armada I: the `game_cfg.h` slot map and the extra info values drawn (UI-19).
    slots: Option<ButtonSlots>,
    a1_shape: Option<a1::info::Shape>,
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

/// Reuse the stock info panel's five authored wireframe layers for queue jobs.
pub(super) fn queue_wireframe(commands: &mut Commands, root: Entity, r: R, model: &str) {
    info::wireframe_layers(commands, root, r, model);
}

fn metadata<'a>(native: &'a NativeCommands, act: &Act) -> Option<&'a NativeCommand> {
    if let Act::NativeFormation(odf) = act {
        return native.commands.get(odf);
    }
    native.get(match act {
        Act::Target(TargetMode::Move) => "move",
        Act::Target(TargetMode::Attack) => "attack",
        Act::Target(TargetMode::Mine) => "harvest",
        Act::Target(TargetMode::Rally) => "rally",
        Act::Target(TargetMode::Colonize) => "colonize",
        Act::Target(TargetMode::Transport) => "transport",
        Act::Target(TargetMode::TransportAttack) => "capture",
        Act::Target(TargetMode::Repair) => "repair",
        Act::Target(TargetMode::Decommission) => "decom",
        Act::Stop => "stop",
        Act::Recrew => "recrew",
        Act::Cancel(_) => "cancelbuild",
        _ => return None,
    })
}

fn menu_button(id: u32, name: &str) -> Button {
    let mut sprite_names = vec![format!("b_{name}"), name.to_owned()];
    if name.eq_ignore_ascii_case("orders") {
        sprite_names.insert(0, "b_order".into());
    }
    Button {
        cell: 0,
        label: name.to_uppercase(),
        act: Act::OpenNativeMenu(id),
        look: Look::Ready,
        tip: name.to_owned(),
        verbose: None,
        sprite_names,
        keep_label: false,
    }
}

/// Native menus contain HUD actions with an explicit capability gate; ODF
/// metadata cannot bypass live Recrew eligibility.
pub(in crate::ui::hud) fn buttons(feed: &HudFeed, page: Page, native: &NativeCommands) -> Vec<Button> {
    buttons_with(feed, page, native, None)
}

/// [`buttons`] with the mounted string table, which gives the item-page menu buttons their tooltips.
pub(in crate::ui::hud) fn buttons_with(feed: &HudFeed, page: Page, native: &NativeCommands, strings: Option<&crate::ui::i18n::Strings>) -> Vec<Button> {
    let mut buttons = model::grid_with_capacity(feed, page, usize::MAX);
    // The stock HUD shows only what the original shows: TRANSPORT only where the install's command data
    // gives it a place (Armada I has none here: it puts it on its speed rail, `a1::speed`); the fallback
    // cell is for STA64's own layout.
    buttons.retain(|b| b.act != Act::Target(TargetMode::Transport) || metadata(native, &b.act).is_some_and(|m| m.preferred.is_some()));
    // Capture is an authored stock command in the trade palette. It has no LCARS
    // fallback cell: adding one would compete with the existing special slots.
    if matches!(page, Page::Top | Page::Native(_))
        && native.get("capture").is_some_and(|m| m.preferred.is_some() && m.command.eq_ignore_ascii_case("TRANSPORT_ATTACK"))
        && super::commands::shared(feed).transport_attack_ready
    {
        buttons.push(model::button(
            usize::MAX,
            &crate::ui::i18n::tr("Capture"),
            Act::Target(TargetMode::TransportAttack),
            &crate::ui::i18n::tooltip_key("Transport attack"),
        ));
    }
    if page == Page::Formations && buttons.iter().any(|b| matches!(b.act, Act::Formation(_))) {
        let formations: Vec<_> = native.formations().collect();
        if !formations.is_empty() {
            buttons.retain(|b| !matches!(b.act, Act::Formation(_)));
            for (cell, record) in formations.into_iter().enumerate() {
                let stem = record.odf.trim_end_matches(".odf");
                buttons.push(Button {
                    cell,
                    label: stem.to_uppercase(),
                    act: Act::NativeFormation(stem.to_owned()),
                    look: Look::Locked,
                    tip: record.tooltip.clone().unwrap_or_else(|| stem.to_owned()),
                    verbose: record.verbose.clone(),
                    sprite_names: record.formation_sprite_names(),
                    keep_label: false,
                });
            }
        }
    }
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
            button.sprite_names = if matches!(button.act, Act::NativeFormation(_)) {
                meta.formation_sprite_names()
            } else {
                meta.sprite_names()
            };
            if let Some(tip) = &meta.tooltip {
                button.tip = tip.clone();
            }
            if let Some(verbose) = &meta.verbose {
                button.verbose = Some(verbose.clone());
            }
        }
    }
    // The item-page menu buttons take the string table's short and verbose tooltips when the
    // mounted table has the keys (otherwise the source text stays).
    if let Some(strings) = strings {
        for button in &mut buttons {
            if let Some((short, verbose)) = model::menu_tip_keys(&button.act)
                && strings.has_game_key(short)
            {
                button.tip = short.to_owned();
                button.verbose = strings.has_game_key(verbose).then(|| verbose.to_owned());
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

/// What a palette shows: the buttons with the cell index each takes, whether the list is paged
/// and how many pages there are.
pub(in crate::ui::hud) struct Placed<'a> {
    pub shown: Vec<(usize, &'a Button)>,
    pub paged: bool,
    pub pages: usize,
    pub page: usize,
}

/// Places a page's buttons on a palette of `capacity` cells. The buttons are packed in list
/// order, except that the build page leaves a cell empty for each of `holes` (the build cells of
/// items a text tree does not offer, `BuildItem::slot`); with no holes the placement is the
/// plain list order. A list longer than the palette is paged, Back and Cancel stay on every page.
pub(in crate::ui::hud) fn place_buttons<'a>(next: &'a [Button], capacity: usize, build_page: usize, holes: &[usize]) -> Placed<'a> {
    // The cell index of each button: its place in the list plus an offset that counts the empty
    // build cells before it. The list is sorted by command rank and position, so Cancel, Back and
    // other buttons may sit anywhere among the build buttons. Invariant: the offset only changes
    // at a build button (to the number of empty cells below its build cell) and never goes down
    // along the list, so the indices strictly increase and no two buttons share a cell, whatever
    // the sort order. A button that is not a build item keeps the offset of the build button
    // before it; empty cells after the last build item push nothing (interim: whether the
    // original pushes Cancel and Back along is not measured).
    let index = |content: &[&Button]| -> Vec<usize> {
        let mut offset = 0;
        content
            .iter()
            .enumerate()
            .map(|(i, b)| {
                if matches!(b.act, Act::Build { .. }) {
                    offset = offset.max(model::holes_before(b, holes));
                }
                i + offset
            })
            .collect()
    };
    let all: Vec<&Button> = next.iter().collect();
    let all_index = index(&all);
    let pinned: Vec<&Button> = next.iter().filter(|b| matches!(b.act, Act::Back | Act::Cancel(_))).collect();
    let total = all_index.last().map_or(0, |i| i + 1);
    let paged = total.max(next.len()) > capacity && capacity >= pinned.len() + 3;
    let slots = if paged { capacity - 2 - pinned.len() } else { capacity };
    let content: Vec<&Button> = next.iter().filter(|b| !paged || !matches!(b.act, Act::Back | Act::Cancel(_))).collect();
    let content_index = index(&content);
    let total = content_index.last().map_or(0, |i| i + 1);
    let pages = total.div_ceil(slots.max(1)).max(1);
    let page = build_page.min(pages - 1);
    let start = page * slots;
    let mut shown: Vec<(usize, &Button)> = content
        .iter()
        .zip(&content_index)
        .filter(|(_, i)| **i >= start && **i < start + slots)
        .map(|(b, i)| (i - start, *b))
        .collect();
    if paged {
        shown.extend(pinned.iter().enumerate().map(|(i, b)| (slots + i, *b)));
    }
    Placed { shown, paged, pages, page }
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
            sprite_text_styled(
                b.label.clone(),
                (rect.h * 0.22).clamp(18.0, 22.0),
                theme.text,
                // STA64's fallback label (shown only where the sprite is missing), not in the original.
                SpriteTextStyle::default().centered().wrapped(rect.w).stock_fallback(),
            ),
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
    strings: Option<Res<crate::ui::i18n::Strings>>,
    (action, more): (Option<Res<crate::render::action_camera::ActionView>>, Res<buttons::ShowMore>),
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
            commands.insert_resource(StockTextCanvas::default());
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
    if mount_changed {
        state.slots = None;
    }
    let a1 = a1::is_a1(profile.as_deref());
    if a1 && state.slots.is_none() && let Some(m) = mounted.as_ref() {
        // UI-19: `game_cfg.h` as the mod chain inherits it (ADR 0004 D9, D11).
        state.slots = Some(match ButtonSlots::load(&m.0) {
            Ok((slots, warnings)) => {
                for w in &warnings {
                    warn!("hud: {}: {}", w.file, w.message);
                }
                slots
            }
            Err(error) => {
                warn!("hud: game_cfg.h: {error}");
                ButtonSlots::default()
            }
        });
    }
    let changed = (config.is_changed() && state.cfg.as_ref() != config.gui.as_ref())
        || state.profile != profile.as_deref().copied()
        || state
            .layout
            .as_ref()
            .is_none_or(|l| (l.screen.x - super::layout::SCREEN_H * config.aspect).abs() > 0.1 || l.fit != config.fit)
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
            .map(|cfg| StockLayout::for_aspect_fit(cfg, config.aspect, config.fit));
        commands.insert_resource(StockTextCanvas(state.layout.as_ref().map(|l| StockCanvas {
            hud: Vec2::new(l.hud.w, l.hud.h),
            window: l.screen,
            fit: l.fit,
        })));
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
    if a1 && replaced {
        a1::spawn_frames(&mut commands, body, &layout);
        a1::action::spawn(&mut commands, body, &layout, &theme, strings.as_deref(), action.as_deref());
    }
    if !a1 && replaced {
        buttons::spawn(&mut commands, body, &layout, &theme, more.0);
        // #1073: the cinematic window, when the mounted layout declares one (nothing otherwise).
        window::spawn_a2(&mut commands, body, &layout, action.as_deref());
    }
    let now = model::info(&feed);
    let a1_shape = a1.then(|| a1::info::extra(&feed).map(|e| e.shape(&state.info_style))).flatten();
    let structural = state
        .info
        .as_ref()
        .is_none_or(|old| !super::panels::same_info(old, &now))
        || state.a1_shape != a1_shape;
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
        let bar = commands.spawn((StockInfoBar, Pickable::IGNORE, Node::default(), ChildOf(body))).id();
        if a1 {
            a1::info::spawn(&mut commands, parent, &layout, &feed, &theme, &state.info_style, strings.as_deref());
        } else {
            info::spawn(&mut commands, parent, &layout, &feed, &theme);
            infobar::spawn(&mut commands, bar, &layout, &theme, layout::info_variant(&now));
        }
    }
    state.info = Some(now.clone());
    state.a1_shape = a1_shape;
    if a1 {
        let settled = a1::control::settle_page(&feed, hud.page);
        if settled != hud.page {
            hud.page = settled;
        }
    }
    let mut next = if a1 {
        state.slots.as_ref().map_or_else(Vec::new, |slots| a1::control::buttons(&feed, hud.page, slots, strings.as_deref()))
    } else {
        buttons_with(&feed, hud.page, &state.native, strings.as_deref())
    };
    if a1 {
        // The speed rail is on every page: the transport button is not a control slot.
        next.extend(a1::speed::buttons(&feed, strings.as_deref()));
    }
    if !a1 && matches!(hud.page, Page::Native(_)) && next.iter().all(|b| b.act == Act::Back) {
        hud.page = Page::Top;
        next = buttons_with(&feed, hud.page, &state.native, strings.as_deref());
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
    if !a1 {
        // The window's art takes the pointer like the other panels (none when the layout declares it off).
        state.solid.extend(layout.panel("cinematic"));
    }
    if a1 {
        if rebuild {
            if !replaced {
                for entity in &grids {
                    commands.entity(entity).despawn();
                }
            }
            let parent = commands.spawn((StockGrid, Pickable::IGNORE, Node::default(), ChildOf(body))).id();
            a1::control::spawn_grid(&mut commands, parent, &theme, &layout, &next, &state, &hud);
        }
        state.solid.extend(a1::solid(&layout));
    } else if let Some(palette) = layout.palette() {
        let cells = visible_cells(&palette, layout.hud);
        let capacity = cells.len();
        let holes = if hud.page == Page::Build { model::build_holes_of(&feed) } else { Vec::new() };
        let place = place_buttons(&next, capacity, state.build_page, &holes);
        state.build_page = place.page;
        let (paged, pages) = (place.paged, place.pages);
        let shown = place.shown;
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
                            // Where the TTF label sat before: top left, unwrapped.
                            sprite_text_styled(label, 18.0, theme.text, SpriteTextStyle::default().stock_fallback()),
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
    // The button panel is a group anchored at the window's edge: solid where it is, outside the sheet's area too.
    if !a1 {
        state.solid.extend(buttons::panel_rect(&layout));
    }
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

pub(super) fn navigate(mut clicks: MessageReader<ButtonClicked>, nav: Query<&StockNav>, mut state: ResMut<StockState>, cheat: Option<Res<crate::cheats::input::CheatInput>>) {
    if cheat.is_some_and(|c| c.open || c.blocked_this_frame) {
        clicks.read().for_each(drop);
        return;
    }
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

#[cfg(test)]
mod recrew_tests {
    use std::sync::Arc;

    use sta_content::{ContentDb, UnitDef};
    use sta_sim::view::{UnitFlags, UnitView};
    use sta_sim::{EntityId, Fixed, PlayerId, Vec3, WorldView};

    use super::*;

    #[test]
    fn authored_recrew_menu_preserves_live_look_art_and_tooltips() {
        let db = Arc::new(ContentDb::builder().unit(UnitDef::new("station", "starbase")).build().unwrap());
        let unit = UnitView {
            id: EntityId(1), def: db.key("station").unwrap(), owner: PlayerId(1),
            pos: Vec3::ZERO, heading: (Fixed::ZERO, Fixed::ONE), speed: Fixed::ZERO,
            hull: [Fixed::ONE; 2], shields: [[Fixed::ZERO; 2]; 4],
            crew: Some([Fixed::from_int(50), Fixed::from_int(100)]), energy: None, autonomy: None,
            flags: UnitFlags::SELECTABLE, progress: None,
        };
        let mut feed = HudFeed {
            view: Arc::new(WorldView { units: vec![unit], ..Default::default() }),
            player: PlayerId(1), selected: vec![EntityId(1)], content: Some(db), ..Default::default()
        };
        let mut native = NativeCommands::default();
        native.groups.push("orders".into());
        native.commands.insert("recrew".into(), NativeCommand {
            odf: "recrew.odf".into(), command: "RECREW".into(), button: "recrew".into(),
            preferred: Some([0, 1]), menu: Some("Orders".into()),
            tooltip: Some("GUI_CP_RECREW_TOOLTIP".into()),
            verbose: Some("GUI_CP_RECREW_VTOOLTIP".into()),
            normal: None, hovered: None, pressed: None, disabled: None, active: None,
        });
        let root = buttons(&feed, Page::Top, &native);
        assert!(root.iter().any(|b| b.act == Act::OpenNativeMenu(0)));
        assert!(!root.iter().any(|b| b.act == Act::Recrew));
        let in_menu = buttons(&feed, Page::Native(0), &native);
        let recrew = in_menu.iter().find(|b| b.act == Act::Recrew).unwrap();
        assert_eq!(recrew.look, Look::Ready);
        assert_eq!(recrew.sprite_names, ["b_recrew", "recrew"]);
        assert_eq!(recrew.tip, "GUI_CP_RECREW_TOOLTIP");
        assert_eq!(recrew.verbose.as_deref(), Some("GUI_CP_RECREW_VTOOLTIP"));
        Arc::make_mut(&mut feed.view).units[0].crew = Some([Fixed::from_int(100); 2]);
        assert_eq!(buttons(&feed, Page::Native(0), &native).iter().find(|b| b.act == Act::Recrew).unwrap().look, Look::Locked);
    }
}

#[cfg(test)]
mod capture_tests {
    use super::*;
    use std::sync::Arc;
    use sta_sim::{EntityId, Fixed, PlayerId, Vec3, WorldView};
    use sta_sim::view::{UnitFlags, UnitView};

    #[test]
    fn authored_capture_stays_in_trade_menu_and_uses_localized_keys() {
        let mut carrier = sta_content::UnitDef::new("ui_carrier", "craft");
        carrier.transport.welding_anim_speed = Some(Fixed::ZERO);
        let db = Arc::new(sta_content::ContentDb::builder()
            .profile(sta_content::Profile::Fo4)
            .features(sta_content::Profile::Fo4.features())
            .unit(carrier).build().unwrap());
        let unit = UnitView {
            id: EntityId(1), def: db.key("ui_carrier").unwrap(), owner: PlayerId(1),
            pos: Vec3::ZERO, heading: (Fixed::ZERO, Fixed::ONE), speed: Fixed::ZERO,
            hull: [Fixed::ONE; 2], shields: [[Fixed::ZERO; 2]; 4],
            crew: Some([Fixed::from_int(40), Fixed::from_int(50)]), energy: None, autonomy: None,
            flags: UnitFlags::SELECTABLE, progress: None,
        };
        let feed = HudFeed {
            view: Arc::new(WorldView { units: vec![unit], ..Default::default() }),
            player: PlayerId(1), selected: vec![EntityId(1)], content: Some(db), ..Default::default()
        };
        let mut native = NativeCommands::default();
        native.groups.push("trade".into());
        native.commands.insert("capture".into(), NativeCommand {
            odf: "capture.odf".into(), command: "TRANSPORT_ATTACK".into(), button: "capture".into(),
            preferred: Some([0, 10]), menu: Some("trade".into()),
            tooltip: Some("GUI_CP_TRANSPORT_ATTACK_TOOLTIP".into()),
            verbose: Some("GUI_CP_TRANSPORT_ATTACK_VTOOLTIP".into()),
            normal: None, hovered: None, pressed: None, disabled: None, active: None,
        });
        let top = buttons(&feed, Page::Top, &native);
        assert!(top.iter().any(|b| b.act == Act::OpenNativeMenu(0)));
        assert!(!top.iter().any(|b| b.act == Act::Target(TargetMode::TransportAttack)));
        for page in [Page::Build, Page::Formations] {
            assert!(!buttons(&feed, page, &native).iter().any(|b| b.act == Act::Target(TargetMode::TransportAttack)));
        }
        let trade = buttons(&feed, Page::Native(0), &native);
        let capture = trade.iter().find(|b| b.act == Act::Target(TargetMode::TransportAttack)).unwrap();
        assert_eq!(capture.sprite_names, ["b_capture", "capture"]);
        assert_eq!(capture.tip, "GUI_CP_TRANSPORT_ATTACK_TOOLTIP");
        assert_eq!(capture.verbose.as_deref(), Some("GUI_CP_TRANSPORT_ATTACK_VTOOLTIP"));
        native.commands.clear();
        assert!(!buttons(&feed, Page::Top, &native).iter().any(|b| b.act == Act::Target(TargetMode::TransportAttack)));
    }
}

#[cfg(test)]
mod menu_tip_tests {
    use super::*;
    use crate::ui::i18n::Strings;
    use sta_sim::view::UnitView;
    use sta_sim::{EntityId, Fixed, PlayerId, Vec3, WorldView};

    fn feed_with(keys: &str) -> HudFeed {
        let mut def = sta_content::UnitDef::new("m_research", "station");
        def.build.list = vec![sta_content::DefRef::new("m_research")];
        def.raw = std::sync::Arc::new(sta_odf::resolve("m", &mut |_| Some(keys.to_owned())).unwrap());
        let db = sta_content::ContentDb::builder().unit(def).build().unwrap();
        let f = Fixed::from_int;
        let unit = UnitView {
            id: EntityId(1), def: db.key("m_research").unwrap(), owner: PlayerId(1), pos: Vec3::ZERO,
            heading: (Fixed::ZERO, Fixed::ONE), speed: Fixed::ZERO, hull: [f(100), f(100)], shields: [[f(1), f(1)]; 4],
            crew: None, energy: None, autonomy: None, flags: sta_sim::view::UnitFlags::SELECTABLE, progress: None,
        };
        HudFeed { view: std::sync::Arc::new(WorldView { units: vec![unit], ..Default::default() }), player: PlayerId(1), selected: vec![EntityId(1)], content: Some(std::sync::Arc::new(db)), ..Default::default() }
    }

    fn menu(feed: &HudFeed, strings: Option<&Strings>) -> Button {
        buttons_with(feed, Page::Top, &NativeCommands::default(), strings)
            .into_iter()
            .find(|b| matches!(b.act, Act::OpenBuild | Act::OpenResearch | Act::OpenEvolve))
            .unwrap()
    }

    #[test]
    fn menu_buttons_take_the_string_table_tooltips_when_it_has_the_keys() {
        let feed = feed_with("research = 1\n");
        let table = Strings::from_game("english", "\"GUI_CP_RESEARCH_TOOLTIP\", \"a\", \"GUI_CP_RESEARCH_VTOOLTIP\", \"b\"");
        let b = menu(&feed, Some(&table));
        assert_eq!((b.act.clone(), b.tip.as_str(), b.verbose.as_deref()), (Act::OpenResearch, "GUI_CP_RESEARCH_TOOLTIP", Some("GUI_CP_RESEARCH_VTOOLTIP")));
        assert_eq!(b.sprite_names, ["b_research"]);
        // A table without the keys leaves the source text; no table the same.
        let b = menu(&feed, Some(&Strings::from_game("english", "\"OTHER\", \"x\"")));
        assert_eq!((b.tip.as_str(), b.verbose.clone()), ("Research", None));
        assert_eq!(menu(&feed, None).tip, "Research");
        // Build's own keys apply to the Build button.
        let build = feed_with("builder_ship = 1\n");
        let table = Strings::from_game("english", "\"GUI_CP_BUILD_TOOLTIP\", \"a\"");
        let b = menu(&build, Some(&table));
        assert_eq!((b.act.clone(), b.tip.as_str(), b.verbose.clone()), (Act::OpenBuild, "GUI_CP_BUILD_TOOLTIP", None));
    }
}
